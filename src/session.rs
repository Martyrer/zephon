//! One AirPlay 2 session to one receiver: start, 2 s keep-alive, live volume, stop.
//! Modelled on Nyx `apps/nyxr/src/ap2run.rs` (`run_ap2`) and `apps/nyx-refrain/src/engine.rs`.
//!
//! Each session runs as its own tokio task, so one receiver failing never stops the others.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use raop::ap2::remote::Command as RemoteCommand;
use raop::ap2::session::{Ap2Config, Ap2Error, Ap2Session, MIN_SYNC_LATENCY_MS};
use raop::volume::pct_to_db;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::advice::{LOSS_WINDOW, LossStats};
use crate::capture::{FanoutSource, QueueControl};

/// How long a session start may take before we give up. Without the firewall rule the
/// receiver waits about 32 s and then answers the stream SETUP with 400; fail sooner.
pub const START_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a control request (keep-alive, volume) may wait for its answer before the session
/// counts as lost. Nyx waits forever, so a receiver that keeps the connection open but stops
/// answering would otherwise hold the session indefinitely. On a lossy Wi-Fi link an answer
/// that TCP has to resend often takes over 3 s while the audio still plays, so this is well
/// above that. Stop never waits for it ([`control`]).
const CONTROL_TIMEOUT: Duration = Duration::from_secs(10);
/// TEARDOWN wait after Stop, short enough that a slow receiver still gets its TEARDOWN within
/// the controller's `SHUTDOWN_TIMEOUT` when the app closes.
const TEARDOWN_TIMEOUT: Duration = Duration::from_millis(1500);
/// TEARDOWN wait once the receiver has already stopped answering: it is sent as a courtesy.
const TEARDOWN_AFTER_LOSS: Duration = Duration::from_millis(500);
/// How long Stop waits for a start that was given up on, so the receiver is usually torn down
/// before the controller starts its next session ([`abandon_start`]).
const ABANDON_GRACE: Duration = Duration::from_secs(2);
/// How long a start that was given up on may then keep running in the background. Without
/// the firewall rule the receiver answers after about 32 s, so this covers that case.
const ABANDON_LIMIT: Duration = Duration::from_secs(30);
const FEEDBACK_INTERVAL: Duration = Duration::from_secs(2);
const STATS_INTERVAL: Duration = Duration::from_secs(5);
/// Volume for a receiver whose current volume is unknown (and the start volume when the one
/// asked for is not a number).
pub const DEFAULT_VOLUME_PCT: f32 = 30.0;

/// Named trade-off between lag and robustness. Lag figures were measured on a HomePod mini
/// (behind the local speakers): low about 125 ms, balanced about 165 ms, stable about 257 ms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LatencyProfile {
    /// Sync latency -140 ms (about 125 ms lag). Glitched in a listening test.
    Low,
    /// Sync latency -100 ms (about 165 ms lag). Default.
    #[default]
    Balanced,
    /// Sync latency 0 ms (about 257 ms lag). For bad Wi-Fi.
    Stable,
}

impl LatencyProfile {
    pub const ALL: [LatencyProfile; 3] = [Self::Low, Self::Balanced, Self::Stable];

    pub fn sync_latency_ms(self) -> i32 {
        match self {
            Self::Low => -140,
            Self::Balanced => -100,
            Self::Stable => 0,
        }
        .clamp(MIN_SYNC_LATENCY_MS, 3000)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Balanced => "balanced",
            Self::Stable => "stable",
        }
    }

    /// Approximate lag behind the local speakers, measured on a HomePod mini.
    pub fn lag_ms(self) -> u32 {
        match self {
            Self::Low => 125,
            Self::Balanced => 165,
            Self::Stable => 257,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub name: String,
    pub addr: SocketAddr,
    /// Start volume; the receiver never starts louder than this.
    pub volume_pct: f32,
    pub profile: LatencyProfile,
    pub interface: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Idle,
    Connecting,
    Streaming,
    Error(String),
}

/// Session counters. The retransmit counters are Nyx's public `Ap2Stats`, counted by its RAOP
/// control-channel thread (`raop/src/ap2/session.rs`) when the receiver asks for lost packets.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SessionStats {
    pub packets_sent: u64,
    pub timing_requests: u64,
    /// Retransmit requests received from the receiver (each covers a run of lost packets).
    pub retransmit_requests: u64,
    /// Packets resent in answer to those requests.
    pub retransmit_served: u64,
    /// Packets requested but no longer available (older than the ~4 s history, or never sent).
    pub retransmit_missing: u64,
    pub drift_ppm: f64,
    pub buffer_fill: usize,
    /// Loss rates over the last ~60 s.
    pub loss: LossStats,
}

