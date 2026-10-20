//! The controller's background side: owns the capture, the sessions, discovery, the firewall
//! jobs and the settings writes, and applies each [`Command`] to the shared [`AppState`].

use std::collections::{HashMap, HashSet};

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{info, warn};

use crate::advice;
use crate::capture::{self, CaptureHub};
use crate::discovery::{self, Receiver};
use crate::firewall::{self, FirewallState};
use crate::level::LevelTap;
use crate::session::{self, SessionEvent, SessionHandle, SessionSpec, Status};
use crate::settings::{self, Settings};

use super::{
    AppState, Command, DEFAULT_VOLUME_PCT, DISCOVERY_INTERVAL, EndpointView, Msg, Notifier,
    RECONNECT_ATTEMPTS, RECONNECT_STABLE, RESUME_WAIT, ReceiverView, Reconnect, ResumeStep,
    SAVE_DELAY, SHUTDOWN_TIMEOUT, demo, lock, reconnect_wait, resume_step,
};

enum Internal {
    Endpoints(Result<Vec<EndpointView>, String>),
    Discovered(Result<Vec<Receiver>, String>),
    Volume(String, Option<f32>),
    /// A firewall check or install has finished (tagged with its [`FirewallJobs`] generation).
    Firewall(u64, FirewallState),
    /// The capture endpoint has been opened for Start (tagged with the attempt).
    Capture(u64, Result<CaptureHub, String>),
}

/// A room that lost its connection while playing and is being started again.
struct PendingReconnect {
    /// 1 to [`RECONNECT_ATTEMPTS`].
    attempt: u32,
    /// When the attempt starts; `None` while the failed session is still ending, or while the
    /// attempt is connecting or playing.
    at: Option<Instant>,
    /// When the attempt started playing. Once it has played for [`RECONNECT_STABLE`], a new
    /// loss starts the count afresh.
    playing_since: Option<Instant>,
    /// Why the connection was first lost, for the message when the app gives up.
    cause: String,
}

/// Where the sessions get their audio.
enum Audio {
    Hub(CaptureHub),
    /// Demo mode: the sessions only pretend to play.
    Demo,
}

/// Writes the settings file: [`Settings::save`] in the app; unit tests record the writes
/// instead, so they never touch the user's file.
type SettingsWriter = Arc<dyn Fn(&Settings) + Send + Sync>;

/// Keeps firewall checks from piling up: every failed start that looks like the firewall asks
/// for a check, and each runs PowerShell for a few seconds. A check that finished after an
/// install could also report the state from before it.
#[derive(Debug, Default)]
struct FirewallJobs {
    /// Tags each job; only the latest job's result is used.
    generation: u64,
    /// The latest job is still running (`true` = an install).
    running: Option<bool>,
}

impl FirewallJobs {
    /// The generation to tag a new job with, or `None` when it is not needed: a check while
    /// any job runs (an install checks again at its end), or a second install.
    fn begin(&mut self, install: bool) -> Option<u64> {
        match self.running {
            Some(true) => return None,
            Some(false) if !install => return None,
            _ => {}
        }
        self.generation += 1;
        self.running = Some(install);
        Some(self.generation)
    }

    /// Whether a finished job is the latest one; results of older ones are dropped.
    fn finish(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        self.running = None;
        true
    }
}

pub(super) struct Engine {
    state: Arc<Mutex<AppState>>,
    notify: Notifier,
    settings: Settings,
    demo: bool,
    hub: Option<Audio>,
    /// The Start whose capture endpoint is being opened (see [`Internal::Capture`]).
    opening: Option<u64>,
    next_open: u64,
    /// Each receiver's session, until its task has ended (also while it is stopping).
    sessions: HashMap<String, SessionHandle>,
    /// Session id -> receiver name, until the session task has ended.
    owners: HashMap<u64, String>,
    next_id: u64,
    probing: HashSet<String>,
    /// Receivers to start again once their current session has ended: one session per
    /// receiver at a time (a profile change, Stop then Play, or off then on).
    restart: HashSet<String>,
    /// Rooms that lost their connection while playing, by name.
    reconnects: HashMap<String, PendingReconnect>,
    firewall_jobs: FirewallJobs,
    /// Waiting to resume playing (on launch) until the rooms are found, up to this time.
    resume_until: Option<Instant>,
    /// Unsaved settings changes are written at this time ([`SAVE_DELAY`] after the last one).
    save_at: Option<Instant>,
    /// The latest settings write, running off the runtime's workers.
    writing: Option<tokio::task::JoinHandle<()>>,
    write: SettingsWriter,
    ev_tx: session::EventTx,
    ev_rx: mpsc::UnboundedReceiver<(u64, SessionEvent)>,
    int_tx: mpsc::UnboundedSender<Internal>,
    int_rx: mpsc::UnboundedReceiver<Internal>,
}

impl Engine {
    pub(super) fn new(state: Arc<Mutex<AppState>>, notify: Notifier) -> Self {
        Self::with_settings(
            state,
            notify,
            Settings::load(),
            settings::demo_mode(),
            Arc::new(Settings::save),
        )
    }

    fn with_settings(
        state: Arc<Mutex<AppState>>,
        notify: Notifier,
        settings: Settings,
        demo: bool,
        write: SettingsWriter,
    ) -> Self {
        {
            let mut st = lock(&state);
            st.default_profile = settings.default_profile;
            st.settings = settings.clone();
        }
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let (int_tx, int_rx) = mpsc::unbounded_channel();
        Self {
            state,
            notify,
            settings,
            demo,
            hub: None,
            opening: None,
            next_open: 1,
            sessions: HashMap::new(),
            owners: HashMap::new(),
            next_id: 1,
            probing: HashSet::new(),
            restart: HashSet::new(),
            reconnects: HashMap::new(),
            firewall_jobs: FirewallJobs::default(),
            resume_until: None,
            save_at: None,
            writing: None,
            write,
            ev_tx,
            ev_rx,
            int_tx,
            int_rx,
        }
    }

