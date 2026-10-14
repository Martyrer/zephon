//! Small building blocks: icons, the Windows 11 switch and volume slider, icon buttons.
//!
//! The switch and the slider are styled on gpui-base's unstyled primitives (which own the
//! interaction, focus and accessibility), because gpui-kit's styled versions cannot take the
//! Windows look: an outlined off track with an Ink 2 thumb, a 20 px slider thumb with a
//! Lagoon inner dot on a Track-colored rail.

use std::time::Duration;

use gpui_kit::base::{self, Spring, SwitchThumb, SwitchTrack, spring};
use gpui_kit::component::slider::SliderState;
use gpui_kit::component::{Icon, Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::style::Palette;

// Icons from the mockup (design/mockup.html). GPUI draws SVGs as a mask in the text color.
pub const GLYPH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="1" y="5" width="2.4" height="6" rx="1.2" fill="black"/><path d="M6.5 4.6a4.6 4.6 0 0 1 0 6.8M10 2.4a7.6 7.6 0 0 1 0 11.2" fill="none" stroke="black" stroke-width="1.5" stroke-linecap="round"/><circle cx="14" cy="8" r="1.3" fill="black"/></svg>"#;
/// Settings: an outlined eight-tooth gear, like the Fluent "Settings" icon.
pub const GEAR: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M12.90 5.97L13.12 6.63L14.90 6.84L14.90 9.16L13.12 9.37L12.90 10.03L12.59 10.65L13.70 12.06L12.06 13.70L10.65 12.59L10.03 12.90L9.37 13.12L9.16 14.90L6.84 14.90L6.63 13.12L5.97 12.90L5.35 12.59L3.94 13.70L2.30 12.06L3.41 10.65L3.10 10.03L2.88 9.37L1.10 9.16L1.10 6.84L2.88 6.63L3.10 5.97L3.41 5.35L2.30 3.94L3.94 2.30L5.35 3.41L5.97 3.10L6.63 2.88L6.84 1.10L9.16 1.10L9.37 2.88L10.03 3.10L10.65 3.41L12.06 2.30L13.70 3.94L12.59 5.35Z" fill="none" stroke="black" stroke-width="1.2" stroke-linejoin="round"/><circle cx="8" cy="8" r="2.3" fill="none" stroke="black" stroke-width="1.2"/></svg>"#;
pub const REFRESH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2v3.2h-3.2" fill="none" stroke="black" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;
pub const SPEAKER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2 6v4h2.6L8 13V3L4.6 6z" fill="none" stroke="black" stroke-width="1.2" stroke-linejoin="round"/><path d="M10.6 5.6a3.4 3.4 0 0 1 0 4.8M12.4 3.8a6 6 0 0 1 0 8.4" fill="none" stroke="black" stroke-width="1.2" stroke-linecap="round"/></svg>"#;
pub const PLAY: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 14"><path d="M3.5 2.2v9.6a.6.6 0 0 0 .9.5l7.6-4.8a.6.6 0 0 0 0-1L4.4 1.7a.6.6 0 0 0-.9.5z" fill="black"/></svg>"#;
pub const STOP: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 14 14"><rect x="2.5" y="2.5" width="9" height="9" rx="1.5" fill="black"/></svg>"#;
pub const WARN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M8 1.8L15 14H1z" fill="none" stroke="black" stroke-width="1.3" stroke-linejoin="round"/><path d="M8 6.2v3.6" stroke="black" stroke-width="1.4" stroke-linecap="round"/><circle cx="8" cy="11.8" r=".85" fill="black"/></svg>"#;

/// 8x8 fillets: a square minus the quarter circle of an 8 px rounded corner, with whether
/// each sits at the top and at the left.
pub const CORNERS: [(&[u8], bool, bool); 4] = [
    (br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><path d="M0 0H8A8 8 0 0 0 0 8Z" fill="black"/></svg>"#, true, true),
    (br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><path d="M0 0H8V8A8 8 0 0 0 0 0Z" fill="black"/></svg>"#, true, false),
    (br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><path d="M8 0V8H0A8 8 0 0 0 8 0Z" fill="black"/></svg>"#, false, false),
    (br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><path d="M0 0A8 8 0 0 0 8 8H0Z" fill="black"/></svg>"#, false, true),
];

pub fn icon(svg: &'static [u8], size: f32, color: Hsla) -> Icon {
    Icon::default()
        .data(svg)
        .with_size(px(size))
        .text_color(color)
}

/// A rounded group (rooms, a settings section) on the glass, 16 px in from each side. Every
/// pixel's tint is painted once (see `style`): the root tint on the side strips and in the
/// four corners outside the group's rounded edge; the group's rows paint their own.
pub fn group_on_glass(group: impl IntoElement, p: &Palette) -> impl IntoElement {
    let strip = || div().flex_none().w(px(16.)).bg(p.root);
    h_flex()
        .flex_none()
        .items_stretch()
        .child(strip())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .relative()
                .child(group)
                .children(CORNERS.iter().map(|&(svg, top, left)| {
                    div()
                        .absolute()
                        .when(top, |this| this.top_0())
                        .when(!top, |this| this.bottom_0())
                        .when(left, |this| this.left_0())
                        .when(!left, |this| this.right_0())
                        .child(icon(svg, 8., p.root))
                })),
        )
        .child(strip())
}

/// Thumb travel of the switch, for the slide animation.
const SWITCH_SPRING: Spring = Spring::new(Duration::from_millis(160));

/// Windows 11 toggle switch: 40x20, outlined when off with an Ink 2 thumb, filled Lagoon
/// when on. The thumb slides (instantly with reduced motion; the spring checks it).
pub fn switch(
    id: impl Into<ElementId>,
    on: bool,
    label: SharedString,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let id: ElementId = id.into();
    let p = Palette::of(cx);
    // Off: 12 px thumb 3 px from the left edge; on: 12 px thumb 3 px from the right
    // (inside a 1 px border, so 38 px of travel room).
    let x = spring(
        (id.clone(), "thumb"),
        if on { 22. } else { 3. },
        SWITCH_SPRING,
        window,
        cx,
    );
    base::Switch::new(id.clone())
        .checked(on)
        .accessibility_label(label)
        .on_change(move |next, _, window, cx| on_change(next, window, cx))
        .flex_none()
        .child(
            SwitchTrack::new((id, "track"))
                .checked(on)
                .relative()
                .w(px(40.))
                .h(px(20.))
                .rounded_full()
                .border_1()
                .border_color(if on { p.lagoon } else { p.track })
                .when(on, |this| this.bg(p.lagoon))
                .hover(|this| this.border_color(if on { p.lagoon } else { p.ink_2 }))
                .child(
                    SwitchThumb::new(on)
                        .absolute()
                        .top(px(3.))
                        .left(px(x))
                        .size(px(12.))
                        .rounded_full()
                        .bg(if on { p.on_lagoon } else { p.ink_2 }),
                ),
        )
}

/// Windows 11 slider: 4 px Track rail, Lagoon fill, 20 px thumb with a Lagoon inner dot.
pub fn volume_slider(state: &Entity<SliderState>, cx: &App) -> impl IntoElement {
    let p = Palette::of(cx);
    let pct = state.read(cx).percentage().end;
    base::Slider::new(state).flex_1().min_w(px(80.)).child(
        base::SliderTrack::new(state)
            .flex()
            .items_center()
            .h(px(20.))
            .w_full()
            .child(
                base::SliderIndicator::new(state)
                    .relative()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(p.track)
                    .child(
                        div()
                            .absolute()
                            .h_full()
                            .left_0()
                            .right(relative(1. - pct))
                            .rounded(px(2.))
                            .bg(p.lagoon),
                    )
                    .child(
                        base::SliderThumb::new(state)
                            .group("thumb")
                            .absolute()
                            .top(px(-8.))
                            .left(relative(pct))
                            .ml(px(-10.))
                            .size(px(20.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(p.thumb)
                            .border_1()
                            .border_color(p.stroke_strong)
                            .shadow(vec![BoxShadow {
                                color: hsla(0., 0., 0., 0.16),
                                offset: point(px(0.), px(1.)),
                                blur_radius: px(2.),
                                spread_radius: px(0.),
                                inset: false,
                            }])
                            .child(
                                div()
                                    .size(px(12.))
                                    .rounded_full()
                                    .bg(p.lagoon)
                                    .group_hover("thumb", |this| this.size(px(14.))),
                            ),
                    ),
            ),
    )
}
