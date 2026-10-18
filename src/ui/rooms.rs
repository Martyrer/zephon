//! The Rooms list (one row per AirPlay receiver) and the firewall banner.
//!
//! Rooms are listed by name (case-insensitive) and expand in place: a room that is on
//! shows its volume, latency profile and loss; a room that is off is a single line.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::controls::{self, icon};
use super::meter;
use super::style::{Palette, TypeExt as _, lead_text};
use super::{SenderView, profile_at, profile_title};
use crate::controller::{Command, RECONNECT_ATTEMPTS, ReceiverView, Reconnect};
use crate::firewall::FirewallState;
use crate::session::{LatencyProfile, Status};

/// Fade-in of a room's controls when it is switched on (skipped with reduced motion).
const EXPAND: Duration = Duration::from_millis(180);

/// Friendly names for the Apple model identifiers mDNS reports; others are shown as they are.
pub(super) fn model_name(model: &str) -> &str {
    match model {
        "AudioAccessory5,1" => "HomePod mini",
        "AudioAccessory1,1" | "AudioAccessory1,2" | "AudioAccessory6,1" => "HomePod",
        "AppleTV5,3" => "Apple TV HD",
        "AppleTV6,2" | "AppleTV11,1" | "AppleTV14,1" => "Apple TV 4K",
        other => other,
    }
}

/// Per-minute rates: whole numbers from 10 up, one decimal below.
fn rate(v: f64) -> String {
    // Decided on the one-decimal value, so 9.96 shows as "10" and 2.97 as "3", not "10.0"
    // and "3.0". Whole numbers come from `v` itself: rounding twice would make 11.46 "12".
    let r = (v * 10.0).round() / 10.0;
    if r >= 10.0 || r.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{r:.1}")
    }
}

