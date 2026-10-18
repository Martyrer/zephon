//! App state and commands between the core and the UI (the gpui-kit window, `zephon status`).
//!
//! The UI only calls [`Controller::send`] with a [`Command`] and renders
//! [`Controller::snapshot`]. A background thread with its own tokio runtime owns the
//! shared capture, one session per checked receiver, discovery, and the firewall check,
//! and calls the notifier after every state change (e.g. to request a repaint).
//!
//! In demo mode ([`crate::settings::demo_mode`]) it touches no device and no network: the
//! endpoints, rooms and sessions are made up (`demo.rs`).

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::firewall::{self, FirewallState};
use crate::level::LevelTap;
use crate::session::{self, LatencyProfile, SessionStats, Status};
use crate::settings::{AccentChoice, Backdrop, Settings, TextSize, ThemeChoice};

mod demo;
mod engine;

use engine::Engine;

/// Slider value for a receiver whose current volume could not be read.
pub const DEFAULT_VOLUME_PCT: f32 = session::DEFAULT_VOLUME_PCT;
/// How often receivers are re-discovered in the background.
pub const DISCOVERY_INTERVAL: Duration = Duration::from_secs(30);
/// How long "resume on launch" waits for the rooms that were on to be found.
pub const RESUME_WAIT: Duration = Duration::from_secs(10);
/// How long closing the app waits for the sessions to tear down (the windows are already
/// gone). A streaming session takes at most about 1.5 s: Stop gives up a control request in
/// flight, then waits up to 1.5 s for the TEARDOWN answer. A start that was given up on takes
/// at most 2 s + 1.5 s. The rest is margin for Nyx joining its threads.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(6);
/// Settings are written once they have not changed for this long, so dragging a slider (many
/// changes a second) writes the file once, not on every step.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// How many times a room that lost its connection while playing is started again before the
/// app gives up on it.
pub const RECONNECT_ATTEMPTS: u32 = 3;
/// The wait before the first reconnect attempt. It doubles for each later attempt
/// (exponential backoff: 2, 4, 8 s), so a receiver that is busy or rejoining Wi-Fi gets time.
const RECONNECT_FIRST_WAIT: Duration = Duration::from_secs(2);

/// How long a reconnected room must play before a new loss counts as a fresh one (attempt 1)
/// rather than the next attempt.
const RECONNECT_STABLE: Duration = Duration::from_secs(60);

/// The wait before reconnect attempt `attempt` (1-based).
fn reconnect_wait(attempt: u32) -> Duration {
    RECONNECT_FIRST_WAIT * 2u32.saturating_pow(attempt.saturating_sub(1))
}

/// Where a room is in reconnecting after it lost its connection while playing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Reconnect {
    #[default]
    No,
    /// Waiting for, or connecting in, this attempt (1 to [`RECONNECT_ATTEMPTS`]).
    Attempt(u32),
    /// Every attempt failed; the status holds the error.
    GaveUp,
}

pub type Notifier = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub struct EndpointView {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Debug, Clone)]
pub struct ReceiverView {
    pub name: String,
    pub addr: SocketAddr,
    pub model: Option<String>,
    /// Seen in the latest mDNS browse.
    pub online: bool,
    /// Checkbox: included in streaming.
    pub selected: bool,
    pub volume_pct: f32,
    /// `volume_pct` came from the receiver (or the user), not the 30 % default.
    pub volume_known: bool,
    pub status: Status,
    pub stats: Option<SessionStats>,
    /// This receiver's latency profile (saved by name; new receivers get the default).
    pub profile: LatencyProfile,
    /// The session is being restarted to apply a profile change.
    pub restarting: bool,
    /// Reconnecting after the connection was lost while playing.
    pub reconnect: Reconnect,
    /// A more robust profile suggested from recent loss ([`crate::advice::suggest_profile`]).
    pub suggestion: Option<LatencyProfile>,
    /// Hidden in the settings: left out of the Rooms list, never selected.
    pub hidden: bool,
    /// The peak levels of the audio sent to the room's latest session, for its meter.
    pub level: Option<Arc<LevelTap>>,
}

/// A receiver for the settings window's "Hidden rooms": every one known, found now or only
/// remembered in the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct KnownRoom {
    pub name: String,
    pub model: Option<String>,
    pub online: bool,
    pub hidden: bool,
}

