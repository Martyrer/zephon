//! Capture endpoint selection and the shared loopback capture.
//!
//! One WASAPI loopback source (Nyx `capture`) is read by a pump thread and shared into a
//! queue per session (fan-out). Each queue is an [`AudioSource`], so every session keeps its
//! own Nyx pipeline (resampling, drift compensation, silence on underrun). Silence fill on
//! the endpoint itself (keep-alive render stream, rebuild silence) stays inside Nyx.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ::capture::wasapi::{
    ComMtaGuard, DeviceInfo, DeviceSpec, WasapiLoopbackConfig, WasapiLoopbackSource,
    list_render_endpoints,
};
use ::capture::{AudioChunk, AudioSource};
use anyhow::{Context, anyhow};
use tracing::{info, warn};

use crate::settings::Settings;

/// Lists active render endpoints (the possible capture endpoints).
pub fn list_endpoints() -> anyhow::Result<Vec<DeviceInfo>> {
    let _com = ComMtaGuard::new().map_err(|e| anyhow!("COM init: {e}"))?;
    list_render_endpoints().map_err(|e| anyhow!("listing render endpoints: {e}"))
}

/// The endpoint to use: the saved one (by ID, then by name), else the Windows default.
/// `Err` when a saved endpoint is missing (never falls back silently: the default can be a
/// silent device, such as a virtual mixer input).
pub fn choose_endpoint(
    endpoints: &[DeviceInfo],
    settings: &Settings,
) -> Result<DeviceInfo, String> {
    if settings.capture_device_id.is_some() || settings.capture_device_name.is_some() {
        if let Some(id) = &settings.capture_device_id
            && let Some(d) = endpoints.iter().find(|d| &d.id == id)
        {
            return Ok(d.clone());
        }
        if let Some(name) = &settings.capture_device_name
            && let Some(d) = find_by_name(endpoints, name)
        {
            return Ok(d.clone());
        }
        return Err(format!(
            "Saved capture endpoint \"{}\" is not present. Pick another capture endpoint.",
            settings.capture_device_name.as_deref().unwrap_or("?")
        ));
    }
    endpoints
        .iter()
        .find(|d| d.is_default)
        .cloned()
        .ok_or_else(|| "No active playback device found.".to_string())
}

/// Case-insensitive exact match, then substring match on the friendly name.
pub fn find_by_name<'a>(endpoints: &'a [DeviceInfo], name: &str) -> Option<&'a DeviceInfo> {
    let lower = name.to_lowercase();
    endpoints
        .iter()
        .find(|d| d.friendly_name.to_lowercase() == lower)
        .or_else(|| {
            endpoints
                .iter()
                .find(|d| d.friendly_name.to_lowercase().contains(&lower))
        })
}

/// Chunks kept per session queue before the oldest are dropped (about 1 s of 10 ms chunks).
/// Sessions drain every few ms, so this only matters for a session that has hung.
const MAX_QUEUED_CHUNKS: usize = 100;

/// Chunks are shared between the queues, not copied per session: the last session to read a
/// chunk takes it without a copy (with one room playing, chunks are usually not copied at all).
#[derive(Default)]
struct SessionQueue {
    chunks: Mutex<VecDeque<Arc<AudioChunk>>>,
    /// The hub adds chunks only from the moment the session streams. Nyx's feeder reads from
    /// the start, but its ring (about 0.25 s) fills while the receiver is still being set up;
    /// the feeder then waits, so a slow start (over about 1.25 s) would overflow this queue.
    /// The dropped chunks leave a gap that Nyx's drift estimate (frames counted against the
    /// capture time since its first chunk) reads as a clock far too slow, which pins its
    /// correction at the -500 ppm limit for many minutes.
    feeding: AtomicBool,
    closed: AtomicBool,
}

impl SessionQueue {
    /// Adds a captured chunk if the session takes audio yet. `false` once the queue is
    /// closed: the hub then forgets it.
    fn offer(&self, chunk: &Arc<AudioChunk>) -> bool {
        if self.closed.load(Ordering::Relaxed) {
            return false;
        }
        if self.feeding.load(Ordering::Relaxed)
            && let Ok(mut c) = self.chunks.lock()
        {
            if c.len() >= MAX_QUEUED_CHUNKS {
                c.pop_front();
            }
            c.push_back(chunk.clone());
        }
        true
    }
}

