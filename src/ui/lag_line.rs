//! The lag line: a short timeline from "This PC" (0 ms) to 300 ms with one dot per latency
//! profile in use, labelled with its lag and the rooms on it. It replaces the alpha's
//! paragraph about rooms not being clock-synced: the gap between two dots is the offset
//! between those rooms.
//!
//! Plain divs: a 1 px rail, a 3 px gradient run up to the furthest dot, a tick for each
//! unused profile slot, and absolutely positioned dots and labels. A dot slides to its new
//! place when a room changes profile (a spring, so reduced motion makes it jump).

use std::time::Duration;

use gpui_kit::base::{Spring, spring};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SenderView;
use super::style::{Palette, TypeExt as _};
use crate::session::{LatencyProfile, Status};

/// The scale's right end.
const SCALE_MS: f32 = 300.;
/// Vertical layout at the default text size: the rail sits 13 px under the 24 px figure
/// line, the names start 15 px under the rail. Text-bound sizes grow with the text size.
const RAIL_GAP: f32 = 13.;
const FIGURE_LINE: f32 = 24.;
const NAMES_GAP: f32 = 15.;
const NAME_LINE: f32 = 15.;
/// Half the width of the box a lag figure is centred in.
const FIGURE_HALF: f32 = 36.;
/// Rough advance of a 12 px Segoe UI character, for label collision checks only.
const CHAR_W: f32 = 6.4;
/// About 200 ms to settle, without overshoot.
const SLIDE: Spring = Spring::new(Duration::from_millis(200));

struct Group {
    profile: LatencyProfile,
    /// Room names, with "losing packets" (shown in amber).
    rooms: Vec<(String, bool)>,
}

fn x_of(p: LatencyProfile) -> f32 {
    (p.lag_ms() as f32 / SCALE_MS).min(1.)
}