/// The receivers in the list (found or cached) plus those only in the settings, by name.
pub fn known_rooms(receivers: &[ReceiverView], settings: &Settings) -> Vec<KnownRoom> {
    let mut rooms: Vec<KnownRoom> = receivers
        .iter()
        .map(|r| KnownRoom {
            name: r.name.clone(),
            model: r.model.clone(),
            online: r.online,
            hidden: settings.is_hidden(&r.name),
        })
        .collect();
    for (name, prefs) in &settings.receivers {
        if !rooms.iter().any(|r| &r.name == name) {
            rooms.push(KnownRoom {
                name: name.clone(),
                model: None,
                online: false,
                hidden: prefs.hidden,
            });
        }
    }
    rooms.sort_by_key(|r| r.name.to_lowercase());
    rooms
}

/// What "resume on launch" does next.
#[derive(Debug, Clone, PartialEq)]
pub enum ResumeStep {
    Wait,
    /// Press Play: rooms that are on but not found get the normal error.
    Play,
    /// Nothing to play, or none of the rooms was found in time (with a message for the user).
    GiveUp(Option<String>),
}

/// `rooms_on`: each room that is on (and not hidden), with whether mDNS has found it.
pub fn resume_step(rooms_on: &[(&str, bool)], endpoint_ready: bool, timed_out: bool) -> ResumeStep {
    if rooms_on.is_empty() {
        return ResumeStep::GiveUp(None);
    }
    let found = rooms_on.iter().filter(|(_, online)| *online).count();
    if found == rooms_on.len() && endpoint_ready {
        return ResumeStep::Play;
    }
    if !timed_out {
        return ResumeStep::Wait;
    }
    if found == 0 {
        return ResumeStep::GiveUp(Some(format!(
            "Didn't resume playing: no room that was on was found within {} s.",
            RESUME_WAIT.as_secs()
        )));
    }
    // Play shows why when the capture endpoint is missing.
    ResumeStep::Play
}

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub endpoints: Vec<EndpointView>,
    /// The capture endpoint that Start will use.
    pub endpoint: Option<EndpointView>,
    pub endpoint_error: Option<String>,
    pub receivers: Vec<ReceiverView>,
    pub discovering: bool,
    /// Profile for receivers without their own choice (new rooms).
    pub default_profile: LatencyProfile,
    /// The saved settings, as last written (for the settings window).
    pub settings: Settings,
    pub streaming: bool,
    pub firewall: FirewallState,
    /// A receiver failed in a way that points at the firewall (the 10 s timeout); the GUI
    /// then shows the full firewall banner. Cleared when the rule is in place.
    pub firewall_suspected: bool,
    pub exe_path: String,
    /// Last general message for the user (e.g. why Start did nothing).
    pub message: Option<String>,
}

#[derive(Debug)]
pub enum Command {
    RefreshEndpoints,
    /// Select the capture endpoint by ID (remembered in settings).
    SelectEndpoint(String),
    RefreshReceivers,
    /// Check or uncheck a receiver. While streaming this starts or stops its session.
    SetSelected(String, bool),
    /// Receiver volume 0-100 %, applied live while streaming.
    SetVolume(String, f32),
    /// Set one receiver's latency profile (saved). A running session of that receiver is
    /// restarted to apply it; the others keep streaming.
    SetProfile(String, LatencyProfile),
    Start,
    Stop,
    CheckFirewall,
    /// One UAC prompt, then re-check.
    InstallFirewall,
    /// Hide or show a receiver (saved). Hiding switches it off, stopping it if it plays.
    SetHidden(String, bool),
    /// Latency profile for rooms switched on for the first time (saved).
    SetDefaultProfile(LatencyProfile),
    /// Start volume for rooms switched on for the first time (saved).
    SetDefaultVolume(f32),
    /// Play again on launch if it was playing when it closed (saved).
    SetResumeOnLaunch(bool),
    /// Receiver-reported volume changes move the slider (saved).
    SetVolumeFollowsReceiver(bool),
    /// The GUI has started: play again if `resume_on_launch` and it was playing, once the rooms
    /// that were on are found (up to [`RESUME_WAIT`]).
    Resume,
    /// GUI theme (saved).
    SetTheme(ThemeChoice),
    /// GUI window material (saved).
    SetBackdrop(Backdrop),
    /// GUI font family, `None` = default (saved).
    SetFont(Option<String>),
    /// GUI text size (saved).
    SetTextSize(TextSize),
    /// GUI accent color (saved).
    SetAccent(AccentChoice),
}