/// Lets the owner of a session start the hub feeding its queue once it streams, and stop it
/// (e.g. once a start was given up on: Nyx's feeder thread keeps reading until that start has
/// finished).
#[derive(Clone)]
pub struct QueueControl(Arc<SessionQueue>);

impl QueueControl {
    /// Audio captured from now on goes to the session.
    pub fn start_feeding(&self) {
        self.0.feeding.store(true, Ordering::Relaxed);
    }

    pub fn close(&self) {
        self.0.closed.store(true, Ordering::Relaxed);
        if let Ok(mut q) = self.0.chunks.lock() {
            q.clear();
        }
    }
}

/// One session's view of the shared capture.
pub struct FanoutSource {
    queue: Arc<SessionQueue>,
    sample_rate: u32,
    channels: u16,
}

impl AudioSource for FanoutSource {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    /// Returns the next captured chunk whole (about 10 ms; the Nyx pipeline accepts any
    /// size). Timestamps are the capture's own, so drift estimation is unchanged.
    fn read_chunk(
        &mut self,
        _max_frames: usize,
    ) -> Result<Option<AudioChunk>, Box<dyn std::error::Error + Send + Sync>> {
        if self.queue.closed.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let chunk = self
            .queue
            .chunks
            .lock()
            .ok()
            .and_then(|mut q| q.pop_front());
        Ok(chunk.map(Arc::unwrap_or_clone))
    }

    fn is_live(&self) -> bool {
        true
    }