    pub(super) async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Msg>) {
        if self.demo {
            self.fill_demo();
        } else {
            self.handle(Command::RefreshEndpoints);
            self.handle(Command::CheckFirewall);
            // Show known receivers at once (offline until mDNS confirms them): one browse can
            // miss a Wi-Fi receiver's reply.
            self.merge_receivers(discovery::cached(), false);
        }
        let mut discover_tick = tokio::time::interval(DISCOVERY_INTERVAL);
        // A second browse soon after the first, for receivers whose replies were lost.
        let early_rebrowse = tokio::time::sleep(Duration::from_secs(5));
        tokio::pin!(early_rebrowse);
        let mut early_done = false;
        loop {
            let reconnect_at = self.next_reconnect();
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(Msg::Cmd(c)) => self.handle(c),
                    Some(Msg::Shutdown(done, deadline)) => {
                        self.shutdown(deadline).await;
                        let _ = done.send(());
                        break;
                    }
                    None => {
                        self.shutdown(Instant::now() + SHUTDOWN_TIMEOUT).await;
                        break;
                    }
                },
                Some((id, ev)) = self.ev_rx.recv() => self.on_session_event(id, ev),
                Some(i) = self.int_rx.recv() => self.on_internal(i),
                _ = discover_tick.tick() => self.handle(Command::RefreshReceivers),
                _ = &mut early_rebrowse, if !early_done => {
                    early_done = true;
                    self.handle(Command::RefreshReceivers);
                }
                _ = tokio::time::sleep_until(self.resume_until.unwrap_or_else(Instant::now)),
                    if self.resume_until.is_some() => self.try_resume(true),
                _ = tokio::time::sleep_until(self.save_at.unwrap_or_else(Instant::now)),
                    if self.save_at.is_some() => self.write_settings(),
                _ = tokio::time::sleep_until(reconnect_at.unwrap_or_else(Instant::now)),
                    if reconnect_at.is_some() => self.start_due_reconnects(),
            }
            (self.notify)();
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            // Demo mode: the made-up endpoints, rooms and firewall state stay as they are.
            Command::RefreshEndpoints
            | Command::RefreshReceivers
            | Command::CheckFirewall
            | Command::InstallFirewall
                if self.demo => {}
            Command::RefreshEndpoints => {
                let tx = self.int_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let list = capture::list_endpoints()
                        .map(|l| {
                            l.into_iter()
                                .map(|d| EndpointView {
                                    id: d.id,
                                    name: d.friendly_name,
                                    is_default: d.is_default,
                                })
                                .collect()
                        })
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Internal::Endpoints(list));
                });
            }
            Command::SelectEndpoint(id) => {
                let mut st = lock(&self.state);
                if st.streaming {
                    st.message = Some("Stop to change where the sound comes from.".into());
                    return;
                }
                if let Some(ep) = st.endpoints.iter().find(|e| e.id == id).cloned() {
                    self.settings.capture_device_id = Some(ep.id.clone());
                    self.settings.capture_device_name = Some(ep.name.clone());
                    st.endpoint = Some(ep);
                    st.endpoint_error = None;
                    drop(st);
                    self.save();
                }
            }
            Command::RefreshReceivers => {
                let mut st = lock(&self.state);
                if st.discovering {
                    return;
                }
                st.discovering = true;
                drop(st);
                let tx = self.int_tx.clone();
                let iface = self.settings.interface.clone();
                tokio::spawn(async move {
                    let r = discovery::discover(iface).await.map_err(|e| e.to_string());
                    let _ = tx.send(Internal::Discovered(r));
                });
            }
            Command::SetSelected(name, on) => {
                if on && self.settings.is_hidden(&name) {
                    return;
                }
                // The first switch-on pins the new-room defaults (profile and start volume).
                let first_volume = if on {
                    self.settings.first_switch_on(&name)
                } else {
                    None
                };
                let profile = self.settings.profile_for(&name);
                let streaming = {
                    let mut st = lock(&self.state);
                    if let Some(r) = st.receivers.iter_mut().find(|r| r.name == name) {
                        r.selected = on;
                        if on {
                            r.profile = profile;
                        }
                        if let Some(v) = first_volume {
                            r.volume_pct = v;
                            r.volume_known = true;
                        }
                    }
                    st.streaming
                };
                self.settings.set_selected(&name, on);
                self.save();
                if streaming {
                    if on {
                        self.start_session(&name);
                    } else {
                        self.cancel_restart(&name);
                        self.cancel_reconnect(&name);
                        if let Some(h) = self.sessions.get(&name) {
                            h.stop();
                        }
                        // With no room left on there is nothing to play: stop, as the Stop
                        // button would, so the footer offers Play (disabled until a room is on).
                        // A hidden room can still be on from an older settings file, but never
                        // plays, so it does not count.
                        let any_on = lock(&self.state)
                            .receivers
                            .iter()
                            .any(|r| r.selected && !r.hidden);
                        if !any_on {
                            info!("No room on; stopping");
                            self.handle(Command::Stop);
                        }
                    }
                }
            }
            Command::SetVolume(name, pct) => {
                let Some(pct) = session::finite_volume(pct) else {
                    warn!("[{name}] volume {pct} ignored (not a number)");
                    return;
                };
                if let Some(r) = lock(&self.state)
                    .receivers
                    .iter_mut()
                    .find(|r| r.name == name)
                {
                    r.volume_pct = pct;
                    r.volume_known = true;
                }
                if let Some(h) = self.sessions.get(&name) {
                    h.set_volume(pct);
                }
                if self
                    .settings
                    .volume_of(&name)
                    .is_none_or(|v| (v - pct).abs() >= 0.5)
                {
                    self.settings.set_volume(&name, pct);
                    self.save();
                }
            }
            Command::SetProfile(name, p) => {
                self.settings.set_profile(&name, p);
                self.save();
                let restart = {
                    let mut st = lock(&self.state);
                    let streaming = st.streaming;
                    let Some(r) = st.receivers.iter_mut().find(|r| r.name == name) else {
                        return;
                    };
                    if r.profile == p {
                        return;
                    }
                    r.profile = p;
                    r.suggestion = None;
                    // While the capture is (re)opening after Play there is nothing to restart:
                    // the sessions start with the new profile once it is open.
                    let restart = streaming
                        && r.selected
                        && self.hub.is_some()
                        && self.sessions.contains_key(&name);
                    if restart {
                        r.restarting = true;
                        r.stats = None;
                    }
                    restart
                };
                // A running session (connecting or streaming) restarts with the new profile
                // once its TEARDOWN is done; the other receivers keep streaming.
                if restart {
                    info!("[{name}] profile -> {}; restarting its session", p.name());
                    self.start_session(&name);
                }
            }
            Command::Start => {
                self.resume_until = None;
                self.start();
            }
            Command::Stop => {
                self.resume_until = None;
                self.stop();
                if self.settings.was_playing {
                    self.settings.was_playing = false;
                    self.save();
                }
            }
            Command::CheckFirewall | Command::InstallFirewall => {
                let install = matches!(cmd, Command::InstallFirewall);
                let Some(generation) = self.firewall_jobs.begin(install) else {
                    return;
                };
                lock(&self.state).firewall = if install {
                    FirewallState::Installing
                } else {
                    FirewallState::Checking
                };
                let tx = self.int_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let st = if install {
                        firewall::install()
                    } else {
                        firewall::check_state()
                    };
                    let _ = tx.send(Internal::Firewall(generation, st));
                });
            }
            Command::SetHidden(name, hidden) => {
                if hidden && self.settings.is_selected(&name) {
                    // Switch it off first (stops just that room if it plays).
                    self.handle(Command::SetSelected(name.clone(), false));
                }
                self.settings.set_hidden(&name, hidden);
                self.save();
                if let Some(r) = lock(&self.state)
                    .receivers
                    .iter_mut()
                    .find(|r| r.name == name)
                {
                    r.hidden = hidden;
                }
            }
            Command::SetDefaultProfile(p) => {
                self.settings.set_default_profile(p);
                self.save();
                // Rooms that follow the default show it; those that are on were pinned above.
                let mut st = lock(&self.state);
                for r in st.receivers.iter_mut() {
                    r.profile = self.settings.profile_for(&r.name);
                }
            }
            Command::SetDefaultVolume(pct) => {
                let Some(pct) = session::finite_volume(pct) else {
                    warn!("New-room volume {pct} ignored (not a number)");
                    return;
                };
                self.settings.default_volume = pct.round();
                self.save();
            }
            Command::SetResumeOnLaunch(on) => {
                self.settings.resume_on_launch = on;
                self.save();
            }
            Command::SetVolumeFollowsReceiver(on) => {
                self.settings.volume_follows_receiver = on;
                self.save();
            }
            Command::Resume => {
                if self.settings.resume_on_launch && self.settings.was_playing {
                    info!(
                        "Resume on launch: waiting up to {} s for the rooms",
                        RESUME_WAIT.as_secs()
                    );
                    self.resume_until = Some(Instant::now() + RESUME_WAIT);
                    self.try_resume(false);
                }
            }
            Command::SetTheme(theme) => {
                self.settings.theme = theme;
                self.save();
            }
            Command::SetBackdrop(backdrop) => {
                self.settings.backdrop = backdrop;
                self.save();
            }
            Command::SetFont(family) => {
                self.settings.font_family = family;
                self.save();
            }
            Command::SetTextSize(size) => {
                self.settings.text_size = size;
                self.save();
            }
            Command::SetAccent(accent) => {
                self.settings.accent = accent;
                self.save();
            }
        }
    }

    /// Publishes the settings to the UI now and writes them [`SAVE_DELAY`] after the last
    /// change ([`Engine::write_settings`]).
    fn save(&mut self) {
        self.save_at = Some(Instant::now() + SAVE_DELAY);
        let mut st = lock(&self.state);
        st.default_profile = self.settings.default_profile;
        st.settings = self.settings.clone();
    }

    /// Writes the settings off the runtime's workers (the write waits for the disk). One write
    /// at a time: while the previous one runs this waits another [`SAVE_DELAY`], so an older
    /// copy can never land after a newer one.
    fn write_settings(&mut self) {
        if self.writing.as_ref().is_some_and(|w| !w.is_finished()) {
            self.save_at = Some(Instant::now() + SAVE_DELAY);
            return;
        }
        self.save_at = None;
        let settings = self.settings.clone();
        let write = self.write.clone();
        self.writing = Some(tokio::task::spawn_blocking(move || write(&settings)));
    }

    /// Writes unsaved settings now, after any write still running (at shutdown).
    async fn flush_settings(&mut self) {
        if let Some(w) = self.writing.take() {
            let _ = w.await;
        }
        if self.save_at.take().is_some() {
            (self.write)(&self.settings);
        }
    }

    /// Resume on launch: plays once every room that was on has been found and the capture
    /// endpoint is known; at the deadline, plays whichever were found.
    fn try_resume(&mut self, timed_out: bool) {
        if self.resume_until.is_none() {
            return;
        }
        let step = {
            let st = lock(&self.state);
            if st.streaming {
                ResumeStep::GiveUp(None)
            } else {
                let rooms_on: Vec<(&str, bool)> = self
                    .settings
                    .receivers
                    .iter()
                    .filter(|(_, r)| r.selected && !r.hidden)
                    .map(|(name, _)| {
                        let found = st.receivers.iter().any(|r| &r.name == name && r.online);
                        (name.as_str(), found)
                    })
                    .collect();
                resume_step(&rooms_on, st.endpoint.is_some(), timed_out)
            }
        };
        match step {
            ResumeStep::Wait => {}
            ResumeStep::Play => {
                self.resume_until = None;
                info!("Resume on launch: playing");
                self.start();
            }
            ResumeStep::GiveUp(message) => {
                self.resume_until = None;
                if let Some(m) = message {
                    warn!("{m}");
                    lock(&self.state).message = Some(m);
                }
            }
        }
    }

    fn start(&mut self) {
        let (endpoint, selected) = {
            let mut st = lock(&self.state);
            if st.streaming {
                return;
            }
            st.message = None;
            let selected: Vec<String> = st
                .receivers
                .iter()
                .filter(|r| r.selected && !r.hidden)
                .map(|r| r.name.clone())
                .collect();
            (st.endpoint.clone(), selected)
        };
        let Some(endpoint) = endpoint else {
            lock(&self.state).message = Some("Choose where the sound comes from first.".into());
            return;
        };
        if selected.is_empty() {
            lock(&self.state).message = Some("Turn on at least one room.".into());
            return;
        }
        lock(&self.state).streaming = true;
        if self.demo {
            info!("Start (demo) -> {selected:?}");
            self.hub = Some(Audio::Demo);
            self.start_sessions();
            return;
        }
        // Opening the endpoint (COM, WASAPI activation) blocks, so it runs off the runtime's
        // workers; the sessions start when it is open (`on_internal`). Stop meanwhile drops
        // the attempt.
        let attempt = self.next_open;
        self.next_open += 1;
        self.opening = Some(attempt);
        let tx = self.int_tx.clone();
        tokio::task::spawn_blocking(move || {
            let hub = CaptureHub::open(&endpoint.id, &endpoint.name).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Internal::Capture(attempt, hub));
        });
    }

    /// The capture endpoint for Start is open (or failed to open).
    fn on_capture(&mut self, attempt: u64, hub: Result<CaptureHub, String>) {
        if self.opening != Some(attempt) {
            // Stopped while it was opening.
            if let Ok(hub) = hub {
                close_hub(hub);
            }
            return;
        }
        self.opening = None;
        match hub {
            Ok(hub) => {
                info!("Start: capture \"{}\"", hub.endpoint_name());
                self.hub = Some(Audio::Hub(hub));
                self.start_sessions();
            }
            Err(e) => {
                warn!("Capture failed: {e}");
                // No session starts now, so no room may stay "Switching to…" or wait to restart.
                for name in self.restart.iter().cloned().collect::<Vec<_>>() {
                    self.cancel_restart(&name);
                }
                let mut st = lock(&self.state);
                st.streaming = false;
                st.message = Some(format!("Capture failed: {e}"));
                for r in st.receivers.iter_mut() {
                    r.restarting = false;
                }
            }
        }
    }

    /// Starts a session for every room that is on (now, not when Start was pressed).
    fn start_sessions(&mut self) {
        let selected: Vec<String> = lock(&self.state)
            .receivers
            .iter()
            .filter(|r| r.selected && !r.hidden)
            .map(|r| r.name.clone())
            .collect();
        info!("Playing to {selected:?}");
        // Kept when the app closes while playing, for resume on launch.
        if !self.settings.was_playing {
            self.settings.was_playing = true;
            self.save();
        }
        for name in selected {
            self.start_session(&name);
        }
    }

    fn start_session(&mut self, name: &str) {
        if self.hub.is_none() {
            return;
        }
        // One session per receiver at a time: a session that has not ended yet (it may be
        // tearing down after Stop or a switch-off) is stopped, and the new one starts when it
        // has ended (`on_session_event`). Its last events are ignored meanwhile.
        if let Some(old) = self.sessions.get(name) {
            old.stop();
            self.restart.insert(name.to_string());
            if let Some(r) = lock(&self.state)
                .receivers
                .iter_mut()
                .find(|r| r.name == name)
            {
                r.status = Status::Connecting;
                r.stats = None;
                r.suggestion = None;
            }
            return;
        }
        let spec = {
            let st = lock(&self.state);
            let Some(r) = st.receivers.iter().find(|r| r.name == name) else {
                return;
            };
            SessionSpec {
                name: r.name.clone(),
                addr: r.addr,
                volume_pct: r.volume_pct,
                profile: r.profile,
                interface: self.settings.interface.clone(),
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        // A tap per session, so the meter never shows what an earlier session sent.
        let level = Arc::new(LevelTap::new());
        let handle = match &self.hub {
            Some(Audio::Hub(hub)) => {
                let (source, queue) = hub.subscribe();
                let source = source.metered(level.clone());
                session::spawn(id, spec, source, queue, self.ev_tx.clone())
            }
            Some(Audio::Demo) => {
                demo::spawn_session(id, spec.name, level.clone(), self.ev_tx.clone())
            }
            None => return,
        };
        self.sessions.insert(name.to_string(), handle);
        self.owners.insert(id, name.to_string());
        if let Some(r) = lock(&self.state)
            .receivers
            .iter_mut()
            .find(|r| r.name == name)
        {
            r.status = Status::Connecting;
            r.stats = None;
            r.suggestion = None;
            r.restarting = false;
            r.reconnect = Reconnect::No;
            r.level = Some(level);
        }
        self.restart.remove(name);
        // A start for any other reason (Play, switch on, profile change) is a fresh one.
        self.reconnects.remove(name);
    }

    /// Forgets a pending reconnect (the room is stopping for good).
    fn cancel_reconnect(&mut self, name: &str) {
        if self.reconnects.remove(name).is_some()
            && let Some(r) = lock(&self.state)
                .receivers
                .iter_mut()
                .find(|r| r.name == name)
        {
            r.reconnect = Reconnect::No;
            // It showed "Reconnecting" while it waited for the next attempt.
            r.status = Status::Idle;
        }
    }

    /// When the next reconnect attempt is due.
    fn next_reconnect(&self) -> Option<Instant> {
        self.reconnects.values().filter_map(|p| p.at).min()
    }

    /// Starts every reconnect attempt that is due.
    fn start_due_reconnects(&mut self) {
        let now = Instant::now();
        let due: Vec<String> = self
            .reconnects
            .iter()
            .filter(|(_, p)| p.at.is_some_and(|at| at <= now))
            .map(|(name, _)| name.clone())
            .collect();
        for name in due {
            let wanted = {
                let st = lock(&self.state);
                st.streaming && st.receivers.iter().any(|r| r.name == name && r.selected)
            };
            // Stop and switching the room off cancel reconnects already; this is a backstop.
            if !wanted || self.hub.is_none() {
                self.cancel_reconnect(&name);
                continue;
            }
            // start_session forgets the reconnect, as any fresh start does; it is put back so
            // the attempt's outcome counts.
            let Some(mut pending) = self.reconnects.remove(&name) else {
                continue;
            };
            info!(
                "[{name}] reconnect attempt {} of {RECONNECT_ATTEMPTS}",
                pending.attempt
            );
            self.start_session(&name);
            pending.at = None;
            if let Some(r) = lock(&self.state)
                .receivers
                .iter_mut()
                .find(|r| r.name == name)
            {
                r.reconnect = Reconnect::Attempt(pending.attempt);
            }
            self.reconnects.insert(name, pending);
        }
    }

    /// A session failed. A room that was playing (or reconnecting) is started again, up to
    /// [`RECONNECT_ATTEMPTS`] times; a failed first start shows its error as it is.
    fn on_session_error(&mut self, name: &str, msg: String) {
        let mut st = lock(&self.state);
        let streaming = st.streaming;
        let Some(r) = st.receivers.iter_mut().find(|r| r.name == name) else {
            return;
        };
        // A reconnect that has played for a while counts as a success: a new loss starts afresh.
        // One that drops again soon counts as a failed attempt, so a link that keeps dropping
        // ends in "gave up" instead of reconnecting forever.
        let now = Instant::now();
        let prev = self.reconnects.get(name).filter(|p| {
            p.playing_since
                .is_none_or(|t| now.duration_since(t) < RECONNECT_STABLE)
        });
        let lost = r.status == Status::Streaming || prev.is_some();
        if !(lost && streaming && r.selected) {
            r.status = Status::Error(msg);
            return;
        }
        let attempt = prev.map_or(1, |p| p.attempt + 1);
        let cause = prev.map_or_else(|| msg.clone(), |p| p.cause.clone());
        r.stats = None;
        r.suggestion = None;
        if attempt > RECONNECT_ATTEMPTS {
            warn!("[{name}] {msg}; giving up after {RECONNECT_ATTEMPTS} reconnect attempts");
            self.reconnects.remove(name);
            r.reconnect = Reconnect::GaveUp;
            r.status = Status::Error(format!(
                "{cause}. Reconnecting failed {RECONNECT_ATTEMPTS} times. Check that the \
                 speaker is on and has a good Wi-Fi signal, then try again."
            ));
            return;
        }
        warn!("[{name}] {msg}; will reconnect (attempt {attempt} of {RECONNECT_ATTEMPTS})");
        // The attempt is timed once this session has ended (on_session_event).
        self.reconnects.insert(
            name.to_string(),
            PendingReconnect {
                attempt,
                at: None,
                playing_since: None,
                cause,
            },
        );
        r.reconnect = Reconnect::Attempt(attempt);
        r.status = Status::Connecting;
    }

    /// Stops once every room that is on has failed and none is playing, connecting or waiting
    /// to reconnect: nothing is left to play, so the footer offers Play again.
    fn stop_if_all_failed(&mut self) {
        if self.hub.is_none()
            || self.opening.is_some()
            || !self.sessions.is_empty()
            || !self.restart.is_empty()
            || !self.reconnects.is_empty()
        {
            return;
        }
        let failed = {
            let st = lock(&self.state);
            st.streaming
                && st
                    .receivers
                    .iter()
                    .any(|r| r.selected && matches!(r.status, Status::Error(_)))
        };
        if failed {
            info!("No room left playing; stopping");
            self.stop();
        }
    }

    /// Forgets a pending restart (the session is stopping for good).
    fn cancel_restart(&mut self, name: &str) {
        if self.restart.remove(name)
            && let Some(r) = lock(&self.state)
                .receivers
                .iter_mut()
                .find(|r| r.name == name)
        {
            r.restarting = false;
            // It showed "Connecting" while the old session's last statuses were ignored.
            r.status = Status::Idle;
        }
    }

    fn stop(&mut self) {
        for name in self.restart.iter().cloned().collect::<Vec<_>>() {
            self.cancel_restart(&name);
        }
        for name in self.reconnects.keys().cloned().collect::<Vec<_>>() {
            self.cancel_reconnect(&name);
        }
        for h in self.sessions.values() {
            h.stop();
        }
        self.opening = None;
        // Sessions still being fed get silence until they have torn down.
        if let Some(Audio::Hub(hub)) = self.hub.take() {
            close_hub(hub);
        }
        lock(&self.state).streaming = false;
    }

    async fn shutdown(&mut self, deadline: Instant) {
        self.stop();
        // First, so the settings are on disk even when the sessions use up the deadline (the
        // process may exit right after it).
        self.flush_settings().await;
        // Every session that has not ended yet, including those that were already stopping.
        while !self.owners.is_empty() {
            match tokio::time::timeout_at(deadline, self.ev_rx.recv()).await {
                Ok(Some((id, ev))) => self.on_session_event(id, ev),
                _ => break,
            }
        }
    }

    fn on_session_event(&mut self, id: u64, ev: SessionEvent) {
        let Some(name) = self.owners.get(&id).cloned() else {
            return;
        };
        let current = self.sessions.get(&name).is_some_and(|h| h.id == id);
        if let SessionEvent::Ended(_) = ev {
            self.owners.remove(&id);
            if current {
                self.sessions.remove(&name);
                if self.restart.contains(&name) {
                    // The replacement waited for this; start_session clears the mark.
                    let (streaming, selected) = {
                        let st = lock(&self.state);
                        let selected = st.receivers.iter().any(|r| r.name == name && r.selected);
                        (st.streaming, selected)
                    };
                    if streaming && selected && self.hub.is_some() {
                        self.start_session(&name);
                    } else {
                        self.cancel_restart(&name);
                    }
                } else if let Some(p) = self.reconnects.get_mut(&name)
                    && p.at.is_none()
                    && p.playing_since.is_none()
                {
                    // Timed from now: one session per receiver at a time.
                    let wait = reconnect_wait(p.attempt);
                    p.at = Some(Instant::now() + wait);
                    info!(
                        "[{name}] reconnect attempt {} of {RECONNECT_ATTEMPTS} in {} s",
                        p.attempt,
                        wait.as_secs()
                    );
                }
            }
            self.stop_if_all_failed();
            return;
        }
        // Ignore stale sessions, and the old session's last events during a restart.
        if !current || self.restart.contains(&name) {
            return;
        }
        match ev {
            // It played before, so the firewall is not why a reconnect attempt timed out.
            SessionEvent::FirewallSuspected if self.reconnects.contains_key(&name) => {}
            SessionEvent::FirewallSuspected => {
                lock(&self.state).firewall_suspected = true;
                self.handle(Command::CheckFirewall);
            }
            SessionEvent::ReceiverVolume(p) => {
                // The session filters these too; a NaN here would reach the slider and the file.
                let Some(p) = session::finite_volume(p) else {
                    return;
                };
                if !self.settings.volume_follows_receiver {
                    info!("[{name}] receiver volume {p:.0}% ignored (not followed)");
                    return;
                }
                if let Some(r) = lock(&self.state)
                    .receivers
                    .iter_mut()
                    .find(|r| r.name == name)
                {
                    r.volume_pct = p;
                    r.volume_known = true;
                }
                self.settings.set_volume(&name, p);
                self.save();
            }
            SessionEvent::Status(Status::Error(msg)) => self.on_session_error(&name, msg),
            SessionEvent::Status(Status::Streaming) if self.reconnects.contains_key(&name) => {
                // Kept until it has played for RECONNECT_STABLE (on_session_error).
                if let Some(p) = self.reconnects.get_mut(&name) {
                    p.playing_since = Some(Instant::now());
                }
                info!("[{name}] reconnected");
                if let Some(r) = lock(&self.state)
                    .receivers
                    .iter_mut()
                    .find(|r| r.name == name)
                {
                    r.reconnect = Reconnect::No;
                    r.status = Status::Streaming;
                }
            }
            other => {
                let mut st = lock(&self.state);
                if let Some(r) = st.receivers.iter_mut().find(|r| r.name == name) {
                    match other {
                        SessionEvent::Status(s) => r.status = s,
                        SessionEvent::Stats(s) => {
                            r.stats = Some(s);
                            r.suggestion = advice::suggest_profile(r.profile, &s.loss)
                                .filter(|p| *p != r.profile);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn on_internal(&mut self, msg: Internal) {
        match msg {
            Internal::Endpoints(Ok(list)) => {
                let infos: Vec<_> = list
                    .iter()
                    .map(|e| ::capture::wasapi::DeviceInfo {
                        id: e.id.clone(),
                        friendly_name: e.name.clone(),
                        is_default: e.is_default,
                    })
                    .collect();
                let mut st = lock(&self.state);
                st.endpoints = list;
                if !st.streaming {
                    match capture::choose_endpoint(&infos, &self.settings) {
                        Ok(d) => {
                            st.endpoint = st.endpoints.iter().find(|e| e.id == d.id).cloned();
                            st.endpoint_error = None;
                        }
                        Err(e) => {
                            st.endpoint = None;
                            st.endpoint_error = Some(e);
                        }
                    }
                }
            }
            Internal::Endpoints(Err(e)) => lock(&self.state).endpoint_error = Some(e),
            Internal::Discovered(Ok(list)) => self.merge_receivers(list, true),
            Internal::Discovered(Err(e)) => {
                let mut st = lock(&self.state);
                st.discovering = false;
                st.message = Some(e);
            }
            Internal::Volume(name, pct) => {
                self.probing.remove(&name);
                if let Some(pct) = pct.and_then(session::finite_volume)
                    && !self.sessions.contains_key(&name)
                    && let Some(r) = lock(&self.state)
                        .receivers
                        .iter_mut()
                        .find(|r| r.name == name)
                    && !r.volume_known
                {
                    r.volume_pct = pct;
                    r.volume_known = true;
                }
            }
            Internal::Firewall(generation, fw) => {
                if self.firewall_jobs.finish(generation) {
                    let mut st = lock(&self.state);
                    if fw == FirewallState::Ok {
                        st.firewall_suspected = false;
                    }
                    st.firewall = fw;
                }
            }
            Internal::Capture(attempt, hub) => self.on_capture(attempt, hub),
        }
        self.try_resume(false);
    }

    /// Adds or updates receivers. `browse` = the list is a complete mDNS browse result
    /// (receivers not in it are marked offline); otherwise cached entries, added offline.
    fn merge_receivers(&mut self, found: Vec<Receiver>, browse: bool) {
        let mut to_probe = Vec::new();
        {
            let mut st = lock(&self.state);
            if browse {
                st.discovering = false;
                for r in st.receivers.iter_mut() {
                    r.online = false;
                }
            }
            for f in found {
                if let Some(r) = st.receivers.iter_mut().find(|r| r.name == f.name) {
                    if !browse {
                        continue;
                    }
                    r.addr = f.addr;
                    r.model = f.model;
                    r.online = true;
                    if !r.volume_known && !self.sessions.contains_key(&r.name) {
                        to_probe.push((r.name.clone(), r.addr));
                    }
                } else {
                    to_probe.push((f.name.clone(), f.addr));
                    // Start volume: the saved one. While the receiver's own volume is
                    // followed, the volume read from it (just below) replaces it.
                    let saved = self.settings.volume_of(&f.name);
                    st.receivers.push(ReceiverView {
                        selected: self.settings.is_selected(&f.name),
                        profile: self.settings.profile_for(&f.name),
                        hidden: self.settings.is_hidden(&f.name),
                        name: f.name,
                        addr: f.addr,
                        model: f.model,
                        online: browse,
                        volume_pct: saved.unwrap_or(DEFAULT_VOLUME_PCT),
                        volume_known: saved.is_some() && !self.settings.volume_follows_receiver,
                        status: Status::Idle,
                        stats: None,
                        restarting: false,
                        reconnect: Reconnect::No,
                        suggestion: None,
                        level: None,
                    });
                }
            }
            st.receivers.sort_by_key(|r| r.name.to_lowercase());
        }
        for (name, addr) in to_probe {
            if !self.probing.insert(name.clone()) {
                continue;
            }
            let tx = self.int_tx.clone();
            tokio::spawn(async move {
                let pct = match discovery::read_volume_pct(addr).await {
                    Ok(p) => Some(p),
                    Err(e) => {
                        info!("Volume of \"{name}\" unknown ({e}); using {DEFAULT_VOLUME_PCT}%");
                        None
                    }
                };
                let _ = tx.send(Internal::Volume(name, pct));
            });
        }
    }
}

/// Closes a capture off the runtime's workers: dropping it joins the pump thread, which
/// stops WASAPI.
fn close_hub(hub: CaptureHub) {
    tokio::task::spawn_blocking(move || drop(hub));
}

impl Engine {
    /// Fills the state with the demo endpoints and rooms (instead of WASAPI, mDNS, the
    /// firewall check and the volume probes).
    fn fill_demo(&mut self) {
        let receivers = demo::receivers();
        // The demo settings live in a scratch folder; pinning the rooms there keeps switching
        // a room off and on consistent with what is shown.
        for r in &receivers {
            self.settings.set_selected(&r.name, r.selected);
            self.settings.set_profile(&r.name, r.profile);
            self.settings.set_volume(&r.name, r.volume_pct);
        }
        self.save();
        let endpoints = demo::endpoints();
        let mut st = lock(&self.state);
        st.endpoint = endpoints.iter().find(|e| e.is_default).cloned();
        st.endpoints = endpoints;
        st.receivers = receivers;
        st.firewall = FirewallState::Ok;
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::*;
    use crate::controller::test_support::room;
    use crate::session::{LatencyProfile, SessionCmd, SessionStats};
    use crate::settings::ThemeChoice;

    #[test]
    fn firewall_jobs_do_not_pile_up_and_drop_stale_results() {
        let mut jobs = FirewallJobs::default();
        let check = jobs.begin(false).unwrap();
        // More suspected failures while it runs: no second check.
        assert_eq!(jobs.begin(false), None);
        // An install overrides the running check, whose late result is dropped.
        let install = jobs.begin(true).unwrap();
        assert_eq!(jobs.begin(true), None);
        assert_eq!(jobs.begin(false), None);
        assert!(!jobs.finish(check));
        assert!(jobs.finish(install));
        // Idle again: the next check runs.
        assert!(jobs.begin(false).is_some_and(|g| g > install));
    }

    /// Every settings write, with the (paused) time it ran at.
    type Writes = Arc<Mutex<Vec<(Instant, Settings)>>>;

    /// An engine in demo mode (pretend sessions) with `rooms` found and switched on and a
    /// capture endpoint chosen, not playing yet. Settings writes go nowhere.
    fn idle_engine(rooms: &[&str], mut settings: Settings) -> Engine {
        for name in rooms {
            settings.set_selected(name, true);
        }
        let state = AppState {
            endpoint: demo::endpoints().into_iter().next(),
            receivers: rooms.iter().map(|n| room(n, true, true)).collect(),
            ..Default::default()
        };
        Engine::with_settings(
            Arc::new(Mutex::new(state)),
            Arc::new(|| {}),
            settings,
            true,
            Arc::new(|_: &Settings| {}),
        )
    }

    /// An engine in demo mode that is already streaming to `rooms`.
    fn demo_engine(rooms: &[&str]) -> Engine {
        let mut e = idle_engine(rooms, Settings::default());
        lock(&e.state).streaming = true;
        e.hub = Some(Audio::Demo);
        e
    }

    /// Records the settings writes instead of writing the file.
    fn record_writes(e: &mut Engine) -> Writes {
        let writes = Writes::default();
        let w = writes.clone();
        e.write = Arc::new(move |s: &Settings| w.lock().unwrap().push((Instant::now(), s.clone())));
        writes
    }

    /// Puts a stand-in session for `name` in place, as `start_session` does. The test plays
    /// the session: it reads the commands and sends the events.
    fn stand_in_session(e: &mut Engine, name: &str) -> (u64, mpsc::UnboundedReceiver<SessionCmd>) {
        let id = e.next_id;
        e.next_id += 1;
        let (handle, cmds) = SessionHandle::new(id);
        e.sessions.insert(name.into(), handle);
        e.owners.insert(id, name.into());
        (id, cmds)
    }

    /// The Stop commands a stand-in session has received since the last call.
    fn stops(cmds: &mut mpsc::UnboundedReceiver<SessionCmd>) -> usize {
        std::iter::from_fn(|| cmds.try_recv().ok())
            .filter(|c| matches!(c, SessionCmd::Stop))
            .count()
    }

    fn view(e: &Engine, name: &str) -> ReceiverView {
        lock(&e.state)
            .receivers
            .iter()
            .find(|r| r.name == name)
            .cloned()
            .unwrap()
    }

    fn session_id(e: &Engine, name: &str) -> Option<u64> {
        e.sessions.get(name).map(|h| h.id)
    }

    /// Runs the engine's loop beside `test`, which drives it through the channel. The loop
    /// ends (with a shutdown) when `test` is done and drops the sender.
    async fn run_beside<F: Future<Output = ()>>(
        e: Engine,
        test: impl FnOnce(mpsc::UnboundedSender<Msg>) -> F,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::join!(e.run(rx), test(tx));
    }

    /// Handles session events until `done`, failing after a few seconds.
    async fn pump_until(e: &mut Engine, mut done: impl FnMut(&Engine, u64, &SessionEvent) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let (id, ev) = tokio::time::timeout_at(deadline, e.ev_rx.recv())
                .await
                .expect("timed out waiting for session events")
                .unwrap();
            let stop = done(e, id, &ev);
            e.on_session_event(id, ev);
            if stop {
                return;
            }
        }
    }

    #[tokio::test]
    async fn a_replacement_session_starts_only_after_the_old_one_has_ended() {
        let mut e = demo_engine(&["Kitchen"]);
        e.start_session("Kitchen");
        let first = session_id(&e, "Kitchen").unwrap();
        // Off then on (or Stop then Play) before the first session has torn down.
        e.start_session("Kitchen");
        assert_eq!(session_id(&e, "Kitchen"), Some(first));
        assert_eq!(e.owners.len(), 1);
        assert!(e.restart.contains("Kitchen"));
        pump_until(&mut e, |e, id, ev| {
            // Nothing new starts while the old session is still running.
            assert_eq!(session_id(e, "Kitchen"), Some(first));
            id == first && matches!(ev, SessionEvent::Ended(_))
        })
        .await;
        let second = session_id(&e, "Kitchen").unwrap();
        assert_ne!(second, first);
        assert!(e.restart.is_empty());
        assert_eq!(lock(&e.state).receivers[0].status, Status::Connecting);
    }

    #[tokio::test(start_paused = true)]
    async fn each_session_feeds_a_level_tap_of_its_own() {
        let mut e = demo_engine(&["Kitchen"]);
        e.start_session("Kitchen");
        let first = session_id(&e, "Kitchen").unwrap();
        let tap = view(&e, "Kitchen").level.expect("a level tap");
        pump_until(&mut e, |_, _, ev| {
            *ev == SessionEvent::Status(Status::Streaming)
        })
        .await;
        let since = Instant::now().into_std();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(tap.peak(since, Instant::now().into_std()) > 0.0);

        // Started again (off then on, a profile change): the new session gets a new tap, and
        // the old one stops filling, so the meter never shows what the old session sent.
        e.start_session("Kitchen");
        pump_until(&mut e, |_, id, ev| {
            id == first && matches!(ev, SessionEvent::Ended(_))
        })
        .await;
        let new_tap = view(&e, "Kitchen").level.expect("a level tap");
        assert!(!Arc::ptr_eq(&tap, &new_tap));
        pump_until(&mut e, |_, _, ev| {
            *ev == SessionEvent::Status(Status::Streaming)
        })
        .await;
        let since = Instant::now().into_std();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let now = Instant::now().into_std();
        assert!(new_tap.peak(since, now) > 0.0);
        assert_eq!(tap.peak(since, now), 0.0);
    }

    #[tokio::test]
    async fn stop_cancels_a_pending_replacement() {
        let mut e = demo_engine(&["Kitchen"]);
        e.start_session("Kitchen");
        e.start_session("Kitchen");
        e.stop();
        assert!(e.restart.is_empty());
        assert_eq!(lock(&e.state).receivers[0].status, Status::Idle);
        pump_until(&mut e, |_, _, ev| matches!(ev, SessionEvent::Ended(_))).await;
        assert!(e.sessions.is_empty() && e.owners.is_empty());
    }

    #[test]
    fn the_old_sessions_last_events_are_ignored_during_a_restart() {
        let mut e = demo_engine(&["Kitchen"]);
        let (old, _cmds) = stand_in_session(&mut e, "Kitchen");
        e.start_session("Kitchen");
        let stats = SessionStats {
            packets_sent: 1000,
            ..Default::default()
        };
        e.on_session_event(old, SessionEvent::Status(Status::Streaming));
        e.on_session_event(old, SessionEvent::Stats(stats));
        e.on_session_event(old, SessionEvent::ReceiverVolume(20.0));
        let r = view(&e, "Kitchen");
        assert_eq!(
            (r.status, r.stats, r.volume_pct),
            (Status::Connecting, None, 50.0)
        );
    }

    #[tokio::test]
    async fn the_replacement_sessions_events_show() {
        let mut e = demo_engine(&["Kitchen"]);
        let (old, _cmds) = stand_in_session(&mut e, "Kitchen");
        e.start_session("Kitchen");
        e.on_session_event(old, SessionEvent::Ended(SessionStats::default()));
        let new = session_id(&e, "Kitchen").unwrap();
        let stats = SessionStats {
            packets_sent: 1000,
            ..Default::default()
        };
        e.on_session_event(new, SessionEvent::Status(Status::Streaming));
        e.on_session_event(new, SessionEvent::Stats(stats));
        let r = view(&e, "Kitchen");
        assert_eq!((r.status, r.stats), (Status::Streaming, Some(stats)));
    }

    #[tokio::test]
    async fn volumes_that_are_not_numbers_are_ignored() {
        let mut e = demo_engine(&["Kitchen"]);
        e.start_session("Kitchen");
        let id = session_id(&e, "Kitchen").unwrap();
        e.handle(Command::SetVolume("Kitchen".into(), f32::NAN));
        e.handle(Command::SetVolume("Kitchen".into(), f32::INFINITY));
        e.handle(Command::SetDefaultVolume(f32::NAN));
        e.on_session_event(id, SessionEvent::ReceiverVolume(f32::NAN));
        assert_eq!(lock(&e.state).receivers[0].volume_pct, 50.0);
        assert_eq!(e.settings.volume_of("Kitchen"), None);
        assert_eq!(e.settings.default_volume, settings::DEFAULT_NEW_ROOM_VOLUME);
        // Numbers still get through, clamped.
        e.handle(Command::SetVolume("Kitchen".into(), 120.0));
        assert_eq!(lock(&e.state).receivers[0].volume_pct, 100.0);
        e.on_session_event(id, SessionEvent::ReceiverVolume(40.0));
        assert_eq!(e.settings.volume_of("Kitchen"), Some(40.0));
    }

    #[test]
    fn receiver_volume_moves_the_slider_only_while_it_is_followed() {
        let mut e = demo_engine(&["Kitchen"]);
        let (id, _cmds) = stand_in_session(&mut e, "Kitchen");
        e.handle(Command::SetVolumeFollowsReceiver(false));
        e.on_session_event(id, SessionEvent::ReceiverVolume(40.0));
        assert_eq!(view(&e, "Kitchen").volume_pct, 50.0);
        assert_eq!(e.settings.volume_of("Kitchen"), None);
        e.handle(Command::SetVolumeFollowsReceiver(true));
        e.on_session_event(id, SessionEvent::ReceiverVolume(40.0));
        assert_eq!(view(&e, "Kitchen").volume_pct, 40.0);
        assert_eq!(e.settings.volume_of("Kitchen"), Some(40.0));
    }

    #[tokio::test]
    async fn settings_reach_the_ui_at_once_and_the_file_later() {
        let mut e = demo_engine(&["Kitchen"]);
        let writes = record_writes(&mut e);
        e.handle(Command::SetTheme(ThemeChoice::Dark));
        e.handle(Command::SetVolume("Kitchen".into(), 70.0));
        assert_eq!(lock(&e.state).settings.theme, ThemeChoice::Dark);
        assert!(writes.lock().unwrap().is_empty());
        // Shutdown writes what is still pending.
        e.shutdown(Instant::now() + Duration::from_secs(1)).await;
        let writes = writes.lock().unwrap();
        let [(_, s)] = writes.as_slice() else {
            panic!("{} writes", writes.len());
        };
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert_eq!(s.volume_of("Kitchen"), Some(70.0));
    }

    #[tokio::test(start_paused = true)]
    async fn settings_are_written_once_they_have_not_changed_for_a_while() {
        let mut e = idle_engine(&[], Settings::default());
        let writes = record_writes(&mut e);
        // Disk writes must not block the runtime, whose one thread runs this test.
        let (record, test_thread) = (e.write.clone(), std::thread::current().id());
        e.write = Arc::new(move |s: &Settings| {
            assert_ne!(
                std::thread::current().id(),
                test_thread,
                "written on the runtime"
            );
            record(s);
        });
        let w = writes.clone();
        let start = Instant::now();
        let ms = Duration::from_millis;
        run_beside(e, |tx| async move {
            let send = |c| tx.send(Msg::Cmd(c)).unwrap();
            send(Command::SetTheme(ThemeChoice::Dark));
            tokio::time::sleep(ms(300)).await;
            send(Command::SetVolume("Kitchen".into(), 70.0));
            tokio::time::sleep(ms(300)).await;
            send(Command::SetTheme(ThemeChoice::Light));
            tokio::time::sleep(SAVE_DELAY - ms(1)).await;
            assert!(w.lock().unwrap().is_empty());
            // Paused time does not move on while a write runs, so it is done when this wakes.
            tokio::time::sleep(ms(2)).await;
            assert_eq!(w.lock().unwrap().len(), 1);
            tokio::time::sleep(Duration::from_secs(10)).await;
        })
        .await;
        let writes = writes.lock().unwrap();
        let [(at, s)] = writes.as_slice() else {
            panic!("{} writes", writes.len());
        };
        assert_eq!(*at, start + ms(600) + SAVE_DELAY);
        assert_eq!(s.theme, ThemeChoice::Light);
        assert_eq!(s.volume_of("Kitchen"), Some(70.0));
    }

    #[tokio::test(start_paused = true)]
    async fn a_settings_write_still_running_defers_the_next() {
        let mut e = demo_engine(&["Kitchen"]);
        let writes = record_writes(&mut e);
        // A slow disk: writes wait until the test lets them through (drops `release`).
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (record, gate) = (e.write.clone(), Mutex::new(gate));
        e.write = Arc::new(move |s: &Settings| {
            let _ = gate.lock().unwrap().recv();
            record(s);
        });
        e.handle(Command::SetTheme(ThemeChoice::Dark));
        e.write_settings();
        e.handle(Command::SetTheme(ThemeChoice::Light));
        // The next write is due while the first is still stuck on the disk.
        tokio::time::advance(SAVE_DELAY * 2).await;
        e.write_settings();
        // Tried again a while later, so the older copy cannot land last.
        assert_eq!(e.save_at, Some(Instant::now() + SAVE_DELAY));
        drop(release);
        e.flush_settings().await;
        let themes: Vec<_> = writes
            .lock()
            .unwrap()
            .iter()
            .map(|(_, s)| s.theme)
            .collect();
        assert_eq!(themes, [ThemeChoice::Dark, ThemeChoice::Light]);
    }

    #[test]
    fn a_profile_change_while_the_capture_opens_restarts_nothing() {
        let mut e = demo_engine(&["Kitchen"]);
        let (_, mut cmds) = stand_in_session(&mut e, "Kitchen");
        // Stop, then Play while the old session is still tearing down: the capture is opening.
        e.stop();
        assert_eq!(stops(&mut cmds), 1);
        lock(&e.state).streaming = true;
        e.opening = Some(7);
        e.handle(Command::SetProfile(
            "Kitchen".into(),
            LatencyProfile::Stable,
        ));
        let r = view(&e, "Kitchen");
        assert_eq!((r.profile, r.restarting), (LatencyProfile::Stable, false));
        assert!(e.restart.is_empty());
        assert_eq!(stops(&mut cmds), 0);
    }

    #[test]
    fn a_failed_capture_open_cancels_the_rooms_waiting_to_restart() {
        let mut e = demo_engine(&["Kitchen", "Living Room"]);
        e.hub = None;
        e.opening = Some(7);
        // Kitchen waits for its old session to end, to start with a new profile.
        e.restart.insert("Kitchen".into());
        {
            let mut st = lock(&e.state);
            st.receivers[0].restarting = true;
            st.receivers[0].status = Status::Connecting;
        }
        e.on_capture(7, Err("device gone".into()));
        assert!(e.restart.is_empty());
        let st = lock(&e.state);
        assert!(!st.streaming);
        assert_eq!(st.message.as_deref(), Some("Capture failed: device gone"));
        let r = &st.receivers[0];
        assert_eq!((r.restarting, &r.status), (false, &Status::Idle));
    }

    #[test]
    fn a_capture_opened_for_an_older_start_changes_nothing() {
        let mut e = demo_engine(&["Kitchen"]);
        // Play, Stop, Play: the first start's capture comes back late.
        e.hub = None;
        e.opening = Some(2);
        e.on_internal(Internal::Capture(1, Err("device gone".into())));
        {
            let st = lock(&e.state);
            assert!(st.streaming);
            assert_eq!(st.message, None);
        }
        e.on_internal(Internal::Capture(2, Err("device gone".into())));
        assert!(!lock(&e.state).streaming);
    }

    #[test]
    fn a_firewall_result_from_an_older_job_is_ignored() {
        let mut e = idle_engine(&[], Settings::default());
        // A check was running when the user chose to install the rule.
        let check = e.firewall_jobs.begin(false).unwrap();
        let install = e.firewall_jobs.begin(true).unwrap();
        {
            let mut st = lock(&e.state);
            st.firewall = FirewallState::Installing;
            st.firewall_suspected = true;
        }
        let missing = FirewallState::Missing {
            blocked: false,
            tcp: false,
            udp: false,
        };
        e.on_internal(Internal::Firewall(check, missing));
        assert_eq!(lock(&e.state).firewall, FirewallState::Installing);
        e.on_internal(Internal::Firewall(install, FirewallState::Ok));
        let st = lock(&e.state);
        assert_eq!(st.firewall, FirewallState::Ok);
        assert!(!st.firewall_suspected);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_waits_for_every_unfinished_session() {
        let mut e = demo_engine(&["Kitchen", "Living Room"]);
        e.start_session("Living Room");
        let (old, mut cmds) = stand_in_session(&mut e, "Kitchen");
        // A replacement is pending for one room: its old session must end, and no new one start.
        e.start_session("Kitchen");
        assert_eq!(stops(&mut cmds), 1);
        // The old session takes its time to tear down, until the test lets it end.
        let (end, ended) = tokio::sync::oneshot::channel::<()>();
        let events = e.ev_tx.clone();
        tokio::spawn(async move {
            let _ = ended.await;
            let _ = events.send((old, SessionEvent::Ended(SessionStats::default())));
        });
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        {
            let shutdown = e.shutdown(deadline);
            tokio::pin!(shutdown);
            tokio::select! {
                () = &mut shutdown => panic!("shutdown returned while a session was tearing down"),
                () = tokio::time::sleep(Duration::from_secs(3)) => {}
            }
            end.send(()).unwrap();
            shutdown.await;
        }
        assert!(Instant::now() < deadline);
        assert_eq!(stops(&mut cmds), 1);
        assert!(e.sessions.is_empty() && e.owners.is_empty());
        assert!(!lock(&e.state).streaming);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_gives_up_on_a_session_at_the_deadline() {
        let mut e = demo_engine(&["Kitchen"]);
        let writes = record_writes(&mut e);
        e.handle(Command::SetTheme(ThemeChoice::Dark));
        // A session that never ends.
        let (_, _cmds) = stand_in_session(&mut e, "Kitchen");
        let start = Instant::now();
        let deadline = start + SHUTDOWN_TIMEOUT;
        tokio::time::timeout(SHUTDOWN_TIMEOUT * 2, e.shutdown(deadline))
            .await
            .expect("shutdown waited past its deadline");
        assert_eq!(Instant::now(), deadline);
        // The settings went to disk first, not after the wait: the process may exit next.
        let times: Vec<_> = writes.lock().unwrap().iter().map(|(at, _)| *at).collect();
        assert_eq!(times, [start]);
    }

    fn addr(last: u8) -> SocketAddr {
        SocketAddr::from(([192, 0, 2, last], 7000))
    }

    fn found(name: &str, last: u8, model: Option<&str>) -> Receiver {
        Receiver {
            name: name.into(),
            addr: addr(last),
            model: model.map(Into::into),
        }
    }

    /// Skips the volume reads of these new rooms (they would reach the network).
    fn no_probes(e: &mut Engine, names: &[&str]) {
        e.probing.extend(names.iter().map(|n| n.to_string()));
    }

    #[test]
    fn a_browse_marks_the_rooms_it_did_not_see_offline() {
        let mut e = idle_engine(&["Den", "Kitchen"], Settings::default());
        lock(&e.state).discovering = true;
        e.merge_receivers(vec![found("Kitchen", 99, Some("AudioAccessory6,1"))], true);
        let st = lock(&e.state);
        assert!(!st.discovering);
        let rooms: Vec<_> = st
            .receivers
            .iter()
            .map(|r| (r.name.as_str(), r.online, r.addr, r.model.as_deref()))
            .collect();
        assert_eq!(
            rooms,
            [
                ("Den", false, addr(20), Some("AudioAccessory5,1")),
                ("Kitchen", true, addr(99), Some("AudioAccessory6,1")),
            ]
        );
    }

    #[test]
    fn cached_rooms_never_overwrite_the_found_ones() {
        let mut e = idle_engine(&["Kitchen"], Settings::default());
        no_probes(&mut e, &["Office"]);
        let cached = vec![
            found("Kitchen", 99, None),
            found("Office", 30, Some("AudioAccessory5,1")),
        ];
        e.merge_receivers(cached, false);
        let st = lock(&e.state);
        let rooms: Vec<_> = st
            .receivers
            .iter()
            .map(|r| (r.name.as_str(), r.online, r.addr, r.model.as_deref()))
            .collect();
        assert_eq!(
            rooms,
            [
                ("Kitchen", true, addr(20), Some("AudioAccessory5,1")),
                ("Office", false, addr(30), Some("AudioAccessory5,1")),
            ]
        );
    }

    #[test]
    fn new_rooms_take_their_switches_from_the_settings_and_sort_ignoring_case() {
        let mut s = Settings::default();
        s.set_selected("living room", true);
        s.set_profile("living room", LatencyProfile::Stable);
        s.set_hidden("Bedroom", true);
        let mut e = idle_engine(&["Kitchen"], s);
        no_probes(&mut e, &["living room", "Bedroom", "attic"]);
        let browse = vec![
            found("living room", 22, None),
            found("Bedroom", 23, None),
            found("attic", 24, None),
        ];
        e.merge_receivers(browse, true);
        let st = lock(&e.state);
        let rooms: Vec<_> = st
            .receivers
            .iter()
            .map(|r| (r.name.as_str(), r.selected, r.profile, r.hidden))
            .collect();
        assert_eq!(
            rooms,
            [
                ("attic", false, LatencyProfile::Balanced, false),
                ("Bedroom", false, LatencyProfile::Balanced, true),
                ("Kitchen", true, LatencyProfile::Balanced, false),
                ("living room", true, LatencyProfile::Stable, false),
            ]
        );
    }

    #[tokio::test]
    async fn switching_on_a_hidden_room_keeps_it_off() {
        let mut e = demo_engine(&["Kitchen"]);
        e.handle(Command::SetHidden("Kitchen".into(), true));
        e.handle(Command::SetSelected("Kitchen".into(), true));
        assert!(!view(&e, "Kitchen").selected);
        assert!(!e.settings.is_selected("Kitchen"));
        assert!(e.sessions.is_empty());
    }

    #[test]
    fn hiding_a_playing_room_stops_only_that_room() {
        let mut e = demo_engine(&["Kitchen", "Living Room"]);
        let (_, mut kitchen) = stand_in_session(&mut e, "Kitchen");
        let (_, mut living_room) = stand_in_session(&mut e, "Living Room");
        e.handle(Command::SetHidden("Kitchen".into(), true));
        assert_eq!((stops(&mut kitchen), stops(&mut living_room)), (1, 0));
        let k = view(&e, "Kitchen");
        assert!(k.hidden && !k.selected);
        assert!(view(&e, "Living Room").selected);
    }

    #[tokio::test]
    async fn a_profile_change_restarts_only_that_room_and_only_if_it_changed() {
        let mut e = demo_engine(&["Kitchen", "Living Room"]);
        let (old, mut kitchen) = stand_in_session(&mut e, "Kitchen");
        let (_, mut living_room) = stand_in_session(&mut e, "Living Room");
        e.handle(Command::SetProfile(
            "Kitchen".into(),
            LatencyProfile::Balanced,
        ));
        assert_eq!(stops(&mut kitchen), 0);
        assert!(!view(&e, "Kitchen").restarting);
        e.handle(Command::SetProfile(
            "Kitchen".into(),
            LatencyProfile::Stable,
        ));
        assert_eq!((stops(&mut kitchen), stops(&mut living_room)), (1, 0));
        assert!(view(&e, "Kitchen").restarting);
        // The new session starts once the old one has ended.
        e.on_session_event(old, SessionEvent::Ended(SessionStats::default()));
        assert!(session_id(&e, "Kitchen").is_some_and(|id| id != old));
        let k = view(&e, "Kitchen");
        assert_eq!((k.profile, k.restarting), (LatencyProfile::Stable, false));
    }

    /// Resume on launch is on and the app was playing when it closed.
    fn was_playing() -> Settings {
        let mut s = Settings::default();
        s.resume_on_launch = true;
        s.was_playing = true;
        s
    }

    #[tokio::test]
    async fn resume_plays_only_if_it_is_on_and_the_app_was_playing() {
        for (resume_on_launch, playing) in [(false, true), (true, false), (true, true)] {
            let mut s = Settings::default();
            s.resume_on_launch = resume_on_launch;
            s.was_playing = playing;
            let mut e = idle_engine(&["Kitchen"], s);
            e.handle(Command::Resume);
            assert_eq!(
                lock(&e.state).streaming,
                resume_on_launch && playing,
                "resume_on_launch {resume_on_launch}, was_playing {playing}"
            );
        }
    }

    #[tokio::test]
    async fn resume_does_not_wait_for_hidden_rooms() {
        let mut s = was_playing();
        // On in an older file, and hidden; not found this time.
        s.set_hidden("Den", true);
        s.receivers.get_mut("Den").unwrap().selected = true;
        let mut e = idle_engine(&["Kitchen"], s);
        e.handle(Command::Resume);
        assert!(lock(&e.state).streaming);
        assert_eq!(session_id(&e, "Den"), None);
    }

    #[tokio::test(start_paused = true)]
    async fn resume_plays_the_rooms_found_once_the_wait_is_over() {
        let mut s = was_playing();
        // On when the app closed, not found this time.
        s.set_selected("Attic", true);
        // Demo mode: Kitchen and Living Room are found and on.
        let e = idle_engine(&[], s);
        let state = e.state.clone();
        run_beside(e, |tx| async move {
            tx.send(Msg::Cmd(Command::Resume)).unwrap();
            tokio::time::sleep(RESUME_WAIT - Duration::from_millis(1)).await;
            assert!(!lock(&state).streaming);
            tokio::time::sleep(Duration::from_millis(2)).await;
            let st = lock(&state);
            assert!(st.streaming);
            let playing: Vec<_> = st
                .receivers
                .iter()
                .filter(|r| r.status != Status::Idle)
                .map(|r| r.name.as_str())
                .collect();
            assert_eq!(playing, ["Kitchen", "Living Room"]);
        })
        .await;
    }

    #[tokio::test(start_paused = true)]
    async fn resume_gives_up_with_a_message_when_no_room_turns_up() {
        let mut s = was_playing();
        s.set_selected("Attic", true);
        let e = idle_engine(&[], s);
        let state = e.state.clone();
        run_beside(e, |tx| async move {
            let send = |c| tx.send(Msg::Cmd(c)).unwrap();
            // Only Attic, which is not found, is on.
            send(Command::SetSelected("Kitchen".into(), false));
            send(Command::SetSelected("Living Room".into(), false));
            send(Command::Resume);
            tokio::time::sleep(RESUME_WAIT - Duration::from_millis(1)).await;
            assert_eq!(lock(&state).message, None);
            tokio::time::sleep(Duration::from_millis(2)).await;
            let st = lock(&state);
            assert!(!st.streaming);
            assert!(
                st.message
                    .as_deref()
                    .is_some_and(|m| m.starts_with("Didn't resume playing"))
            );
        })
        .await;
    }

    fn lost(e: &mut Engine, id: u64, why: &str) {
        e.on_session_event(id, SessionEvent::Status(Status::Error(why.into())));
        e.on_session_event(id, SessionEvent::Ended(SessionStats::default()));
    }

    /// Moves paused time to just before the next reconnect is due, checks nothing starts, then
    /// to when it is due. Returns the wait and the new attempt's session.
    async fn wait_for_reconnect(e: &mut Engine, name: &str) -> (Duration, u64) {
        let from = Instant::now();
        let at = e.next_reconnect().expect("a reconnect is scheduled");
        tokio::time::advance(at - from - Duration::from_millis(1)).await;
        e.start_due_reconnects();
        assert_eq!(session_id(e, name), None, "started before it was due");
        tokio::time::advance(Duration::from_millis(1)).await;
        e.start_due_reconnects();
        let id = session_id(e, name).expect("the attempt has started");
        (at - from, id)
    }

    #[tokio::test(start_paused = true)]
    async fn a_lost_room_reconnects_with_growing_waits_then_gives_up_and_stops() {
        let mut e = demo_engine(&["Kitchen"]);
        let (mut id, _cmds) = stand_in_session(&mut e, "Kitchen");
        e.on_session_event(id, SessionEvent::Status(Status::Streaming));
        let mut waits = Vec::new();
        for attempt in 1..=RECONNECT_ATTEMPTS {
            let why = if attempt == 1 {
                "Connection lost: connection closed by receiver"
            } else {
                "No answer within 10 s."
            };
            lost(&mut e, id, why);
            let r = view(&e, "Kitchen");
            assert_eq!(
                (r.reconnect, r.status),
                (Reconnect::Attempt(attempt), Status::Connecting)
            );
            assert!(lock(&e.state).streaming);
            let (wait, next) = wait_for_reconnect(&mut e, "Kitchen").await;
            waits.push(wait);
            id = next;
            // A timed-out attempt is not the firewall: the room played before.
            e.on_session_event(id, SessionEvent::FirewallSuspected);
            assert!(!lock(&e.state).firewall_suspected);
        }
        let secs = Duration::from_secs;
        assert_eq!(waits, [secs(2), secs(4), secs(8)]);
        lost(&mut e, id, "No answer within 10 s.");
        let r = view(&e, "Kitchen");
        assert_eq!(r.reconnect, Reconnect::GaveUp);
        // The message keeps the first cause, not the last attempt's timeout.
        assert!(
            matches!(&r.status, Status::Error(m) if m.starts_with("Connection lost: connection closed by receiver."))
        );
        // Nothing is left playing, so the app stops (the footer offers Play).
        assert!(!lock(&e.state).streaming);
        assert_eq!(e.next_reconnect(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn only_a_reconnect_that_keeps_playing_starts_the_count_afresh() {
        let mut e = demo_engine(&["Kitchen"]);
        let (id, _cmds) = stand_in_session(&mut e, "Kitchen");
        e.on_session_event(id, SessionEvent::Status(Status::Streaming));
        lost(&mut e, id, "Connection lost: connection closed by receiver");
        let (_, id) = wait_for_reconnect(&mut e, "Kitchen").await;
        e.on_session_event(id, SessionEvent::Status(Status::Connecting));
        assert_eq!(view(&e, "Kitchen").reconnect, Reconnect::Attempt(1));
        e.on_session_event(id, SessionEvent::Status(Status::Streaming));
        let r = view(&e, "Kitchen");
        assert_eq!((r.reconnect, r.status), (Reconnect::No, Status::Streaming));
        // Dropped again soon after: that attempt failed, so the count goes on.
        tokio::time::advance(RECONNECT_STABLE - Duration::from_secs(1)).await;
        lost(&mut e, id, "Connection lost: no answer within 10 s");
        assert_eq!(view(&e, "Kitchen").reconnect, Reconnect::Attempt(2));
        let (_, id) = wait_for_reconnect(&mut e, "Kitchen").await;
        e.on_session_event(id, SessionEvent::Status(Status::Streaming));
        // Played long enough: the next loss is a fresh one.
        tokio::time::advance(RECONNECT_STABLE).await;
        lost(&mut e, id, "Connection lost: no answer within 10 s");
        assert_eq!(view(&e, "Kitchen").reconnect, Reconnect::Attempt(1));
        assert_eq!(
            e.next_reconnect(),
            Some(Instant::now() + Duration::from_secs(2))
        );
    }

    #[test]
    fn a_failed_first_start_is_not_retried_and_stops_once_no_room_plays() {
        let mut e = demo_engine(&["Kitchen", "Living Room"]);
        let (kitchen, _k) = stand_in_session(&mut e, "Kitchen");
        let (living, _l) = stand_in_session(&mut e, "Living Room");
        e.on_session_event(living, SessionEvent::Status(Status::Streaming));
        e.on_session_event(kitchen, SessionEvent::Status(Status::Connecting));
        lost(&mut e, kitchen, "No answer within 10 s.");
        let r = view(&e, "Kitchen");
        assert_eq!(r.reconnect, Reconnect::No);
        assert!(matches!(r.status, Status::Error(_)));
        assert_eq!(e.next_reconnect(), None);
        // Living Room still plays.
        assert!(lock(&e.state).streaming);
        // Switched off, it ends normally; the only room left on has failed.
        e.handle(Command::SetSelected("Living Room".into(), false));
        e.on_session_event(living, SessionEvent::Status(Status::Idle));
        e.on_session_event(living, SessionEvent::Ended(SessionStats::default()));
        assert!(!lock(&e.state).streaming);
    }

    #[test]
    fn switching_off_the_last_room_stops() {
        let mut e = demo_engine(&["Kitchen", "Living Room", "Den"]);
        // On in an older settings file, and hidden: it never plays, so it doesn't count.
        lock(&e.state).receivers[2].hidden = true;
        e.settings.was_playing = true;
        let (kitchen, _k) = stand_in_session(&mut e, "Kitchen");
        let (living, _l) = stand_in_session(&mut e, "Living Room");
        e.on_session_event(kitchen, SessionEvent::Status(Status::Streaming));
        e.on_session_event(living, SessionEvent::Status(Status::Streaming));
        // One room still on: it keeps playing.
        e.handle(Command::SetSelected("Kitchen".into(), false));
        assert!(lock(&e.state).streaming);
        e.handle(Command::SetSelected("Living Room".into(), false));
        assert!(!lock(&e.state).streaming);
        // As with Stop, there is nothing to resume on the next launch.
        assert!(!e.settings.was_playing);
    }

    #[tokio::test(start_paused = true)]
    async fn switching_off_or_stop_cancels_a_pending_reconnect() {
        for stop in [false, true] {
            let mut e = demo_engine(&["Kitchen"]);
            let (id, _cmds) = stand_in_session(&mut e, "Kitchen");
            e.on_session_event(id, SessionEvent::Status(Status::Streaming));
            lost(&mut e, id, "Connection lost: connection closed by receiver");
            if stop {
                e.handle(Command::Stop);
            } else {
                e.handle(Command::SetSelected("Kitchen".into(), false));
            }
            let r = view(&e, "Kitchen");
            assert_eq!((r.reconnect, r.status), (Reconnect::No, Status::Idle));
            assert_eq!(e.next_reconnect(), None);
            tokio::time::advance(Duration::from_secs(60)).await;
            e.start_due_reconnects();
            assert_eq!(session_id(&e, "Kitchen"), None);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_engine_loop_starts_a_reconnect_when_it_is_due() {
        let mut e = demo_engine(&["Kitchen"]);
        let (id, _cmds) = stand_in_session(&mut e, "Kitchen");
        e.on_session_event(id, SessionEvent::Status(Status::Streaming));
        lost(&mut e, id, "Connection lost: connection closed by receiver");
        let state = e.state.clone();
        let kitchen = move || {
            lock(&state)
                .receivers
                .iter()
                .find(|r| r.name == "Kitchen")
                .map(|r| r.status.clone())
        };
        run_beside(e, |tx| async move {
            let _tx = tx;
            // The demo session plays 1 s after it starts, which is 2 s after the loss.
            tokio::time::sleep(Duration::from_millis(2990)).await;
            assert_ne!(kitchen(), Some(Status::Streaming));
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(kitchen(), Some(Status::Streaming));
        })
        .await;
    }
}