/// Turns cumulative retransmit counters, sampled every few seconds, into per-minute rates
/// over the last [`LOSS_WINDOW`].
#[derive(Debug, Default)]
pub struct LossWindow {
    /// (time, retransmit requests, missing packets), oldest first.
    samples: VecDeque<(Instant, u64, u64)>,
}

impl LossWindow {
    /// Adds a sample of the cumulative counters and returns the rates over the window.
    pub fn update(&mut self, now: Instant, requests: u64, missing: u64) -> LossStats {
        self.samples.push_back((now, requests, missing));
        // Keep one sample at or before the window start, so the window spans a full minute.
        while self.samples.len() > 2 && now.duration_since(self.samples[1].0) >= LOSS_WINDOW {
            self.samples.pop_front();
        }
        let (t0, r0, m0) = self.samples[0];
        let window = now.duration_since(t0);
        let minutes = window.as_secs_f64() / 60.0;
        if minutes <= 0.0 {
            return LossStats::default();
        }
        LossStats {
            window,
            resend_requests_per_min: requests.saturating_sub(r0) as f64 / minutes,
            missed_per_min: missing.saturating_sub(m0) as f64 / minutes,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    Status(Status),
    /// The receiver reported a volume change (phone, Home app, buttons), 0-100 %.
    ReceiverVolume(f32),
    Stats(SessionStats),
    /// The start failed in the way a missing firewall rule fails.
    FirewallSuspected,
    /// The task has finished (after TEARDOWN, or after an error).
    Ended(SessionStats),
}

pub enum SessionCmd {
    SetVolume(f32),
    Stop,
}

/// Events are tagged with the session's id so the owner can ignore stale sessions.
pub type EventTx = mpsc::UnboundedSender<(u64, SessionEvent)>;

pub struct SessionHandle {
    pub id: u64,
    cmd: mpsc::UnboundedSender<SessionCmd>,
}

impl SessionHandle {
    /// A handle and the command channel for a session task (a real one from [`spawn`], or
    /// the controller's demo stand-in).
    pub fn new(id: u64) -> (Self, mpsc::UnboundedReceiver<SessionCmd>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { id, cmd: tx }, rx)
    }

    pub fn set_volume(&self, pct: f32) {
        let _ = self.cmd.send(SessionCmd::SetVolume(pct));
    }

    pub fn stop(&self) {
        let _ = self.cmd.send(SessionCmd::Stop);
    }
}

/// Spawns the session task on the current tokio runtime.
pub fn spawn(
    id: u64,
    spec: SessionSpec,
    source: FanoutSource,
    queue: QueueControl,
    events: EventTx,
) -> SessionHandle {
    let (handle, rx) = SessionHandle::new(id);
    tokio::spawn(async move {
        let emit = |e: SessionEvent| {
            let _ = events.send((id, e));
        };
        let stats = run(&spec, source, queue, rx, &emit).await;
        emit(SessionEvent::Ended(stats));
    });
    handle
}

fn firewall_like(e: &Ap2Error) -> bool {
    matches!(e, Ap2Error::Status { method, status, .. } if method == "SETUP" && (*status == 400 || *status == 500))
}

async fn run(
    spec: &SessionSpec,
    source: FanoutSource,
    queue: QueueControl,
    mut cmds: mpsc::UnboundedReceiver<SessionCmd>,
    emit: &(dyn Fn(SessionEvent) + Sync),
) -> SessionStats {
    let name = &spec.name;
    emit(SessionEvent::Status(Status::Connecting));
    if let std::net::IpAddr::V4(ip) = spec.addr.ip() {
        match crate::discovery::lan_interface_for(ip) {
            Some((iface, local)) => info!("[{name}] LAN interface {iface}, local IP {local}"),
            None => info!("[{name}] no matching LAN interface; OS routing"),
        }
    }
    let latency_ms = spec.profile.sync_latency_ms();
    let start_pct = finite_volume(spec.volume_pct).unwrap_or_else(|| {
        warn!(
            "[{name}] start volume {} is not a number; using {DEFAULT_VOLUME_PCT}%",
            spec.volume_pct
        );
        DEFAULT_VOLUME_PCT
    });
    let mut volume_pct = start_pct;
    let cfg = Ap2Config {
        target: spec.addr,
        interface_name: spec.interface.clone(), // None = auto (skips vEthernet / VPN adapters)
        latency_frames: latency_ms * 441 / 10,
        volume_db: pct_to_db(volume_pct),
        // Ap2Config::default() is quiet mode at -144 dB: it streams silence.
        quiet_mode: false,
        ..Default::default()
    };
    info!(
        "[{name}] connecting to {} (profile {:?}, sync latency {latency_ms} ms, volume {volume_pct:.0}%)",
        spec.addr, spec.profile
    );
    let ring = pipeline::DEFAULT_RING_BUFFER_PACKETS;
    let metrics = Arc::new(pipeline::PipelineMetricsTracker::new(ring));

    // Boxed so that a start given up on can be handed to `abandon_start` instead of dropped.
    let mut start = Box::pin(Ap2Session::start_with_source(
        cfg,
        Box::new(source),
        metrics.clone(),
        ring,
    ));
    let deadline = tokio::time::sleep(START_TIMEOUT);
    tokio::pin!(deadline);
    #[allow(clippy::large_enum_variant)] // one value, matched right below
    enum Started {
        Done(Result<Ap2Session, Ap2Error>),
        Stopped,
        TimedOut,
    }
    // Volume changes made while connecting are applied once streaming.
    let started = loop {
        tokio::select! {
            r = &mut start => break Started::Done(r),
            _ = &mut deadline => break Started::TimedOut,
            cmd = cmds.recv() => match cmd {
                Some(SessionCmd::SetVolume(p)) => {
                    if let Some(p) = finite_volume(p) {
                        volume_pct = p;
                    }
                }
                Some(SessionCmd::Stop) | None => break Started::Stopped,
            },
        }
    };
    let mut session = match started {
        Started::Stopped => {
            queue.close();
            info!("[{name}] stopped while connecting");
            emit(SessionEvent::Status(Status::Idle));
            abandon_start(start, name).await;
            return SessionStats::default();
        }
        Started::TimedOut => {
            queue.close();
            let msg = format!(
                "No answer within {} s. The firewall rule is likely missing (the receiver's timing requests are dropped).",
                START_TIMEOUT.as_secs()
            );
            warn!("[{name}] {msg}");
            emit(SessionEvent::FirewallSuspected);
            emit(SessionEvent::Status(Status::Error(msg)));
            abandon_start(start, name).await;
            return SessionStats::default();
        }
        Started::Done(Err(e)) => {
            queue.close();
            warn!("[{name}] start failed: {e}");
            let msg = if firewall_like(&e) {
                emit(SessionEvent::FirewallSuspected);
                format!("{e}. Check the firewall rule.")
            } else {
                e.to_string()
            };
            emit(SessionEvent::Status(Status::Error(msg)));
            return SessionStats::default();
        }
        Started::Done(Ok(s)) => s,
    };
    queue.start_feeding();
    info!("[{name}] streaming");
    emit(SessionEvent::Status(Status::Streaming));
    let mut error: Option<String> = None;
    // A volume asked for while a request (or the start) was waiting; sent next.
    let mut volume = ((volume_pct - start_pct).abs() > 0.5).then_some(volume_pct);

    let ap2_stats = session.stats();
    // The loss window starts when streaming starts, with all counters at zero.
    let mut loss_window = LossWindow::default();
    loss_window.update(Instant::now(), 0, 0);
    let mut snapshot = |m: &pipeline::PipelineMetricsTracker| {
        let s = m.snapshot();
        let retransmit_requests = ap2_stats.retransmit_requests.load(Relaxed);
        let retransmit_missing = ap2_stats.retransmit_missing.load(Relaxed);
        SessionStats {
            packets_sent: ap2_stats.packets_sent.load(Relaxed),
            timing_requests: ap2_stats.timing_requests.load(Relaxed),
            retransmit_requests,
            retransmit_served: ap2_stats.retransmit_served.load(Relaxed),
            retransmit_missing,
            drift_ppm: s.estimated_drift_ppm,
            buffer_fill: s.buffer_fill_packets,
            loss: loss_window.update(Instant::now(), retransmit_requests, retransmit_missing),
        }
    };
    let mut remote = session.take_remote_commands();
    // After a slow request, carry on at the normal pace instead of catching up in a burst.
    let mut feedback = tokio::time::interval(FEEDBACK_INTERVAL);
    feedback.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    feedback.tick().await;
    let mut report = tokio::time::interval(STATS_INTERVAL);
    report.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    report.tick().await;

    // Control requests take commands while they wait ([`control`]), so a Stop is handled at
    // once even when the receiver has stopped answering.
    loop {
        if let Some(p) = volume.take() {
            match apply_volume(&mut session, name, p, &mut cmds, &mut volume).await {
                Next::Continue => continue,
                Next::Stop => break,
                Next::Lost(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        tokio::select! {
            biased;
            cmd = cmds.recv() => match cmd {
                Some(SessionCmd::SetVolume(p)) => {
                    // Coalesce a burst of slider moves into the latest value.
                    volume = Some(p);
                    let mut stop = false;
                    while let Ok(next) = cmds.try_recv() {
                        match next {
                            SessionCmd::SetVolume(n) => volume = Some(n),
                            SessionCmd::Stop => { stop = true; break; }
                        }
                    }
                    if stop { break; }
                }
                Some(SessionCmd::Stop) | None => break,
            },
            _ = feedback.tick() => match control(session.feedback(), &mut cmds, &mut volume).await {
                Control::Answered(()) => {}
                Control::Stopped => break,
                Control::Failed(e) => {
                    warn!("[{name}] keep-alive failed: {e}");
                    error = Some(format!("Connection lost: {e}"));
                    break;
                }
            },
            command = async {
                match remote.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => match command {
                Some(RemoteCommand::Volume(v)) => match finite_volume(v * 100.0) {
                    Some(pct) => {
                        info!("[{name}] receiver volume changed to {pct:.0}%");
                        emit(SessionEvent::ReceiverVolume(pct));
                    }
                    None => warn!("[{name}] receiver reported volume {v}; ignored"),
                },
                Some(other) => debug!("[{name}] receiver command ignored: {other:?}"),
                None => remote = None,
            },
            _ = report.tick() => {
                let st = snapshot(&metrics);
                debug!("[{name}] {}", metrics.snapshot().to_json_line());
                debug!("[{name}] {}", stats_line(spec.profile, &st));
                emit(SessionEvent::Stats(st));
            }
        }
    }

    // Once the receiver has stopped answering, TEARDOWN is only a courtesy.
    let limit = if error.is_some() {
        TEARDOWN_AFTER_LOSS
    } else {
        TEARDOWN_TIMEOUT
    };
    // A Stop that came in while the connection was failing: the user asked for idle.
    if error.is_some() && stop_requested(&mut cmds) {
        error = None;
    }
    let final_stats = snapshot(&metrics);
    info!(
        "[{name}] stopping (TEARDOWN); {}",
        stats_line(spec.profile, &final_stats)
    );
    teardown(session, name, limit).await;
    emit(SessionEvent::Status(match error {
        Some(e) => Status::Error(e),
        None => Status::Idle,
    }));
    final_stats
}

/// Why a control request failed.
enum ControlError {
    /// The receiver answered with an error, or the connection failed.
    Failed(Ap2Error),
    /// No answer within [`CONTROL_TIMEOUT`].
    NoAnswer,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed(e) => e.fmt(f),
            Self::NoAnswer => write!(f, "no answer within {} s", CONTROL_TIMEOUT.as_secs()),
        }
    }
}

/// What became of a control request.
enum Control<T> {
    Answered(T),
    Failed(ControlError),
    /// A Stop came in (or the owner went away) while it waited; the request was given up.
    Stopped,
}

/// What the session loop does next.
enum Next {
    Continue,
    Stop,
    /// The connection is lost (with the message for the user).
    Lost(String),
}

/// Runs one control request on a started session, bounded by [`CONTROL_TIMEOUT`], while still
/// taking commands: a Stop gives the request up at once, and the latest volume asked for
/// meanwhile is kept in `volume` for after it.
///
/// Giving up the wait drops the request mid-flight, possibly with only part of its encrypted
/// frame written, so the control connection cannot be trusted afterwards. That is fine only
/// because every caller then ends the session ([`ControlError::NoAnswer`] or
/// [`Control::Stopped`]), sending at most a TEARDOWN; nothing else is sent on that connection.
async fn control<T>(
    request: impl Future<Output = Result<T, Ap2Error>>,
    cmds: &mut mpsc::UnboundedReceiver<SessionCmd>,
    volume: &mut Option<f32>,
) -> Control<T> {
    let request = tokio::time::timeout(CONTROL_TIMEOUT, request);
    tokio::pin!(request);
    loop {
        tokio::select! {
            biased;
            cmd = cmds.recv() => match cmd {
                Some(SessionCmd::SetVolume(p)) => *volume = Some(p),
                Some(SessionCmd::Stop) | None => return Control::Stopped,
            },
            r = &mut request => return match r {
                Ok(Ok(v)) => Control::Answered(v),
                Ok(Err(e)) => Control::Failed(ControlError::Failed(e)),
                Err(_) => Control::Failed(ControlError::NoAnswer),
            },
        }
    }
}

/// `pct` clamped to 0-100, or `None` when it is not a number. Clamping keeps a NaN, and Nyx's
/// `pct_to_db(NaN)` is 0 dB: full volume.
pub fn finite_volume(pct: f32) -> Option<f32> {
    pct.is_finite().then(|| pct.clamp(0.0, 100.0))
}

/// Sets the receiver volume. The connection counts as lost only when the receiver did not
/// answer; a refused change is just logged. A `pct` that is not a number is ignored, keeping
/// the current volume.
async fn apply_volume(
    session: &mut Ap2Session,
    name: &str,
    pct: f32,
    cmds: &mut mpsc::UnboundedReceiver<SessionCmd>,
    volume: &mut Option<f32>,
) -> Next {
    let Some(pct) = finite_volume(pct) else {
        warn!("[{name}] volume {pct} ignored (not a number)");
        return Next::Continue;
    };
    let db = pct_to_db(pct);
    match control(session.set_volume(db), cmds, volume).await {
        Control::Answered(()) => debug!("[{name}] volume set to {pct:.0}% ({db:.1} dB)"),
        Control::Stopped => return Next::Stop,
        Control::Failed(e @ ControlError::NoAnswer) => {
            warn!("[{name}] set volume failed: {e}");
            return Next::Lost(format!("Connection lost: {e}"));
        }
        Control::Failed(e) => warn!("[{name}] set volume failed: {e}"),
    }
    Next::Continue
}

/// Whether a Stop is queued (or the owner has gone).
fn stop_requested(cmds: &mut mpsc::UnboundedReceiver<SessionCmd>) -> bool {
    loop {
        match cmds.try_recv() {
            Ok(SessionCmd::Stop) | Err(mpsc::error::TryRecvError::Disconnected) => return true,
            Ok(SessionCmd::SetVolume(_)) => {}
            Err(mpsc::error::TryRecvError::Empty) => return false,
        }
    }
}

/// Stops the session's threads, then sends TEARDOWN and waits up to `limit` for the answer.
/// Nyx sets its stop flags and joins its threads before it sends TEARDOWN, so giving up the
/// wait never leaves a thread running.
async fn teardown(session: Ap2Session, name: &str, limit: Duration) {
    if tokio::time::timeout(limit, session.stop()).await.is_err() {
        warn!(
            "[{name}] TEARDOWN got no answer within {} ms; closing anyway",
            limit.as_millis()
        );
    }
}

/// Finishes a start that was given up on (Stop while connecting, or [`START_TIMEOUT`])
/// instead of dropping it. Nyx sets none of its stop flags when its start future is
/// dropped, which would leave the session's feeder thread (polling every few ms at audio
/// priority), the ap2-timing thread with its UDP socket, and the events task running until
/// the app exits. Run to its end, Nyx cleans up after itself: a session that does start is
/// torn down here; when the start fails Nyx stops the feeder, but (at rev 0a2ef7b) still
/// leaves the timing thread and the events task running, which only Nyx can fix.
///
/// Waits up to [`ABANDON_GRACE`] so a receiver is usually torn down before the controller
/// starts the next session for it (two sessions would fight over the receiver); after that
/// the start finishes in the background, so Stop and shutdown are not held up.
async fn abandon_start<F>(mut start: Pin<Box<F>>, name: &str)
where
    F: Future<Output = Result<Ap2Session, Ap2Error>> + Send + 'static,
{
    match tokio::time::timeout(ABANDON_GRACE, &mut start).await {
        Ok(r) => finish_abandoned(r, name).await,
        Err(_) => {
            debug!(
                "[{name}] the start given up on is still running; finishing it in the background"
            );
            let name = name.to_string();
            tokio::spawn(async move {
                match tokio::time::timeout(ABANDON_LIMIT, start).await {
                    Ok(r) => finish_abandoned(r, &name).await,
                    Err(_) => warn!(
                        "[{name}] the start given up on is still unanswered after {} s; dropping \
                         it (its Nyx helper threads keep running until exit)",
                        (ABANDON_GRACE + ABANDON_LIMIT).as_secs()
                    ),
                }
            });
        }
    }
}

async fn finish_abandoned(r: Result<Ap2Session, Ap2Error>, name: &str) {
    match r {
        Ok(s) => {
            info!("[{name}] the start given up on has connected after all; tearing it down");
            teardown(s, name, TEARDOWN_TIMEOUT).await;
        }
        Err(e) => debug!("[{name}] the start given up on has failed: {e}"),
    }
}

/// One-line summary for the log: packets, retransmits (totals and last-minute rates), drift.
pub fn stats_line(profile: LatencyProfile, st: &SessionStats) -> String {
    format!(
        "stats: profile {}, packets {}, timing requests {}, retransmit requests {}, resent {}, \
         missed {}; last {} s: {:.1} requests/min, {:.1} missed/min; buffer {}, drift {:.1} ppm",
        profile.name(),
        st.packets_sent,
        st.timing_requests,
        st.retransmit_requests,
        st.retransmit_served,
        st.retransmit_missing,
        st.loss.window.as_secs(),
        st.loss.resend_requests_per_min,
        st.loss.missed_per_min,
        st.buffer_fill,
        st.drift_ppm,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_volume_rejects_non_finite_values_and_clamps_percentages() {
        assert_eq!(finite_volume(f32::NAN), None);
        assert_eq!(finite_volume(f32::INFINITY), None);
        assert_eq!(finite_volume(f32::NEG_INFINITY), None);
        assert_eq!(finite_volume(42.0), Some(42.0));
        assert_eq!(finite_volume(-5.0), Some(0.0));
        assert_eq!(finite_volume(250.0), Some(100.0));
    }

    #[test]
    fn only_setup_400_and_500_errors_suggest_a_firewall_problem() {
        for (method, status, expected) in [
            ("SETUP", 400, true),
            ("SETUP", 500, true),
            ("SETUP", 403, false),
            ("RECORD", 400, false),
        ] {
            let error = Ap2Error::Status {
                method: method.into(),
                uri: "/stream".into(),
                status,
                reason: "receiver refused".into(),
            };
            assert_eq!(firewall_like(&error), expected, "{method} {status}");
        }
        assert!(!firewall_like(&Ap2Error::Closed));
    }

    #[test]
    fn a_stop_after_queued_volume_changes_is_requested() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(SessionCmd::SetVolume(20.0)).unwrap();
        tx.send(SessionCmd::SetVolume(35.0)).unwrap();
        tx.send(SessionCmd::Stop).unwrap();
        assert!(stop_requested(&mut rx));
    }

    #[test]
    fn a_dropped_command_sender_requests_a_stop() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(SessionCmd::SetVolume(20.0)).unwrap();
        drop(tx);
        assert!(stop_requested(&mut rx));
    }

    #[test]
    fn queued_volume_changes_do_not_request_a_stop() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        tx.send(SessionCmd::SetVolume(20.0)).unwrap();
        tx.send(SessionCmd::SetVolume(35.0)).unwrap();
        assert!(!stop_requested(&mut rx));
    }

    #[test]
    fn latency_profiles_keep_distinct_increasing_sync_latencies() {
        let low = LatencyProfile::Low.sync_latency_ms();
        let balanced = LatencyProfile::Balanced.sync_latency_ms();
        let stable = LatencyProfile::Stable.sync_latency_ms();
        assert!(low < balanced, "low {low} must precede balanced {balanced}");
        assert!(
            balanced < stable,
            "balanced {balanced} must precede stable {stable}"
        );
    }

    #[test]
    fn loss_window_rates_cover_the_last_minute() {
        let t0 = Instant::now();
        let mut w = LossWindow::default();
        assert_eq!(w.update(t0, 0, 0), LossStats::default());
        // 10 requests and 2 missed in the first 30 s: 20/min and 4/min.
        let s = w.update(t0 + Duration::from_secs(30), 10, 2);
        assert_eq!(s.window, Duration::from_secs(30));
        assert!((s.resend_requests_per_min - 20.0).abs() < 1e-9);
        assert!((s.missed_per_min - 4.0).abs() < 1e-9);
        // Then quiet, sampled every 5 s up to 150 s: the startup burst leaves the window.
        let mut last = s;
        for i in 7..=30 {
            last = w.update(t0 + Duration::from_secs(i * 5), 10, 2);
        }
        assert!(last.window >= LOSS_WINDOW && last.window < LOSS_WINDOW + Duration::from_secs(10));
        assert_eq!(last.resend_requests_per_min, 0.0);
        assert_eq!(last.missed_per_min, 0.0);
    }

    /// A request the receiver never answers.
    fn unanswered() -> impl Future<Output = Result<(), Ap2Error>> {
        std::future::pending()
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_answer_is_waited_for_and_no_answer_ends_it() {
        let (handle, mut cmds) = SessionHandle::new(1);
        let mut volume = None;
        let start = tokio::time::Instant::now();
        // An answer that takes as long as TCP resends on a lossy link still counts.
        let slow = async {
            tokio::time::sleep(Duration::from_secs(6)).await;
            Ok(())
        };
        let r = control(slow, &mut cmds, &mut volume).await;
        assert!(matches!(r, Control::Answered(())));
        let start2 = tokio::time::Instant::now();
        let r = control(unanswered(), &mut cmds, &mut volume).await;
        assert!(matches!(r, Control::Failed(ControlError::NoAnswer)));
        assert_eq!(start2 - start, Duration::from_secs(6));
        assert_eq!(tokio::time::Instant::now() - start2, CONTROL_TIMEOUT);
        drop(handle);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_does_not_wait_for_a_request_and_volume_is_kept_for_later() {
        let (handle, mut cmds) = SessionHandle::new(1);
        let mut volume = None;
        let start = tokio::time::Instant::now();
        let stop_later = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            handle.set_volume(40.0);
            handle.set_volume(55.0);
            handle.stop();
        };
        let (r, ()) = tokio::join!(control(unanswered(), &mut cmds, &mut volume), stop_later);
        assert!(matches!(r, Control::Stopped));
        assert_eq!(tokio::time::Instant::now() - start, Duration::from_secs(1));
        assert_eq!(volume, Some(55.0));
    }
}