    /// Called by the Nyx feeder when its session stops: detach from the hub only.
    fn stop(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.queue.closed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

struct HubShared {
    queues: Mutex<Vec<Weak<SessionQueue>>>,
}

/// The shared loopback capture of one endpoint. Dropping it stops the capture.
pub struct CaptureHub {
    shared: Arc<HubShared>,
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
    sample_rate: u32,
    channels: u16,
    endpoint_name: String,
}

impl CaptureHub {
    /// Opens loopback capture of the endpoint `id`, falling back to a name match. Never
    /// opens the Windows default implicitly.
    pub fn open(id: &str, name: &str) -> anyhow::Result<Self> {
        let open = |spec: DeviceSpec| {
            WasapiLoopbackSource::with_config(WasapiLoopbackConfig {
                device_spec: spec,
                ..Default::default() // keeps the silence keep-alive render stream on
            })
        };
        let source = match open(DeviceSpec::ById(id.to_string())) {
            Ok(s) => s,
            Err(e) => {
                warn!("Opening capture endpoint by ID failed ({e}); trying name \"{name}\"");
                open(DeviceSpec::ByName(name.to_string()))
                    .map_err(|e| anyhow!("{e}"))
                    .with_context(|| format!("cannot open capture endpoint \"{name}\""))?
            }
        };
        let endpoint_name = source.device_info().friendly_name.clone();
        let sample_rate = source.sample_rate();
        let channels = source.channels();
        info!("Capturing \"{endpoint_name}\" ({sample_rate} Hz, {channels} ch, loopback)");

        let shared = Arc::new(HubShared {
            queues: Mutex::new(Vec::new()),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let pump = {
            let shared = shared.clone();
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("capture-fanout".into())
                .spawn(move || pump(source, shared, stop))?
        };
        Ok(Self {
            shared,
            stop,
            pump: Some(pump),
            sample_rate,
            channels,
            endpoint_name,
        })
    }

    pub fn endpoint_name(&self) -> &str {
        &self.endpoint_name
    }

    /// A new source for one session. It receives audio captured after
    /// [`QueueControl::start_feeding`].
    pub fn subscribe(&self) -> (FanoutSource, QueueControl) {
        let queue = Arc::new(SessionQueue::default());
        if let Ok(mut qs) = self.shared.queues.lock() {
            qs.push(Arc::downgrade(&queue));
        }
        (
            FanoutSource {
                queue: queue.clone(),
                sample_rate: self.sample_rate,
                channels: self.channels,
            },
            QueueControl(queue),
        )
    }
}

impl Drop for CaptureHub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.pump.take() {
            let _ = t.join();
        }
    }
}

/// Lets the pump keep Nyx's rebuild silence to real time.
///
/// While the endpoint is being rebuilt (for example after it was unplugged), Nyx's
/// `read_chunk` returns a chunk of synthetic silence at once on every call, so an unpaced pump
/// would spin at audio priority, allocating and locking. Real captured audio is never held
/// back: it arrives at real-time speed by itself, and a backlog of it must drain at once.
#[derive(Debug, Default)]
struct SilencePacer {
    /// When the synthetic silence handed out so far has played out.
    until: Option<Instant>,
}

impl SilencePacer {
    /// How far the silence clock may fall behind (a late wake-up) and still be caught up
    /// with; beyond this, silence restarts from now instead of bursting.
    const CATCH_UP: Duration = Duration::from_millis(50);

    /// How long to wait after handing out a chunk of `duration`. `synthetic` = it was read
    /// while the endpoint was being rebuilt.
    fn after_chunk(&mut self, now: Instant, duration: Duration, synthetic: bool) -> Duration {
        if !synthetic {
            self.until = None;
            return Duration::ZERO;
        }
        let start = match self.until {
            Some(t) if t + Self::CATCH_UP >= now => t,
            _ => now,
        };
        let until = start + duration;
        self.until = Some(until);
        until.saturating_duration_since(now)
    }
}

fn pump(mut source: WasapiLoopbackSource, shared: Arc<HubShared>, stop: Arc<AtomicBool>) {
    let _priority = ::capture::enter_audio_thread_priority();
    let _ = source.start();
    let read_frames = (source.sample_rate() / 100).max(128) as usize;
    let mut pacer = SilencePacer::default();
    while !stop.load(Ordering::Relaxed) {
        match source.read_chunk(read_frames) {
            Ok(Some(chunk)) => {
                let duration = chunk.duration();
                let chunk = Arc::new(chunk);
                let Ok(mut queues) = shared.queues.lock() else {
                    break;
                };
                queues.retain(|w| w.upgrade().is_some_and(|q| q.offer(&chunk)));
                drop(queues);
                // Checked after the read, so at the edges of a rebuild one chunk may be paced
                // wrongly: at most one short extra wait, or one unpaced chunk.
                let wait = pacer.after_chunk(Instant::now(), duration, source.is_rebuilding());
                if !wait.is_zero() {
                    std::thread::sleep(wait);
                }
                continue; // drain everything that is ready before sleeping
            }
            Ok(None) => {}
            Err(e) => {
                warn!("Capture read failed: {e}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let _ = source.stop();
    info!("Capture stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints() -> Vec<DeviceInfo> {
        [
            ("usb", "Speakers (USB)", false),
            ("speakers", "Speakers", false),
            ("default", "Virtual mixer", true),
        ]
        .into_iter()
        .map(|(id, name, is_default)| DeviceInfo {
            id: id.into(),
            friendly_name: name.into(),
            is_default,
        })
        .collect()
    }

    fn saved_endpoint(id: Option<&str>, name: Option<&str>) -> Settings {
        let mut settings = Settings::default();
        settings.capture_device_id = id.map(str::to_string);
        settings.capture_device_name = name.map(str::to_string);
        settings
    }

    #[test]
    fn the_saved_endpoint_id_wins_over_its_name_and_the_default() {
        let chosen =
            choose_endpoint(&endpoints(), &saved_endpoint(Some("usb"), Some("Speakers"))).unwrap();
        assert_eq!(chosen.id, "usb");
    }

    #[test]
    fn a_missing_saved_id_falls_back_to_the_saved_name() {
        let chosen = choose_endpoint(
            &endpoints(),
            &saved_endpoint(Some("gone"), Some("speakers")),
        )
        .unwrap();
        assert_eq!(chosen.id, "speakers");
    }

    #[test]
    fn a_missing_saved_endpoint_never_falls_back_to_the_default() {
        let error = choose_endpoint(
            &endpoints(),
            &saved_endpoint(Some("gone"), Some("Missing speakers")),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "Saved capture endpoint \"Missing speakers\" is not present. Pick another capture endpoint."
        );
    }

    #[test]
    fn no_saved_endpoint_uses_the_windows_default() {
        assert_eq!(
            choose_endpoint(&endpoints(), &Settings::default())
                .unwrap()
                .id,
            "default"
        );
    }

    #[test]
    fn no_saved_endpoint_and_no_windows_default_is_an_error() {
        let mut endpoints = endpoints();
        endpoints[2].is_default = false;
        assert_eq!(
            choose_endpoint(&endpoints, &Settings::default()).unwrap_err(),
            "No active playback device found."
        );
    }

    #[test]
    fn endpoint_names_prefer_an_exact_case_insensitive_match() {
        assert_eq!(
            find_by_name(&endpoints(), "speakers").unwrap().id,
            "speakers"
        );
    }

    #[test]
    fn endpoint_names_fall_back_to_a_case_insensitive_substring() {
        assert_eq!(find_by_name(&endpoints(), "uSb").unwrap().id, "usb");
    }

    #[test]
    fn an_unknown_endpoint_name_has_no_match() {
        assert!(find_by_name(&endpoints(), "Headphones").is_none());
    }

    const CHUNK: Duration = Duration::from_millis(10);

    #[test]
    fn real_audio_is_never_held_back() {
        let mut p = SilencePacer::default();
        let t0 = Instant::now();
        // A backlog of real chunks read back to back: no waiting at all.
        for _ in 0..5 {
            assert_eq!(p.after_chunk(t0, CHUNK, false), Duration::ZERO);
        }
    }

    #[test]
    fn rebuild_silence_is_paced_to_real_time() {
        let mut p = SilencePacer::default();
        let t0 = Instant::now();
        assert_eq!(p.after_chunk(t0, CHUNK, true), CHUNK);
        // Woken on time: the next chunk again waits its own length.
        assert_eq!(p.after_chunk(t0 + CHUNK, CHUNK, true), CHUNK);
        // Woken 4 ms late: the next wait is shorter, so silence keeps real-time speed.
        let late = t0 + 2 * CHUNK + Duration::from_millis(4);
        assert_eq!(p.after_chunk(late, CHUNK, true), Duration::from_millis(6));
        // Called again at once (it would spin otherwise): waits for the silence already out.
        assert_eq!(p.after_chunk(late, CHUNK, true), Duration::from_millis(16));
    }

    #[test]
    fn silence_restarts_from_now_after_a_stall_or_real_audio() {
        let mut p = SilencePacer::default();
        let t0 = Instant::now();
        p.after_chunk(t0, CHUNK, true);
        // A long stall: no burst of catch-up silence.
        let later = t0 + Duration::from_secs(1);
        assert_eq!(p.after_chunk(later, CHUNK, true), CHUNK);
        // Real audio in between resets the clock.
        assert_eq!(p.after_chunk(later, CHUNK, false), Duration::ZERO);
        assert_eq!(p.after_chunk(later, CHUNK, true), CHUNK);
    }

    #[test]
    fn a_session_gets_only_audio_captured_once_it_streams() {
        let queue = Arc::new(SessionQueue::default());
        let control = QueueControl(queue.clone());
        let mut source = FanoutSource {
            queue: queue.clone(),
            sample_rate: 48_000,
            channels: 2,
        };
        let t0 = Instant::now();
        let chunk = |i: u32| Arc::new(AudioChunk::new(vec![0.0; 960], 48_000, 2, t0 + i * CHUNK));
        let read = |s: &mut FanoutSource| s.read_chunk(480).unwrap().map(|c| c.timestamp);
        // Captured while the receiver is still being set up: not kept for later.
        for i in 0..3 {
            assert!(queue.offer(&chunk(i)));
        }
        assert_eq!(read(&mut source), None);
        control.start_feeding();
        assert!(queue.offer(&chunk(3)));
        assert!(queue.offer(&chunk(4)));
        assert_eq!(read(&mut source), Some(t0 + 3 * CHUNK));
        assert_eq!(read(&mut source), Some(t0 + 4 * CHUNK));
        control.close();
        assert!(!queue.offer(&chunk(5)));
        assert_eq!(read(&mut source), None);
    }
}