pub fn render(view: &SenderView, window: &mut Window, cx: &mut App) -> impl IntoElement {
    let p = Palette::of(cx);
    let fonts = &view.fonts;
    let s = fonts.scale;
    let streaming = view.state.streaming;
    let rail = RAIL_GAP + FIGURE_LINE * s;
    let names_top = rail + NAMES_GAP;
    let (name_line, char_w, figure_half) = (NAME_LINE * s, CHAR_W * s, FIGURE_HALF * s);

    // Rooms that play (or will play, when stopped), grouped by profile in lag order.
    let mut groups: Vec<Group> = LatencyProfile::ALL
        .iter()
        .map(|&profile| Group {
            profile,
            rooms: Vec::new(),
        })
        .collect();
    for r in &view.state.receivers {
        let active = r.selected
            && (!streaming
                || r.restarting
                || matches!(r.status, Status::Streaming | Status::Connecting));
        if !active {
            continue;
        }
        let losing = r.stats.is_some_and(|s| s.loss.missed_per_min > 0.0);
        if let Some(g) = groups.iter_mut().find(|g| g.profile == r.profile) {
            g.rooms.push((r.name.clone(), losing));
        }
    }
    let used: Vec<&Group> = groups.iter().filter(|g| !g.rooms.is_empty()).collect();

    // The track spans the footer's inner width (16 px padding and a 2 px inset each side).
    let track_w = (f32::from(window.viewport_size().width) - 36.).max(100.);

    // Name labels sit left-aligned under their dot; one that would run past the track end is
    // right-aligned to it instead, and one that would still collide moves down a line.
    let mut taken: Vec<(usize, f32, f32)> = vec![(0, 0., 50.)]; // "This PC"
    let mut placed = Vec::new();
    for g in &used {
        let x = x_of(g.profile) * track_w;
        let widest = g
            .rooms
            .iter()
            .map(|(n, _)| n.chars().count())
            .max()
            .unwrap_or(0) as f32
            * char_w;
        let lines = g.rooms.len();
        let end = x - 7. + widest > track_w;
        let (x0, x1) = if end {
            (track_w - widest, track_w)
        } else {
            (x - 7., x - 7. + widest)
        };
        let mut first = 0;
        while taken.iter().any(|&(line, a, b)| {
            line >= first && line < first + lines && x0 < b + 8. && a < x1 + 8.
        }) {
            first += 1;
        }
        for line in first..first + lines {
            taken.push((line, x0, x1));
        }
        placed.push((first, end));
    }
    let name_lines = taken.iter().map(|&(line, ..)| line + 1).max().unwrap_or(1);
    let height = (names_top + 2. * name_line).max(names_top + name_lines as f32 * name_line + 2.);

    let furthest = used.last().map(|g| x_of(g.profile)).unwrap_or(0.);
    let mut line = div().relative().h(px(height)).mx(px(2.));

    // Rail, and the run from the PC to the furthest room.
    line = line
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(rail))
                .h(px(1.))
                .bg(p.stroke_strong),
        )
        .when(furthest > 0., |this| {
            this.child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(rail - 1.))
                    .h(px(3.))
                    .w(relative(furthest))
                    .rounded(px(2.))
                    .when(streaming, |this| {
                        this.bg(linear_gradient(
                            90.,
                            linear_color_stop(p.stroke_strong, 0.),
                            linear_color_stop(p.lagoon, 1.),
                        ))
                    })
                    .when(!streaming, |this| this.bg(p.track).opacity(0.6)),
            )
        });

    // A faint tick at every profile slot without a dot (all of them when stopped).
    for g in &groups {
        if streaming && !g.rooms.is_empty() {
            continue;
        }
        line = line.child(
            div()
                .absolute()
                .top(px(rail - 4.))
                .left(relative(x_of(g.profile)))
                .w(px(1.))
                .h(px(9.))
                .bg(p.track),
        );
    }

    // "This PC".
    line = line
        .child(
            div()
                .absolute()
                .left_0()
                .top(px(rail - 9.))
                .w(px(2.))
                .h(px(19.))
                .rounded(px(1.))
                .bg(p.ink),
        )
        .child(
            div()
                .absolute()
                .left_0()
                .top(px(rail + 17.))
                .t_small(fonts)
                .text_color(p.ink_2)
                .whitespace_nowrap()
                .child("This PC"),
        );

    for (g, (first_line, end)) in used.iter().zip(placed) {
        // Keyed by the first room on the dot, so a room that moves alone slides with it.
        let key = format!("lag-dot-{}", g.rooms[0].0);
        let x = spring(key, x_of(g.profile), SLIDE, window, cx);
        let figure_end = x * track_w + figure_half > track_w;

        // Lag figure: 20 px Display, "ms" in 12 px Ink 2, centred over the dot.
        line = line.child(
            div()
                .absolute()
                .top_0()
                .left(relative(x))
                .w(px(figure_half * 2.))
                .when(!figure_end, |this| {
                    this.ml(px(-figure_half)).flex().justify_center()
                })
                .when(figure_end, |this| {
                    this.ml(px(10. - figure_half * 2.)).flex().justify_end()
                })
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(2.))
                        .child(
                            div()
                                .t_figure(fonts)
                                .text_color(if streaming { p.ink } else { p.ink_2 })
                                .child(g.profile.lag_ms().to_string()),
                        )
                        .child(div().t_small(fonts).text_color(p.ink_2).child("ms")),
                ),
        );

        // The dot: Lagoon with a thumb-colored ring while playing, hollow when stopped.
        line = line.child(
            div()
                .absolute()
                .top(px(rail - 6.))
                .left(relative(x))
                .ml(px(-7.))
                .size(px(15.))
                .rounded_full()
                .when(streaming, |this| {
                    this.bg(p.lagoon)
                        .border(px(3.))
                        .border_color(p.thumb)
                        .shadow(vec![BoxShadow {
                            color: p.stroke_strong,
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(1.),
                            inset: false,
                        }])
                })
                .when(!streaming, |this| {
                    this.bg(p.thumb).border_2().border_color(p.ink_3)
                }),
        );

        // Room names under the dot.
        let names = v_flex()
            .absolute()
            .top(px(names_top + first_line as f32 * name_line))
            .t_small(fonts)
            .line_height(px(name_line))
            .whitespace_nowrap()
            .when(end, |this| this.right_0().items_end())
            .when(!end, |this| this.left(relative(x)).ml(px(-7.)))
            .children(g.rooms.iter().map(|(name, losing)| {
                div()
                    .text_color(if *losing { p.amber } else { p.ink })
                    .child(name.clone())
            }));
        line = line.child(names);
    }

    v_flex().child(line).child(
        div()
            .mt(px(4.))
            .mb(px(14.))
            .t_small(fonts)
            .text_color(p.ink_2)
            .child(if streaming {
                "Each room plays this far behind the PC. Rooms aren't synced to each other."
            } else {
                "Each room will play this far behind the PC. Rooms aren't synced to each other."
            }),
    )
}
