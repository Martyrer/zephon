//! The level meter in the volume rail of each room that plays (DESIGN.md, "Level meter").
//!
//! The meter must not slow the window down. Re-rendering the main window's content costs a
//! few ms of CPU, so a meter that did it 30 times a second would cost about a sixth of a core.
//! Instead the content ([`SenderView`](super::SenderView)) is drawn as a cached view, and the
//! meters are painted by a sibling view on top of it, [`MeterLayer`]. A meter frame
//! re-renders only that layer, and GPUI replays the rest of the window from the last frame.
//!
//! The content does not know the levels and the layer does not lay anything out: each volume
//! rail records where it is when the content is laid out ([`Rails`]), and the layer paints
//! there. Scrolling, resizing or moving a slider re-renders the content, so the record is
//! never stale when the layer paints.
//!
//! The layer runs a timer only while a room plays and the window can be seen, and asks for a
//! frame only when a bar moves by a device pixel, so silence and a still meter cost no frames.

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
        let fall = dt.as_secs_f32() / RELEASE.as_secs_f32();
        self.level = target.max(self.level - fall);
        if target >= self.peak {
            self.peak = target;
            self.peak_at = Some(now);
        } else if self
            .peak_at
            .is_none_or(|t| now.duration_since(t) >= PEAK_HOLD)
        {
            self.peak = self.level.max(self.peak - fall);
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
}

/// The volume rails of the rooms shown, by room name, as last laid out. Written while the
/// content is laid out, read when the meter layer paints (both on the UI thread).
#[derive(Clone, Default)]
pub struct Rails(Rc<RefCell<HashMap<String, Rail>>>);

impl Rails {
    /// Forgets every rail; call it when the content renders, as it then records each again.
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    /// An element to put in a volume rail (absolute, filling it) that records the rail's
    /// bounds when it is laid out. `volume` is the slider's fraction.
    pub fn marker(&self, name: &str, volume: f32) -> impl IntoElement + use<> {
        let rails = self.0.clone();
        let name = name.to_string();
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
    name: String,
    tap: Arc<LevelTap>,
    /// How long after capture the room plays the audio.
    lag: Duration,
    meter: Ballistics,
    /// The bar and peak line ends as last asked for, in device pixels.
    painted: (i32, i32),
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
                Metered {
                    lag: Duration::from_millis(r.profile.lag_ms().into()),
                    ..kept.unwrap_or_else(|| Metered {
                        name: r.name.clone(),
                        tap: tap.clone(),
                        lag: Duration::ZERO,
                        meter: Ballistics::default(),
                        painted: (0, 0),
                    })
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
            self.read_at = Instant::now();
            self.ticker = Some(cx.spawn_in(window, async move |this, cx| {
                loop {
                    let still = cx.update(|_, cx| cx.reduce_motion()).unwrap_or(false);
                    let wait = if still { STILL_FRAME } else { FRAME };
                    cx.background_executor().timer(wait).await;
                    let ticked =
                        this.update_in(cx, |this, window, cx| this.tick(still, window, cx));
                    if ticked.is_err() {
                        break;
                    }
                }
            }));
        }
    }

    /// Reads the levels that play in the rooms now, and asks for a frame if a bar moved.
    fn tick(&mut self, still: bool, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = now.duration_since(self.read_at);
        let scale = window.scale_factor();
        let mut moved = false;
        for m in &mut self.rooms {
            let (Some(from), Some(to)) = (self.read_at.checked_sub(m.lag), now.checked_sub(m.lag))
            else {
                continue;
            };
            m.meter
                .step(rail_fraction(m.tap.peak(from, to)), dt, now, still);
            let span = self
                .rails
                .get(&m.name)
                .map_or(0.0, |r| f32::from(r.span()) * scale);
            let ends = (
                (m.meter.level * span).round() as i32,
                (m.meter.peak * span).round() as i32,
            );
            moved |= ends != m.painted;
            m.painted = ends;
        }
        self.read_at = now;
        if moved {
            cx.notify();
        }
    }
}

impl Render for MeterLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::of(cx);
        let rails = self.rails.clone();
        let bars: Vec<(String, Ballistics)> = self
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
    use super::{Ballistics, FRAME, PEAK_HOLD, RELEASE, STILL_FRAME, rail_fraction};

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn levels_map_to_the_rail_in_db() {
        assert_eq!(rail_fraction(1.0), 1.0);
        assert_eq!(rail_fraction(2.0), 1.0); // over full scale: still the end of the rail
        assert!((rail_fraction(0.1) - 0.5).abs() < 1e-6); // -20 dB
        assert_eq!(rail_fraction(0.01), 0.0); // -40 dB
        assert_eq!(rail_fraction(0.0), 0.0);
        assert_eq!(rail_fraction(f32::NAN), 0.0);
    }

    #[test]
    fn the_bar_rises_at_once_and_falls_over_the_release() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.step(0.8, FRAME, t0, false);
        assert_eq!(b.level, 0.8);
        // Silence: down by the share of the release that has passed, never below the target.
        b.step(0.0, RELEASE / 4, t0 + RELEASE / 4, false);
        assert!((b.level - 0.55).abs() < 1e-6, "{}", b.level);
        b.step(0.3, RELEASE / 4, t0 + RELEASE / 2, false);
        assert!((b.level - 0.3).abs() < 1e-6, "{}", b.level);
        b.step(0.0, RELEASE, t0 + RELEASE * 3 / 2, false);
        assert_eq!(b.level, 0.0);
    }

    #[test]
    fn the_peak_line_holds_then_falls() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.step(0.9, FRAME, t0, false);
        b.step(0.0, RELEASE, t0 + RELEASE, false);
        assert_eq!((b.level, b.peak), (0.0, 0.9));
        // Still held just before the hold ends.
        b.step(0.0, PEAK_HOLD - RELEASE - MS, t0 + PEAK_HOLD - MS, false);
        assert_eq!(b.peak, 0.9);
        // Then it falls at the bar's pace.
        b.step(0.0, RELEASE / 2, t0 + PEAK_HOLD + RELEASE / 2, false);
        assert!((b.peak - 0.4).abs() < 1e-6, "{}", b.peak);
        // A louder level moves it up and holds again.
        b.step(0.5, FRAME, t0 + 2 * PEAK_HOLD, false);
        assert_eq!(b.peak, 0.5);
    }

    #[test]
    fn reduced_motion_shows_the_level_as_it_is_without_a_peak_line() {
        let t0 = Instant::now();
        let mut b = Ballistics::default();
        b.step(0.9, FRAME, t0, false);
        b.step(0.2, STILL_FRAME, t0 + STILL_FRAME, true);
        assert_eq!((b.level, b.peak), (0.2, 0.0));
    }
}