/// The firewall banner: compact while the rule is just missing, full once a room has failed
/// with the timeout that points at it. Hidden when the rule is in place.
pub fn render_firewall(view: &SenderView, cx: &mut Context<SenderView>) -> Option<AnyElement> {
    let p = Palette::of(cx);
    let fonts = &view.fonts;
    let fw = &view.state.firewall;
    let installing = matches!(fw, FirewallState::Installing);
    if let FirewallState::Error(e) = fw {
        return Some(
            h_flex()
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(6.))
                .bg(p.amber_wash)
                .border_1()
                .border_color(p.stroke)
                .t_small(fonts)
                .child(icon(controls::WARN, 16., p.amber))
                .child(
                    div()
                        .flex_1()
                        .child(format!("Couldn't check the firewall: {e}")),
                )
                .into_any_element(),
        );
    }
    if !fw.needs_rule() && !installing {
        return None;
    }
    let install = |size_small: bool| {
        Button::new(if size_small {
            "install-fw-small"
        } else {
            "install-fw"
        })
        .label(if installing {
            "Waiting for permission…"
        } else {
            "Install firewall rule"
        })
        .loading(installing)
        .disabled(installing)
        .when(size_small, |b| b.small())
        .tooltip(format!("Adds an inbound rule for {}", view.state.exe_path))
        .on_click(cx.listener(|this, _, _, _| this.ctl.send(Command::InstallFirewall)))
    };
    let blocked = matches!(fw, FirewallState::Missing { blocked: true, .. });

    if view.state.firewall_suspected {
        let cause = if blocked {
            "A firewall rule blocks this app. Rooms send timing replies back to this PC, and \
             without them they drop the stream after 10 seconds."
        } else {
            "Rooms send timing replies back to this PC. Without a firewall rule for this app, \
             they drop the stream after 10 seconds."
        };
        return Some(
            h_flex()
                .items_start()
                .gap_3()
                .p(px(14.))
                .rounded(px(6.))
                .bg(p.amber_wash)
                .border_1()
                .border_color(p.stroke)
                .child(div().mt(px(2.)).child(icon(controls::WARN, 16., p.amber)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .mb(px(2.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Windows Firewall is blocking rooms"),
                        )
                        .child(div().text_color(p.ink_2).child(cause))
                        .child(
                            h_flex()
                                .mt_3()
                                .gap_3()
                                .flex_wrap()
                                .items_center()
                                .child(install(false))
                                .child(
                                    div()
                                        .t_small(fonts)
                                        .text_color(p.ink_2)
                                        .child("Windows asks for admin permission once."),
                                ),
                        ),
                )
                .into_any_element(),
        );
    }

    Some(
        h_flex()
            .gap(px(10.))
            .pl(px(12.))
            .pr(px(8.))
            .py(px(8.))
            .rounded(px(6.))
            .bg(p.amber_wash)
            .border_1()
            .border_color(p.stroke)
            .t_small(fonts)
            .child(icon(controls::WARN, 16., p.amber))
            .child(div().flex_1().child(if blocked {
                "Firewall rule blocks this app"
            } else {
                "Firewall rule missing"
            }))
            .child(install(true))
            .into_any_element(),
    )
}

/// The "Rooms" header with the re-browse button, and the grouped list. The group paints no
/// background of its own: each row paints its precomposited tint (see `style`), with the
/// group's rounded corners on the first and last row.
pub fn render_rooms(
    view: &SenderView,
    narrow: bool,
    window: &mut Window,
    cx: &mut Context<SenderView>,
) -> (AnyElement, AnyElement) {
    let p = Palette::of(cx);
    let fonts = &view.fonts;
    let st = &view.state;
    let header = h_flex()
        .justify_between()
        .items_center()
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Rooms"))
        .child(
            Button::new("refresh-rooms")
                .ghost()
                .small()
                .icon(icon(controls::REFRESH, 16., p.ink_2))
                .tooltip(if st.discovering {
                    "Looking for rooms…"
                } else {
                    "Look for rooms now"
                })
                .disabled(st.discovering)
                .on_click(cx.listener(|this, _, _, _| this.ctl.send(Command::RefreshReceivers))),
        );

    let list: AnyElement = if st.receivers.is_empty() {
        div()
            .px(px(14.))
            .py(px(12.))
            .rounded(px(7.))
            .bg(p.group_off)
            .t_small(fonts)
            .text_color(p.ink_2)
            .child(if st.discovering {
                "Looking for rooms…"
            } else {
                "No rooms found yet."
            })
            .into_any_element()
    } else {
        let last = st.receivers.len() - 1;
        let rows: Vec<AnyElement> = st
            .receivers
            .iter()
            .enumerate()
            .map(|(i, r)| render_room(view, i, i == last, r, narrow, window, cx))
            .collect();
        v_flex().children(rows).into_any_element()
    };

    let group = div()
        .rounded(px(8.))
        .border_1()
        .border_color(p.stroke)
        .child(list)
        .into_any_element();
    (header.into_any_element(), group)
}

/// How far below the top of its line box GPUI paints the baseline of `text` in `el`'s text
/// style. Worked out as `paint_line` (gpui `text_system/line.rs`) does it: the shaped line's
/// ascent plus descent centred in the device-pixel-snapped line height. The text itself is
/// shaped, not a sample or the font's metrics, because on Windows the line's ascent and descent
/// come from DirectWrite's line metrics, which grow when a CJK or emoji glyph pulls in a
/// fallback font. Snapped to a device pixel because glyphs are (`Window::paint_glyph`); without
/// it two baselines can still land a pixel apart.
fn baseline(mut el: impl Styled, text: SharedString, window: &Window) -> Pixels {
    let mut style = TextStyle::default();
    style.refine(&el.style().text);
    let size = style.font_size.to_pixels(window.rem_size());
    let line = window.pixel_snap(style.line_height.to_pixels(size.into(), window.rem_size()));
    let run = style.to_run(text.len());
    let shaped = window.text_system().shape_line(text, size, &[run], None);
    window.pixel_snap((line - shaped.ascent - shaped.descent) / 2. + shaped.ascent)
}

fn render_room(
    view: &SenderView,
    i: usize,
    last: bool,
    r: &ReceiverView,
    narrow: bool,
    window: &mut Window,
    cx: &mut Context<SenderView>,
) -> AnyElement {
    let p = Palette::of(cx);
    let fonts = &view.fonts;
    let streaming_now = r.status == Status::Streaming && !r.restarting;

    // Status word at the right of the name line.
    let status: Option<(String, Hsla, bool)> = if r.restarting {
        Some((
            format!("Switching to {}…", profile_title(r.profile).to_lowercase()),
            p.ink_2,
            true,
        ))
    } else if let Reconnect::Attempt(n) = r.reconnect {
        Some((
            format!("Reconnecting, attempt {n} of {RECONNECT_ATTEMPTS}…"),
            p.amber,
            true,
        ))
    } else {
        match &r.status {
            Status::Streaming => Some(("Playing".into(), p.lagoon, true)),
            Status::Connecting => Some(("Connecting…".into(), p.ink_2, true)),
            Status::Error(_) => None,
            Status::Idle if !r.online => Some(("Offline".into(), p.ink_3, false)),
            Status::Idle => None,
        }
    };
    let dimmed = !r.online && !streaming_now;
    // Wide, the name and the meta share a baseline. `items_baseline` cannot do it: GPUI's text
    // leaves report no baseline to taffy, which then uses each box's bottom edge, and the
    // 15/22 name and 12/16 meta paint their baselines at different heights in those boxes.
    // So both hang from the top and the meta drops by the difference. The meta side is
    // measured on the IP: it is always there and always in the UI font, and the model is too
    // unless it holds a glyph that needs a fallback font, which no Apple model name does.
    let ip = r.addr.ip().to_string();
    let meta_drop = baseline(div().t_name(fonts), r.name.clone().into(), window)
        - baseline(div().t_small(fonts), ip.clone().into(), window);

    let name_line = h_flex()
        .gap_3()
        .items_center()
        .min_h(px(22.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .when(narrow, |this| this.flex_col())
                .when(!narrow, |this| this.flex_row().items_start().gap(px(10.)))
                .child(
                    div()
                        .t_name(fonts)
                        .text_color(if dimmed { p.ink_3 } else { p.ink })
                        .truncate()
                        .child(r.name.clone()),
                )
                .child(
                    h_flex()
                        .gap(px(6.))
                        .t_small(fonts)
                        .whitespace_nowrap()
                        .when(!narrow, |this| this.mt(meta_drop))
                        .when_some(r.model.as_deref(), |this, m| {
                            this.child(div().text_color(p.ink_2).child(model_name(m).to_string()))
                        })
                        .child(div().text_color(p.ink_3).child(ip)),
                ),
        )
        .children(status.map(|(text, color, strong)| {
            div()
                .t_small(fonts)
                .whitespace_nowrap()
                .text_color(color)
                .when(strong, |this| this.font_weight(FontWeight::SEMIBOLD))
                .child(text)
        }))
        .child({
            let name = r.name.clone();
            let ctl = view.ctl.clone();
            // Keyed by name: rows can shift when a newly found room sorts above them.
            controls::switch(
                ElementId::Name(format!("room-on-{}", r.name).into()),
                r.selected,
                format!("Play in {}", r.name).into(),
                move |on, _, _| ctl.send(Command::SetSelected(name.clone(), on)),
                window,
                cx,
            )
        });

    let mut row = v_flex()
        .px(px(14.))
        .py(px(12.))
        .when(i > 0, |this| this.border_t_1().border_color(p.stroke))
        .when(i == 0, |this| this.rounded_t(px(7.)))
        .when(last, |this| this.rounded_b(px(7.)))
        .bg(if r.selected { p.group_on } else { p.group_off })
        .child(name_line);

    if r.selected
        && let Some(w) = view.rows.get(&r.name)
    {
        // Volume: shown in Ink 3 until the receiver (or the user) has reported it.
        let known = r.volume_known || w.last_user.is_some();
        // While the room plays, its rail carries the level meter.
        let metered = meter::shows_meter(r);
        let volume = h_flex()
            .flex_1()
            .min_w_0()
            .gap(px(10.))
            .items_center()
            .child(icon(controls::SPEAKER, 16., p.ink_2))
            .child(controls::volume_slider(
                &w.slider,
                metered.then(|| (&view.rails, r.name.clone().into())),
                cx,
            ))
            .child(
                div()
                    .w(fonts.px(26.))
                    .flex_none()
                    .text_right()
                    .text_color(if known { p.ink } else { p.ink_3 })
                    .child(format!("{:.0}", w.value)),
            );
        // Latency profile: one stop per profile, evenly spaced, the same width as the volume
        // slider so the two line up. The labels under the stops are buttons that pick a
        // profile too (Tab and Enter for the keyboard, a name for screen readers). Narrow, the
        // figure goes under the name so the three labels fit.
        let shown = profile_at(w.profile.read(cx).value().end());
        let last = LatencyProfile::ALL.len() - 1;
        let labels = h_flex().children(LatencyProfile::ALL.iter().enumerate().map(|(i, &lp)| {
            let state = w.profile.clone();
            let chosen = lp == shown;
            let name = div()
                .text_color(if chosen { p.ink } else { p.ink_2 })
                .when(chosen, |this| this.font_weight(FontWeight::SEMIBOLD))
                .child(profile_title(lp));
            let figure = div()
                .text_color(p.ink_2)
                .child(format!("{} ms", lp.lag_ms()));
            let text = div()
                .flex()
                .t_small(fonts)
                .whitespace_nowrap()
                .when(narrow, |this| {
                    this.flex_col()
                        .when(i > 0 && i < last, |this| this.items_center())
                        .when(i == last, |this| this.items_end())
                })
                .when(!narrow, |this| this.flex_row().gap(px(4.)))
                .child(name)
                .child(figure);
            div()
                .flex_1()
                .flex()
                .when(i > 0 && i < last, |this| this.justify_center())
                .when(i == last, |this| this.justify_end())
                .child(
                    Button::new(ElementId::Name(
                        format!("profile-{}-{}", r.name, lp.name()).into(),
                    ))
                    .ghost()
                    .compact()
                    .xsmall()
                    .h_auto()
                    .py(px(2.))
                    .px(px(4.))
                    // Text starts and ends under the rail's ends, not the button's.
                    .when(i == 0, |this| this.ml(px(-4.)))
                    .when(i == last, |this| this.mr(px(-4.)))
                    .accessibility_label(format!(
                        "Latency profile {}, {} ms",
                        profile_title(lp),
                        lp.lag_ms()
                    ))
                    .toggled(chosen)
                    .child(text)
                    // Setting the value picks the profile (see `SenderView::add_row`).
                    .on_click(move |_, window, cx| {
                        state.update(cx, |s, cx| s.set_value(i as f32, window, cx));
                    }),
                )
        }));
        let profile = h_flex()
            .gap(px(10.))
            .items_start()
            .child(div().h(px(20.)).flex().items_center().child(icon(
                controls::CLOCK,
                16.,
                p.ink_2,
            )))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(controls::profile_picker(&w.profile, cx))
                    .child(labels),
            )
            // The volume figure's slot, so both rails end at the same x.
            .child(div().w(fonts.px(26.)).flex_none());
        let controls = v_flex()
            .mt(px(10.))
            .gap(px(8.))
            .child(volume)
            .child(profile);
        let controls = controls.with_animation(
            ElementId::Name(format!("expand-{}", r.name).into()),
            Animation::new(EXPAND).with_easing(ease_out_quint()),
            |this, t| this.opacity(t),
        );
        row = row.child(controls);
    }

    // Loss over the last ~60 s, from the receiver's retransmit requests.
    if r.selected
        && streaming_now
        && let Some(s) = &r.stats
    {
        let l = &s.loss;
        row = row.child(
            h_flex()
                .mt(px(8.))
                .gap_3()
                .flex_wrap()
                .t_small(fonts)
                .text_color(p.ink_2)
                .child(format!("{} resends/min", rate(l.resend_requests_per_min)))
                .when(l.missed_per_min > 0.0, |this| {
                    this.child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.amber)
                            .child(format!("{} lost/min", rate(l.missed_per_min))),
                    )
                }),
        );
    }

    // The glitch hint: an offer with its cost, never applied automatically.
    if let Some(better) = r.suggestion.filter(|_| r.selected && !r.restarting) {
        let extra = better.lag_ms().saturating_sub(r.profile.lag_ms());
        let name = r.name.clone();
        row = row.child(
            h_flex()
                .mt(px(8.))
                .gap_3()
                .items_center()
                .pl(px(10.))
                .pr(px(6.))
                .py(px(6.))
                .rounded(px(4.))
                .bg(p.amber_wash)
                .t_small(fonts)
                .child(div().flex_1().min_w_0().child(lead_text(
                    "Glitchy?",
                    &format!(
                        "{} adds {extra} ms of lag and rides out Wi-Fi drops.",
                        profile_title(better)
                    ),
                    p.amber,
                )))
                .child(
                    Button::new(ElementId::Name(format!("switch-profile-{}", r.name).into()))
                        .small()
                        .label(format!(
                            "Switch to {}",
                            profile_title(better).to_lowercase()
                        ))
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.ctl.send(Command::SetProfile(name.clone(), better));
                        })),
                ),
        );
    }

    // Errors say what to do; the cause comes from the session.
    if let Status::Error(e) = &r.status
        && !r.restarting
    {
        // Stats arrive only while streaming, so an error without them is a failed start, unless
        // it ends the reconnect attempts after a lost connection.
        let head = if r.stats.is_some() || r.reconnect == Reconnect::GaveUp {
            "Stopped playing."
        } else {
            "Didn't connect."
        };
        row = row.child(
            div()
                .mt(px(8.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(4.))
                .bg(p.red_wash)
                .t_small(fonts)
                .child(lead_text(head, e, p.red)),
        );
    }

    row.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{model_name, rate};

    #[test]
    fn model_names() {
        assert_eq!(model_name("AudioAccessory5,1"), "HomePod mini");
        assert_eq!(model_name("AppleTV14,1"), "Apple TV 4K");
        assert_eq!(model_name("TV-55"), "TV-55");
    }

    #[test]
    fn rates() {
        assert_eq!(rate(0.0), "0");
        assert_eq!(rate(2.4), "2.4");
        assert_eq!(rate(7.0), "7");
        assert_eq!(rate(9.4), "9.4");
        assert_eq!(rate(9.96), "10");
        assert_eq!(rate(2.97), "3");
        assert_eq!(rate(11.46), "11");
        assert_eq!(rate(10.54), "11");
        assert_eq!(rate(10.25), "10");
        assert_eq!(rate(41.3), "41");
    }
}
