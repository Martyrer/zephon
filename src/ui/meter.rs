//! The level meter in the volume rail of each room that plays (DESIGN.md, "Level meter").
//!
//! The meter must not slow the window down. Re-rendering the main window's content costs a
//! few ms of CPU, so a meter that did it 30 times a second would cost about a sixth of a core.
//! Instead the content ([`SenderView`](super::SenderView)) is drawn as a cached view, and the
//! meters are painted by a sibling view on top of it, [`MeterLayer`]. A meter frame
//! re-renders that layer and the views above it (gpui-kit's root and its small overlay
//! views), and GPUI replays the content from the last frame.
//!
//! A reused cached view adds no accessibility nodes, so while an accessibility client (a
//! screen reader, for one) is attached the content is drawn whole each frame, and the meters
//! update 4 times a second, as with reduced motion, to keep that cost down.
//!
//! The slider states are not views, so their changes do not redraw the cached content by
//! themselves; `SenderView` observes each one.
//!
//! The content does not know the levels and the layer does not lay anything out: each volume
//! rail records where it is when the content is laid out ([`Rails`]), and the layer paints
//! there. Scrolling, resizing or moving a slider re-renders the content, so the record is
//! never stale when the layer paints.
//!
//! The layer runs a timer only while a room plays and the window is not minimized (Windows
//! does not tell a covered window apart from one in view), and asks for a frame only when a
//! bar moves by a device pixel, so silence, a still meter and a rail scrolled out of the list
//! cost no frames.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::*;

use super::Live;
use super::style::Palette;
use crate::controller::ReceiverView;
use crate::level::LevelTap;
use crate::session::Status;

/// Time between meter frames.
const FRAME: Duration = Duration::from_millis(33);
/// Time between meter updates with reduced motion: the bar jumps, without smoothing.
const STILL_FRAME: Duration = Duration::from_millis(250);
/// The quietest level shown: the bar starts at this many dB below full scale.
const FLOOR_DB: f32 = -40.0;
/// How long a falling bar takes to drop the whole rail. It rises at once.
const RELEASE: Duration = Duration::from_millis(450);
/// How long the peak line stays at the loudest recent level before it falls like the bar.
const PEAK_HOLD: Duration = Duration::from_secs(1);
/// Half the slider thumb: the bar stops at the thumb's edge, not under it.
const THUMB_RADIUS: Pixels = px(10.);

/// Where a peak amplitude sits on the rail, 0 (at or below [`FLOOR_DB`]) to 1 (full scale).
/// Steps in dB, as loudness is heard.
pub fn rail_fraction(peak: f32) -> f32 {
    // Also NaN: no level.
    if peak.is_nan() || peak <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * peak.log10();
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// One room's bar and peak line, as fractions of the rail from its start to the thumb.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Ballistics {
    pub level: f32,
    pub peak: f32,
    peak_at: Option<Instant>,
}

impl Ballistics {
    /// Moves to `target` (a [`rail_fraction`]) at `now`, `dt` after the last step: the bar
    /// rises at once and falls over [`RELEASE`]; the peak line holds for [`PEAK_HOLD`], then
    /// falls the same way. With `still` (reduced motion) the bar shows `target` as it is and
    /// there is no peak line.
    pub fn step(&mut self, target: f32, dt: Duration, now: Instant, still: bool) {
        if still {
            *self = Self {
                level: target,
                ..Self::default()
            };
            return;
        }
        let fall = |d: Duration| d.as_secs_f32() / RELEASE.as_secs_f32();
        self.level = target.max(self.level - fall(dt));
        if target >= self.peak {
            self.peak = target;
            self.peak_at = Some(now);
        } else {
            // Only the part of `dt` after the hold counts: a step that crosses the end of the
            // hold must not fall for the time the line was still held.
            let falling = self
                .peak_at
                .map_or(dt, |t| now.saturating_duration_since(t + PEAK_HOLD).min(dt));
            self.peak = self.level.max(self.peak - fall(falling));
        }
    }
}

/// Where one volume rail was laid out.
#[derive(Debug, Clone, Copy)]
struct Rail {
    bounds: Bounds<Pixels>,
    /// The volume, as a fraction of the rail.
    volume: f32,
    /// The part of the window the rail can be seen in (the scrolled rooms list).
    visible: Bounds<Pixels>,
}