enum Msg {
    Cmd(Command),
    /// Stop everything, wait for the sessions until the deadline, then report done.
    Shutdown(std::sync::mpsc::Sender<()>, Instant),
}

pub struct Controller {
    tx: mpsc::UnboundedSender<Msg>,
    state: Arc<Mutex<AppState>>,
}

impl Controller {
    pub fn spawn(notify: Notifier) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let state = Arc::new(Mutex::new(AppState {
            exe_path: firewall::exe_path().display().to_string(),
            ..Default::default()
        }));
        let st = state.clone();
        std::thread::Builder::new()
            .name("controller".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(Engine::new(st, notify).run(rx));
            })
            .expect("spawn controller thread");
        Self { tx, state }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Cmd(cmd));
    }

    pub fn snapshot(&self) -> AppState {
        lock(&self.state).clone()
    }

    /// Stops all sessions (TEARDOWN) and waits up to `timeout` for them to end.
    pub fn shutdown(&self, timeout: Duration) {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        // The engine waits for the sessions until the same deadline.
        if self
            .tx
            .send(Msg::Shutdown(done_tx, Instant::now() + timeout))
            .is_ok()
        {
            let _ = done_rx.recv_timeout(timeout);
        }
    }
}

fn lock(state: &Mutex<AppState>) -> MutexGuard<'_, AppState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Shared by the tests here and in `engine`.
#[cfg(test)]
mod test_support {
    use super::*;

    /// A found room with the given switches, volume 50 and the Balanced profile.
    pub(super) fn room(name: &str, online: bool, selected: bool) -> ReceiverView {
        ReceiverView {
            name: name.into(),
            addr: "192.168.1.20:7000".parse().unwrap(),
            model: Some("AudioAccessory5,1".into()),
            online,
            selected,
            volume_pct: 50.0,
            volume_known: true,
            status: Status::Idle,
            stats: None,
            profile: LatencyProfile::Balanced,
            restarting: false,
            reconnect: Reconnect::No,
            suggestion: None,
            hidden: false,
            level: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::room;
    use super::*;

    #[test]
    fn known_rooms_merge_the_list_and_the_settings() {
        let mut s = Settings::default();
        s.set_selected("Kitchen", true);
        s.set_hidden("Bedroom TV", true);
        s.set_selected("Old Speaker", false); // remembered, never found this run
        let mut list = [room("kitchen", true, true), room("Bedroom TV", true, false)];
        list[1].model = Some("AppleTV11,1".into());
        let known = |name: &str, model: Option<&str>, online, hidden| KnownRoom {
            name: name.into(),
            model: model.map(Into::into),
            online,
            hidden,
        };
        // "kitchen" in the list and "Kitchen" in the settings are different receivers.
        assert_eq!(
            known_rooms(&list, &s),
            [
                known("Bedroom TV", Some("AppleTV11,1"), true, true),
                known("kitchen", Some("AudioAccessory5,1"), true, false),
                known("Kitchen", None, false, false),
                known("Old Speaker", None, false, false),
            ]
        );
    }

    #[test]
    fn resume_waits_for_every_room_then_plays_what_was_found() {
        use ResumeStep::*;
        let both = [("Kitchen", true), ("Living Room", true)];
        let one = [("Kitchen", true), ("Living Room", false)];
        let none = [("Kitchen", false), ("Living Room", false)];
        assert_eq!(resume_step(&[], true, false), GiveUp(None));
        assert_eq!(resume_step(&both, true, false), Play);
        // The capture endpoint is not known yet.
        assert_eq!(resume_step(&both, false, false), Wait);
        assert_eq!(resume_step(&one, true, false), Wait);
        // At the deadline: play whichever were found; the rest show the normal error.
        assert_eq!(resume_step(&one, true, true), Play);
        assert_eq!(resume_step(&one, false, true), Play);
        assert!(matches!(resume_step(&none, true, true), GiveUp(Some(_))));
        assert_eq!(resume_step(&none, true, false), Wait);
    }
}
