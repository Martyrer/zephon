//! Demo mode: made-up endpoints, rooms and sessions for screenshots (README), so they never
//! show the author's devices. Nothing here touches a device or the network.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::Instant;

use super::{DEFAULT_VOLUME_PCT, EndpointView, ReceiverView, Reconnect};
use crate::advice::{LOSS_WINDOW, LossStats};
use crate::level::LevelTap;
use crate::session::{
    self, LatencyProfile, SessionCmd, SessionEvent, SessionHandle, SessionStats, Status,
};

/// How long a pretend session shows "Connecting…".
const CONNECT: Duration = Duration::from_secs(1);
const STATS_INTERVAL: Duration = Duration::from_secs(5);
/// One pretend capture chunk.
const CHUNK: Duration = Duration::from_millis(10);

pub(super) fn endpoints() -> Vec<EndpointView> {
    [
        ("Speakers (USB Audio DAC)", true),
        ("Headphones (USB Audio DAC)", false),
        ("Monitor (HDMI Audio)", false),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, is_default))| EndpointView {
        id: format!("demo-endpoint-{i}"),
        name: name.into(),
        is_default,
    })
    .collect()
}

/// Name, IP, model, (volume, profile) when it is on, online.
type Room = (
    &'static str,
    [u8; 4],
    &'static str,
    Option<(f32, LatencyProfile)>,
    bool,
);

/// Sorted by name, like discovered rooms.
pub(super) fn receivers() -> Vec<ReceiverView> {
    const HOMEPOD_MINI: &str = "AudioAccessory5,1";
    let rooms: [Room; 4] = [
        ("Den", [192, 0, 2, 30], "AppleTV14,1", None, false),
        (
            "Kitchen",
            [192, 0, 2, 21],
            HOMEPOD_MINI,
            Some((45.0, LatencyProfile::Balanced)),
            true,
        ),
        (
            "Living Room",
            [192, 0, 2, 22],
            HOMEPOD_MINI,
            Some((60.0, LatencyProfile::Stable)),
            true,
        ),
        ("Office", [192, 0, 2, 23], HOMEPOD_MINI, None, true),
    ];
    rooms
        .into_iter()
        .map(|(name, ip, model, on, online)| {
            let (volume_pct, profile) =
                on.unwrap_or((DEFAULT_VOLUME_PCT, LatencyProfile::Balanced));
            ReceiverView {
                name: name.into(),
                addr: SocketAddr::from((ip, 7000)),
                model: Some(model.into()),
                online,
                selected: on.is_some(),
                volume_pct,
                volume_known: true,
                status: Status::Idle,
                stats: None,
                profile,
                restarting: false,
                reconnect: Reconnect::No,
                suggestion: None,
                hidden: false,
                level: None,
            }
        })
        .collect()
}

/// Loss figures that make the rooms look alive: resends per minute, none lost.
fn stats(name: &str, playing_for: Duration) -> SessionStats {
    let resends_per_min = match name {
        "Kitchen" => 2.0,
        "Living Room" => 6.0,
        _ => 0.0,
    };
    let secs = playing_for.as_secs_f64();
    let requests = (resends_per_min * secs / 60.0) as u64;
    SessionStats {
        packets_sent: (secs * 44_100.0 / 352.0) as u64,
        timing_requests: (secs / 3.0) as u64,
        retransmit_requests: requests,
        retransmit_served: requests * 2,
        retransmit_missing: 0,
        drift_ppm: 1.8,
        buffer_fill: 3,
        loss: LossStats {
            window: LOSS_WINDOW.min(playing_for),
            resend_requests_per_min: resends_per_min,
            missed_per_min: 0.0,
        },
    }
}

/// The peak of a pretend chunk captured `t` seconds into playing: a beat at 120 BPM over a
/// level that drifts slowly, a little different in each room (`seed`).
fn demo_peak(t: f64, seed: u32) -> f32 {
    let beat = (t * 2.0).fract();
    let kick = (-beat * 7.0).exp();
    let off_beat = (-((t * 2.0 + 0.5).fract()) * 12.0).exp() * 0.45;
    let phrase = 0.55 + 0.3 * (t * 0.37 + f64::from(seed % 7)).sin();
    // A little hash noise per chunk, so the bar never moves like a metronome.
    let n = ((t * 100.0) as u32)
        .wrapping_add(seed)
        .wrapping_mul(2_654_435_761);
    let noise = f64::from(n >> 24) / 255.0 * 0.15;
    ((kick.max(off_beat) * 0.9 + noise) * phrase).min(1.0) as f32
}

/// A pretend session: "Connecting…", playing after [`CONNECT`], loss stats every few
/// seconds, made-up peaks into `level` every chunk while playing, idle on Stop. Volume changes
/// only update the state (the controller's job).
pub(super) fn spawn_session(
    id: u64,
    name: String,
    level: Arc<LevelTap>,
    events: session::EventTx,
) -> SessionHandle {
    let (handle, mut cmds) = SessionHandle::new(id);
    tokio::spawn(async move {
        let emit = |e: SessionEvent| {
            let _ = events.send((id, e));
        };
        emit(SessionEvent::Status(Status::Connecting));
        let connected = tokio::time::sleep(CONNECT);
        tokio::pin!(connected);
        let mut playing_since: Option<Instant> = None;
        let mut report = tokio::time::interval(STATS_INTERVAL);
        let mut chunks = tokio::time::interval(CHUNK);
        chunks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let seed = name
            .bytes()
            .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b.into()));
        loop {
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    Some(SessionCmd::SetVolume(_)) => {}
                    Some(SessionCmd::Stop) | None => break,
                },
                _ = &mut connected, if playing_since.is_none() => {
                    playing_since = Some(Instant::now());
                    emit(SessionEvent::Status(Status::Streaming));
                }
                _ = report.tick(), if playing_since.is_some() => {
                    let since = playing_since.unwrap_or_else(Instant::now);
                    emit(SessionEvent::Stats(stats(&name, since.elapsed().max(STATS_INTERVAL))));
                }
                at = chunks.tick(), if playing_since.is_some() => {
                    let since = playing_since.unwrap_or(at);
                    let peak = demo_peak(at.duration_since(since).as_secs_f64(), seed);
                    level.record(at.into_std(), peak);
                }
            }
        }
        emit(SessionEvent::Status(Status::Idle));
        emit(SessionEvent::Ended(SessionStats::default()));
    });
    handle
}