impl Rail {
    /// The length the bar may fill: from the rail's start to the thumb's edge.
    fn span(&self) -> Pixels {
        (self.bounds.size.width * self.volume - THUMB_RADIUS).max(px(0.))
    }

    /// [`Self::span`] in device pixels, or 0 when the rail is scrolled out of sight, so a
    /// meter no one can see asks for no frames.
    fn seen_span(&self, scale: f32) -> f32 {
        let seen = self.bounds.intersect(&self.visible).size;
        if seen.width <= px(0.) || seen.height <= px(0.) {
            return 0.0;
        }
        f32::from(self.span()) * scale
    }
}

/// The volume rails of the rooms shown, by room name, as last laid out. Written while the
/// content is laid out, read when the meter layer paints (both on the UI thread).
#[derive(Clone, Default)]
pub struct Rails(Rc<RefCell<HashMap<SharedString, Rail>>>);

impl Rails {
    /// Forgets every rail; call it when the content renders, as it then records each again.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// An element to put in a volume rail (absolute, filling it) that records the rail's
    /// bounds when it is laid out. `volume` is the slider's fraction.
    pub fn marker(&self, name: SharedString, volume: f32) -> impl IntoElement + use<> {
        let rails = self.0.clone();
        canvas(
            move |bounds, window, _| {
                let visible = window.content_mask().bounds;
                rails.borrow_mut().insert(
                    name,
                    Rail {
                        bounds,
                        volume,
                        visible,
                    },
                );
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    fn get(&self, name: &str) -> Option<Rail> {
        self.0.borrow().get(name).copied()
    }
}

/// A room whose meter is shown.
struct Metered {
    name: SharedString,
    tap: Arc<LevelTap>,
    /// How long after capture the room plays the audio.
    lag: Duration,
    meter: Ballistics,
    /// The bar and peak line ends as last asked for, in device pixels.
    painted: (i32, i32),
}

impl Metered {
    fn new(name: SharedString, tap: Arc<LevelTap>, lag: Duration) -> Self {
        Self {
            name,
            tap,
            lag,
            meter: Ballistics::default(),
            painted: (0, 0),
        }
    }

    /// Steps the meter to the audio that plays in the room from `read_at` to `now`: the audio
    /// captured one lag earlier. `span` is the bar's room in device pixels (0 when it cannot
    /// be seen). Returns whether the bar or the peak line moved by a device pixel, which is
    /// when a frame is worth drawing.
    fn advance(&mut self, read_at: Instant, now: Instant, still: bool, span: f32) -> bool {
        let (Some(from), Some(to)) = (read_at.checked_sub(self.lag), now.checked_sub(self.lag))
        else {
            return false;
        };
        let target = rail_fraction(self.tap.peak(from, to));
        self.meter
            .step(target, now.saturating_duration_since(read_at), now, still);
        let ends = (
            (self.meter.level * span).round() as i32,
            (self.meter.peak * span).round() as i32,
        );
        let moved = ends != self.painted;
        self.painted = ends;
        moved
    }
}

/// Whether a room shows a meter: on, playing (not restarting), with a level tap.
pub fn shows_meter(r: &ReceiverView) -> bool {
    r.selected && !r.hidden && r.status == Status::Streaming && !r.restarting && r.level.is_some()
}

/// Paints the meters over the main window's content (see the module docs).
pub struct MeterLayer {
    live: Entity<Live>,
    rails: Rails,
    rooms: Vec<Metered>,
    /// When the levels were last read; the next read covers the audio since.
    read_at: Instant,
    ticker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MeterLayer {
    pub fn new(
        live: Entity<Live>,
        rails: Rails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe_in(&live, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe_window_visibility(window, |this, _, window, cx| this.sync(window, cx)),
        ];
        let mut layer = Self {
            live,
            rails,
            rooms: Vec::new(),
            read_at: Instant::now(),
            ticker: None,
            _subscriptions: subscriptions,
        };
        layer.sync(window, cx);
        layer
    }

    /// Follows the snapshot: which rooms show a meter, and whether the timer runs.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = &self.live.read(cx).state;
        let mut old = std::mem::take(&mut self.rooms);
        self.rooms = state
            .receivers
            .iter()
            .filter(|r| shows_meter(r))
            .filter_map(|r| r.level.as_ref().map(|tap| (r, tap)))
            .map(|(r, tap)| {
                // A room keeps its meter while its session does.
                let kept = old
                    .iter()
                    .position(|m| m.name == r.name && Arc::ptr_eq(&m.tap, tap))
                    .map(|i| old.swap_remove(i));
                let lag = Duration::from_millis(r.profile.lag_ms().into());
                Metered {
                    lag,
                    ..kept.unwrap_or_else(|| Metered::new(r.name.clone().into(), tap.clone(), lag))
                }
            })
            .collect();
        // A meter that is gone may still be painted.
        if old.iter().any(|m| m.painted != (0, 0)) {
            cx.notify();
        }
        let run = !self.rooms.is_empty() && window.is_visible();
        if !run {
            self.ticker = None;
        } else if self.ticker.is_none() {
            // What played while the window was hidden is gone: start every bar from rest, not
            // from where it was when the window was hidden.
            for m in &mut self.rooms {
                m.meter = Ballistics::default();
                m.painted = (0, 0);
            }
            cx.notify();
            self.read_at = Instant::now();
            self.ticker = Some(cx.spawn_in(window, async move |this, cx| {
                while let Ok(wait) = this.update_in(cx, |this, window, cx| this.tick(window, cx)) {
                    cx.background_executor().timer(wait).await;
                }
            }));
        }
    }

    /// Reads the levels that play in the rooms now, and asks for a frame if a bar moved.
    /// Returns the wait before the next tick.
    ///
    /// With reduced motion the bars jump 4 times a second. So they do while a screen reader or
    /// other accessibility client is attached: the content is then drawn without its cache
    /// (see `MainWindow`), so each meter frame costs a full render.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Duration {
        let still = cx.reduce_motion() || window.is_a11y_active();
        let now = Instant::now();
        let scale = window.scale_factor();
        let mut moved = false;
        for m in &mut self.rooms {
            let span = self.rails.get(&m.name).map_or(0.0, |r| r.seen_span(scale));
            moved |= m.advance(self.read_at, now, still, span);
        }
        self.read_at = now;
        if moved {
            cx.notify();
        }
        if still { STILL_FRAME } else { FRAME }
    }
}

impl Render for MeterLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(cx);
        let rails = self.rails.clone();
        let bars: Vec<(SharedString, Ballistics)> = self
            .rooms
            .iter()
            .map(|m| (m.name.clone(), m.meter))
            .collect();
        // The rails are looked up when painting: when the content renders in the same frame,
        // it records them after this render, before any paint.
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                for (name, meter) in &bars {
                    if let Some(rail) = rails.get(name) {
                        paint_meter(rail, meter.level, meter.peak, &p, window);
                    }
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }
}

/// The bar from the rail's start, and the peak line ahead of it, clipped to where the rail
/// can be seen.
fn paint_meter(rail: Rail, level: f32, peak: f32, p: &Palette, window: &mut Window) {
    let span = rail.span();
    let origin = rail.bounds.origin;
    let height = rail.bounds.size.height;
    window.with_content_mask(
        Some(ContentMask {
            bounds: rail.visible,
        }),
        |window| {
            let bar = span * level;
            if bar >= px(0.5) {
                window.paint_quad(
                    fill(Bounds::new(origin, size(bar, height)), p.lagoon)
                        .corner_radii(height / 2.),
                );
            }
            if peak > level && span * peak >= px(2.) {
                let x = origin.x + span * peak - px(2.);
                window.paint_quad(fill(
                    Bounds::new(point(x, origin.y - px(1.)), size(px(2.), height + px(2.))),
                    p.ink.opacity(0.8),
                ));
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    // Not `super::*`: gpui's `test` attribute would replace the standard one.
    use std::sync::Arc;

    use gpui_kit::{Bounds, point, px, size};

    use super::{
        Ballistics, FRAME, LevelTap, Metered, PEAK_HOLD, RELEASE, Rail, STILL_FRAME, rail_fraction,
    };

    #[test]
    fn levels_map_to_the_rail_in_db() {
        assert_eq!(rail_fraction(1.0), 1.0);
        assert_eq!(rail_fraction(2.0), 1.0); // over full scale: still the end of the rail
        assert!((rail_fraction(0.1) - 0.5).abs() < 1e-6); // -20 dB
        assert_eq!(rail_fraction(0.01), 0.0); // -40 dB
        assert_eq!(rail_fraction(0.0), 0.0);
        assert_eq!(rail_fraction(f32::NAN), 0.0);
    }

    /// Steps `b` through `(ms after t0, target)` pairs, each `dt` from the step before, as
    /// the timer does. Returns `(level, peak)` after each.
    fn run(b: &mut Ballistics, t0: Instant, steps: &[(u64, f32)]) -> Vec<(f32, f32)> {
        let mut last = t0;
        steps
            .iter()
            .map(|&(ms, target)| {
                let now = t0 + Duration::from_millis(ms);
                b.step(target, now - last, now, false);
                last = now;
                (b.level, b.peak)
            })
            .collect()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_bar_rises_at_once_and_falls_over_the_release() {
        let release = RELEASE.as_millis() as u64;
        let mut b = Ballistics::default();
        let out = run(
            &mut b,
            Instant::now(),
            &[
                (33, 0.8),
                (33 + release / 5, 0.0),
                (33 + release * 2 / 5, 0.5),
                (33 + 2 * release, 0.0),
            ],
        );
        assert_eq!(out[0].0, 0.8);
        // Silence: down by the share of the release that has passed (a fifth), never below the
        // target.
        assert!(close(out[1].0, 0.6), "{out:?}");
        assert!(close(out[2].0, 0.5), "{out:?}");
        assert_eq!(out[3].0, 0.0);
    }

    #[test]
    fn the_peak_line_holds_then_falls_only_for_the_time_after_the_hold() {
        let hold = PEAK_HOLD.as_millis() as u64;
        let release = RELEASE.as_millis() as f32;
        let mut b = Ballistics::default();
        let out = run(
            &mut b,
            Instant::now(),
            &[
                (0, 0.9),
                (hold - 100, 0.0),
                (hold + 100, 0.0),
                (2 * hold, 0.5),
            ],
        );
        // Held while the bar falls away under it.
        assert_eq!(out[1], (0.0, 0.9));
        // The step crosses the end of the hold: only its last 100 ms count.
        assert!(close(out[2].1, 0.9 - 100.0 / release), "{out:?}");
        // A louder level moves it up and holds again.
        assert_eq!(out[3].1, 0.5);
    }

    #[test]
    fn reduced_motion_shows_the_level_as_it_is_without_a_peak_line() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.step(0.9, FRAME, t0, false);
        b.step(0.2, STILL_FRAME, t0 + STILL_FRAME, true);
        assert_eq!((b.level, b.peak), (0.2, 0.0));
    }

    const LAG: Duration = Duration::from_millis(165);
    const SPAN: f32 = 200.0;

    fn ms(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn a_room_shows_the_audio_it_plays_now_captured_one_lag_ago() {
        let tap = Arc::new(LevelTap::new());
        let t0 = Instant::now();
        tap.record(ms(t0, 100), 1.0);
        let mut m = Metered::new("Kitchen".into(), tap, LAG);
        // Captured at 100 ms, it plays at 265 ms: not yet at 250 ms.
        assert!(!m.advance(ms(t0, 200), ms(t0, 250), false, SPAN));
        assert_eq!(m.meter.level, 0.0);
        assert!(m.advance(ms(t0, 250), ms(t0, 283), false, SPAN));
        assert_eq!(m.meter.level, 1.0);
    }

    #[test]
    fn a_still_or_unseen_meter_asks_for_no_frames() {
        let tap = Arc::new(LevelTap::new());
        let t0 = Instant::now();
        let mut m = Metered::new("Kitchen".into(), tap.clone(), LAG);
        // Silence: nothing moves, tick after tick.
        for i in 1..10 {
            assert!(!m.advance(ms(t0, 33 * i), ms(t0, 33 * (i + 1)), false, SPAN));
        }
        // Loud audio on a rail scrolled out of sight: no frames either.
        for i in 10..20 {
            tap.record(ms(t0, 33 * i) - LAG, 1.0);
            assert!(!m.advance(ms(t0, 33 * i), ms(t0, 33 * (i + 1)), false, 0.0));
        }
    }

    #[test]
    fn a_rail_scrolled_out_of_the_list_has_no_room_for_a_bar() {
        let list = Bounds::new(point(px(0.), px(100.)), size(px(400.), px(300.)));
        let rail = |y: f32| Rail {
            bounds: Bounds::new(point(px(50.), px(y)), size(px(220.), px(4.))),
            volume: 0.5,
            visible: list,
        };
        // Half the 220 px rail, less the thumb's radius, at 1.5x.
        assert_eq!(rail(200.).seen_span(1.5), 150.0);
        assert_eq!(rail(40.).seen_span(1.5), 0.0);
        assert_eq!(rail(420.).seen_span(1.5), 0.0);
    }
}
