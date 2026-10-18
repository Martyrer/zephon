//! The peak level of the audio each session sends, for the meter in a room's volume rail.
//!
//! A session's capture source records the peak of every chunk it hands to Nyx, stamped with
//! the chunk's capture time ([`LevelTap::record`]). The window reads the loudest peak captured
//! in a span ([`LevelTap::peak`]); it asks for the span one profile lag ago, when that audio
//! plays in the room. The meter shows what Zephon sends at the room's volume, not what the
//! receiver plays: a packet lost on Wi-Fi still shows.
//!
//! Recording is a few atomic stores into a fixed ring: no lock and no allocation on the
//! audio path, and the reader never blocks the writer.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

/// Chunks kept. Capture chunks are 10 ms or shorter, and the window reads at most the longest
/// lag (257 ms) plus one frame interval back, so 256 hold over 2.5 s of 10 ms chunks and
/// 0.75 s of 3 ms ones.
const SLOTS: usize = 256;

/// The latest chunk peaks of one session, by capture time.
pub struct LevelTap {
    /// Capture times are kept as milliseconds since this instant.
    origin: Instant,
    /// Each slot packs a chunk's capture time (low 32 bits, ms since `origin`) and its peak
    /// (high 32 bits, `f32` bits), so a reader never sees the time of one chunk with the peak
    /// of another. Zero is an empty slot.
    slots: [AtomicU64; SLOTS],
    next: AtomicUsize,
}

impl std::fmt::Debug for LevelTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevelTap").finish_non_exhaustive()
    }
}

impl Default for LevelTap {
    fn default() -> Self {
        Self::new()
    }
}

impl LevelTap {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            slots: std::array::from_fn(|_| AtomicU64::new(0)),
            next: AtomicUsize::new(0),
        }
    }

    /// Records the peak (largest absolute sample) of a chunk captured at `at`.
    pub fn record(&self, at: Instant, peak: f32) {
        let i = self.next.fetch_add(1, Relaxed) % SLOTS;
        let packed = (u64::from(peak.to_bits()) << 32) | u64::from(self.ms(at));
        self.slots[i].store(packed, Relaxed);
    }

    /// The loudest peak recorded for audio captured after `from` and up to `to`; 0 when there
    /// is none (silence, not streaming yet, or a span older than the ring holds).
    pub fn peak(&self, from: Instant, to: Instant) -> f32 {
        let (from, to) = (self.ms(from), self.ms(to));
        self.slots
            .iter()
            .map(|s| s.load(Relaxed))
            .filter(|&packed| packed != 0 && in_span(packed as u32, from, to))
            .map(|packed| f32::from_bits((packed >> 32) as u32))
            .fold(0.0, f32::max)
    }

    /// `at` in ms since `origin`, wrapping after about 49 days ([`in_span`] allows for it). A
    /// time before `origin` counts as `origin`.
    fn ms(&self, at: Instant) -> u32 {
        at.saturating_duration_since(self.origin).as_millis() as u32
    }
}

/// `t` is after `from` and at most `to`, on a clock that wraps: measured back from `to`, so
/// the span stays right across the wrap (the ring only holds the last few seconds).
fn in_span(t: u32, from: u32, to: u32) -> bool {
    let age = to.wrapping_sub(t);
    age < to.wrapping_sub(from)
}

/// The peak of a chunk as the receiver gets it: the largest absolute sample of Nyx's stereo
/// mix of it, ignoring NaN. Mono and stereo go out as they are; more channels are folded down
/// as Nyx does (`pipeline::downmix_to_stereo`, which drops the LFE), in `scratch`, which keeps
/// its allocation from chunk to chunk.
pub fn sent_peak(samples: &[f32], channels: u16, scratch: &mut Vec<f32>) -> f32 {
    let peak = |s: &[f32]| s.iter().fold(0.0, |m: f32, s| m.max(s.abs()));
    if channels <= 2 {
        return peak(samples);
    }
    pipeline::downmix_to_stereo(samples, channels, scratch);
    peak(scratch)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_peak_is_the_loudest_chunk_captured_in_the_span() {
        let tap = LevelTap::new();
        let t0 = tap.origin;
        for (i, peak) in [0.1, 0.9, 0.3, 0.5, 0.2].into_iter().enumerate() {
            tap.record(t0 + 10 * MS * (i as u32 + 1), peak);
        }
        // Chunks at 10, 20, 30, 40, 50 ms. The span is after `from`, up to `to`.
        assert_eq!(tap.peak(t0 + 25 * MS, t0 + 45 * MS), 0.5);
        assert_eq!(tap.peak(t0 + 10 * MS, t0 + 20 * MS), 0.9);
        assert_eq!(tap.peak(t0 + 20 * MS, t0 + 30 * MS), 0.3);
        // Audio captured later than the span (not playing yet) does not show.
        assert_eq!(tap.peak(t0, t0 + 15 * MS), 0.1);
        assert_eq!(tap.peak(t0 + 50 * MS, t0 + 90 * MS), 0.0);
    }

    #[test]
    fn chunks_pushed_out_of_the_ring_are_gone() {
        let tap = LevelTap::new();
        let t0 = tap.origin;
        tap.record(t0 + MS, 1.0);
        for i in 0..SLOTS as u32 {
            tap.record(t0 + 10 * MS * (i + 1), 0.25);
        }
        assert_eq!(tap.peak(t0, t0 + 10 * MS * SLOTS as u32), 0.25);
    }

    #[test]
    fn a_span_reads_right_across_the_clock_wrap() {
        let before = u32::MAX - 5;
        let after = 4; // 10 ms after `before`
        assert!(in_span(before, before - 1, after));
        assert!(in_span(after, before, after));
        assert!(!in_span(after, before, after - 1));
        assert!(!in_span(before, before, after));
    }

    #[test]
    fn the_chunk_peak_is_the_largest_absolute_sample_sent() {
        let mut scratch = Vec::new();
        assert_eq!(sent_peak(&[0.2, -0.7, 0.5, f32::NAN], 2, &mut scratch), 0.7);
        assert_eq!(sent_peak(&[], 2, &mut scratch), 0.0);
        // 5.1 (L, R, C, LFE, SL, SR): the LFE is not sent, the centre goes to both sides at
        // -3 dB.
        assert_eq!(
            sent_peak(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0], 6, &mut scratch),
            0.0
        );
        let centre = sent_peak(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 6, &mut scratch);
        assert!(
            (centre - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "{centre}"
        );
    }
}
