//! Design tokens (design/DESIGN.md): the two palettes, the gpui-kit theme built from them,
//! and the Segoe UI Variable type roles.
//!
//! The accent (Lagoon, or one derived from the Windows accent in `accent`) and the type
//! scale (text size, font family) are chosen in the settings window, so the palettes, the
//! theme JSON and [`Fonts`] are built at run time from these constants.
//!
//! Every surface is a translucent tint over the window material, so no color here is an
//! opaque panel except the select menu. gpui-kit components read the theme JSON; the app's
//! own elements (rooms, switch, sliders) read [`Palette`] directly, because some
//! tokens have no gpui-kit field (amber/red washes) and gpui-kit clamps `list_active` to 20 %.
//!
//! GPUI's renderer adds alpha when it blends (D3D11 `SrcBlendAlpha = DestBlendAlpha = ONE`),
//! so a translucent layer painted over the translucent root comes out more opaque than
//! "source over" and hides the material (light Layer 1 over the root turned grey). The large
//! surfaces are therefore painted once each with the precomposited color ([`Palette::root`],
//! [`Palette::group_off`], [`Palette::group_on`]); the theme `background` is transparent so
//! gpui-kit's root does not paint under them.
//!
//! Contrast (DESIGN.md, "Contrast") is checked against what the material can put behind the
//! window, not a typical color. Acrylic is GPUI's accent-policy blur with a clear tint, so it
//! shows whatever is behind, black to white, and only the root tint bounds it; Mica is
//! bounded by DWM's own tint ([`Tokens::mica_range`]). [`paint`] shows how a stack of tints
//! comes out, and the tests check every color on every surface it is drawn on.

use std::ops::RangeInclusive;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme, Theme, ThemeSet};
use gpui_kit::*;

use crate::settings::Backdrop;

/// One palette as `0xRRGGBBAA` values (CSS `rgba()` alpha rounded to /255).
#[derive(Clone, Copy)]
pub struct Tokens {
    pub ink: u32,
    pub ink_2: u32,
    pub ink_3: u32,
    pub lagoon: u32,
    pub lagoon_hover: u32,
    pub lagoon_active: u32,
    pub on_lagoon: u32,
    pub amber: u32,
    pub red: u32,
    /// Root tint over Mica, over acrylic, and the opaque fallback.
    pub root_mica: u32,
    pub root_acrylic: u32,
    pub root_solid: u32,
    pub layer_1: u32,
    pub layer_2: u32,
    pub control: u32,
    pub control_hover: u32,
    pub control_active: u32,
    pub stroke: u32,
    pub stroke_strong: u32,
    pub track: u32,
    pub thumb: u32,
    pub amber_wash: u32,
    pub red_wash: u32,
    /// Ink at about 4-6 %: list and caption button hover.
    pub hover: u32,
    pub pressed: u32,
    pub popover: u32,
    pub window_border: u32,
    pub overlay: u32,
    /// A typical Mica color, for flattening small fills painted over the root.
    pub material: u32,
    /// The darkest and the brightest grey Mica can be behind the root tint. DWM paints Mica
    /// as its tint (WinUI's values: `#202020` at 80 % dark, `#F3F3F3` at 50 % light) over the
    /// blurred wallpaper with the wallpaper's luminosity replaced by the tint's, so the
    /// wallpaper moves its hue, not its brightness: over every hue that stays as bright as
    /// greys `#1F`-`#22` (dark) and `#F2`-`#F6` (light). The range is wider, for DWM's noise
    /// and builds whose recipe differs.
    pub mica_range: [u8; 2],
}

pub const LIGHT: Tokens = Tokens {
    ink: 0x1B1E25FF,
    ink_2: 0x464B55FF,
    ink_3: 0x616774FF,
    lagoon: 0x00708AFF,
    lagoon_hover: 0x00627AFF,
    lagoon_active: 0x005466FF,
    on_lagoon: 0xFFFFFFFF,
    amber: 0x8C5200FF,
    red: 0xB4232CFF,
    root_mica: 0xEEF1F659, // 0.35: Mica is calm, so a light tint keeps its color
    // 0.90: acrylic can show a black window, and Ink 2 and red must stay readable over it.
    root_acrylic: 0xEEF1F6E6,
    root_solid: 0xEEF1F6FF,
    layer_1: 0xFFFFFF5C,        // 0.36
    layer_2: 0xFFFFFFA8,        // 0.66
    control: 0xFFFFFFB8,        // 0.72
    control_hover: 0xFFFFFFD9,  // 0.85
    control_active: 0xFFFFFF8C, // 0.55
    stroke: 0x161C2D16,         // 0.085
    stroke_strong: 0x161C2D2E,  // 0.18
    track: 0x161C2D7E,          // 0.49, for 3:1 (Windows' strong stroke is 0.45 black)
    thumb: 0xFFFFFFFF,
    amber_wash: 0x8F530016, // 0.085
    red_wash: 0xB4232C13,   // 0.075
    hover: 0x1B1E250F,
    pressed: 0x1B1E251A,
    popover: 0xF9FAFBFF,
    window_border: 0x00000024, // 0.14
    overlay: 0x00000033,       // 0.20
    material: 0xF3F3F3FF,
    mica_range: [0xE6, 0xFF],
};

pub const DARK: Tokens = Tokens {
    ink: 0xF1F3F6FF,
    ink_2: 0xD0D3DBFF,
    ink_3: 0xB3B7BEFF,
    lagoon: 0x5CC8D6FF,
    lagoon_hover: 0x79D4E0FF,
    lagoon_active: 0x4AB4C2FF,
    on_lagoon: 0x062A31FF,
    amber: 0xF0B455FF,
    red: 0xFF9999FF,
    root_mica: 0x1B1E2566, // 0.40
    // 0.92: acrylic can show a white window, and text on rooms that are on must stay
    // readable over it.
    root_acrylic: 0x1B1E25EB,
    root_solid: 0x1B1E25FF,
    layer_1: 0xFFFFFF0A,        // 0.040
    layer_2: 0xFFFFFF13,        // 0.075
    control: 0xFFFFFF11,        // 0.065
    control_hover: 0xFFFFFF1A,  // 0.10
    control_active: 0xFFFFFF0B, // 0.045
    stroke: 0xFFFFFF13,         // 0.075
    stroke_strong: 0xFFFFFF26,  // 0.15
    track: 0xFFFFFF78,          // 0.47, for 3:1 (Windows' strong stroke is 0.55 white)
    thumb: 0x454B56FF,
    amber_wash: 0xF0B4551A, // 0.10
    red_wash: 0xFF949417,   // 0.09
    hover: 0xFFFFFF0F,
    pressed: 0xFFFFFF0A,
    popover: 0x2A2E36FF,
    window_border: 0xFFFFFF1A, // 0.10
    overlay: 0x00000073,       // 0.45
    material: 0x202020FF,
    mica_range: [0x18, 0x2C],
};

/// An accent for one theme: the fill, its hover and pressed fills, and the text on it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccentSet {
    pub base: u32,
    pub hover: u32,
    pub active: u32,
    pub on: u32,
}

/// An accent for both themes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accent {
    pub light: AccentSet,
    pub dark: AccentSet,
}

impl Accent {
    pub fn for_theme(&self, dark: bool) -> &AccentSet {
        if dark { &self.dark } else { &self.light }
    }
}

/// The app's own accent, the cool teal of the design.
pub const LAGOON: Accent = Accent {
    light: AccentSet {
        base: LIGHT.lagoon,
        hover: LIGHT.lagoon_hover,
        active: LIGHT.lagoon_active,
        on: LIGHT.on_lagoon,
    },
    dark: AccentSet {
        base: DARK.lagoon,
        hover: DARK.lagoon_hover,
        active: DARK.lagoon_active,
        on: DARK.on_lagoon,
    },
};

/// The volume rail under a level meter, from the rail's start to the thumb: the accent at
/// 25 % over the text color on it, opaque. The meter's bar is the accent itself, so the bar
/// keeps about 4:1 against it in both themes (a dimmed accent over the grey track was about
/// 1.3:1), and the rail still shows a tinted channel against the room and the track.
pub fn meter_well(accent: &AccentSet) -> u32 {
    over((accent.base & !0xFF) | 0x40, accent.on)
}

/// The accent in use (the theme JSON and [`Palette`] are built from it).
pub struct ActiveAccent(pub Accent);

impl Global for ActiveAccent {}

impl Tokens {
    /// These tokens with `accent` in place of Lagoon.
    pub const fn with_accent(mut self, accent: &AccentSet) -> Self {
        self.lagoon = accent.base;
        self.lagoon_hover = accent.hover;
        self.lagoon_active = accent.active;
        self.on_lagoon = accent.on;
        self
    }

    /// The root tint over `material`.
    pub const fn root(&self, material: Backdrop) -> u32 {
        match material {
            Backdrop::Mica => self.root_mica,
            Backdrop::Acrylic => self.root_acrylic,
            Backdrop::Solid => self.root_solid,
        }
    }

    /// Layer 1 over the root, precomposited: the rooms group (rooms that are off), the
    /// footer, the settings groups.
    pub fn group_off(&self, material: Backdrop) -> u32 {
        over(self.layer_1, self.root(material))
    }

    /// Layer 2 over Layer 1 over the root, precomposited: rooms that are on.
    pub fn group_on(&self, material: Backdrop) -> u32 {
        over(self.layer_2, self.group_off(material))
    }

    /// The control fill over the root tint, flattened over a typical material color: opaque.
    pub fn control_on_root(&self, material: Backdrop) -> u32 {
        over(self.control, over(self.root(material), self.material))
    }

    /// The greys `material` can put behind the window (see the module docs). Acrylic is not
    /// bounded; the solid root tint hides the backdrop, so one grey stands for all.
    pub fn backdrops(&self, material: Backdrop) -> RangeInclusive<u8> {
        match material {
            Backdrop::Mica => self.mica_range[0]..=self.mica_range[1],
            Backdrop::Acrylic => 0..=0xFF,
            Backdrop::Solid => 0..=0,
        }
    }
}

/// The app's own colors for the active theme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub ink: Hsla,
    pub ink_2: Hsla,
    pub ink_3: Hsla,
    pub lagoon: Hsla,
    pub on_lagoon: Hsla,
    pub amber: Hsla,
    pub red: Hsla,
    pub control: Hsla,
    pub stroke: Hsla,
    pub stroke_strong: Hsla,
    pub track: Hsla,
    /// See [`meter_well`].
    pub meter_well: Hsla,
    pub thumb: Hsla,
    pub amber_wash: Hsla,
    pub red_wash: Hsla,
    /// The root tint for the material in use.
    pub root: Hsla,
    /// Layer 1 over the root: the rooms group (rooms that are off) and the footer.
    pub group_off: Hsla,
    /// Layer 2 over Layer 1 over the root: rooms that are on.
    pub group_on: Hsla,
    /// The control fill as it looks over the root tint, flattened over a typical material
    /// color. A translucent fill over the root would come out darker (see the module docs).
    pub control_on_root: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgba(hex).into()
}

/// One channel of a `0xRRGGBBAA` color (shift 24 red, 16 green, 8 blue, 0 alpha), 0 to 1.
fn channel(v: u32, shift: u32) -> f32 {
    ((v >> shift) & 0xFF) as f32 / 255.
}

/// CSS "source over" of two `0xRRGGBBAA` colors, as a straight-alpha color.
pub fn over(top: u32, bottom: u32) -> u32 {
    let (ta, ba) = (channel(top, 0), channel(bottom, 0));
    let a = ta + ba * (1. - ta);
    if a <= 0. {
        return 0;
    }
    let mix = |shift: u32| {
        let v = (channel(top, shift) * ta + channel(bottom, shift) * ba * (1. - ta)) / a;
        (v * 255.).round() as u32
    };
    (mix(24) << 24) | (mix(16) << 16) | (mix(8) << 8) | (a * 255.).round() as u32
}

/// The opaque color GPUI shows for `layers` (bottom first) painted over a grey `backdrop`.
/// Each layer blends its color "source over" but adds its alpha (see the module docs), and
/// DWM puts what is left of the window's transparency over the material.
pub fn paint(layers: &[u32], backdrop: u8) -> u32 {
    let (mut rgb, mut alpha) = ([0f32; 3], 0f32);
    for &layer in layers {
        let a = channel(layer, 0);
        for (c, shift) in rgb.iter_mut().zip([24, 16, 8]) {
            *c = channel(layer, shift) * a + *c * (1. - a);
        }
        alpha = (alpha + a).min(1.);
    }
    let b = f32::from(backdrop) / 255.;
    let [r, g, bl] = rgb.map(|c| (((c + b * (1. - alpha)) * 255.).round() as u32).min(0xFF));
    (r << 24) | (g << 16) | (bl << 8) | 0xFF
}

/// The window material in use (after fallbacks); the root tint depends on it.
pub struct Material(pub Backdrop);

impl Global for Material {}

impl Palette {
    pub fn of(cx: &App) -> Self {
        let dark = cx.theme().is_dark();
        let accent = cx.try_global::<ActiveAccent>().map_or(LAGOON, |a| a.0);
        let accent = accent.for_theme(dark);
        let t = &(if dark { DARK } else { LIGHT }).with_accent(accent);
        let backdrop = cx
            .try_global::<Material>()
            .map_or(Backdrop::default(), |m| m.0);
        Self {
            root: c(t.root(backdrop)),
            group_off: c(t.group_off(backdrop)),
            group_on: c(t.group_on(backdrop)),
            control_on_root: c(t.control_on_root(backdrop)),
            ink: c(t.ink),
            ink_2: c(t.ink_2),
            ink_3: c(t.ink_3),
            lagoon: c(t.lagoon),
            on_lagoon: c(t.on_lagoon),
            amber: c(t.amber),
            red: c(t.red),
            control: c(t.control),
            stroke: c(t.stroke),
            stroke_strong: c(t.stroke_strong),
            track: c(t.track),
            meter_well: c(meter_well(accent)),
            thumb: c(t.thumb),
            amber_wash: c(t.amber_wash),
            red_wash: c(t.red_wash),
        }
    }
}

pub const LIGHT_THEME: &str = "Zephon Light";
pub const DARK_THEME: &str = "Zephon Dark";

fn hex(v: u32) -> String {
    format!("#{v:08X}")
}

/// One gpui-kit `ThemeConfig` (DESIGN.md, "Mapping to gpui-kit theme tokens").
fn theme_config(name: &str, mode: &str, t: &Tokens) -> serde_json::Value {
    const CLEAR: u32 = 0x00000000;
    let colors: serde_json::Map<String, serde_json::Value> = [
        // Transparent: the view paints the root tint itself (see the module docs).
        ("background", CLEAR),
        ("foreground", t.ink),
        ("muted.background", t.layer_1),
        ("muted.foreground", t.ink_2),
        ("border", t.stroke),
        ("input.border", t.stroke_strong),
        ("window.border", t.window_border),
        ("title_bar.background", CLEAR),
        ("title_bar.border", CLEAR),
        ("accent.background", t.layer_2),
        ("accent.foreground", t.ink),
        ("primary.background", t.lagoon),
        ("primary.hover.background", t.lagoon_hover),
        ("primary.active.background", t.lagoon_active),
        ("primary.foreground", t.on_lagoon),
        ("secondary.background", t.control),
        ("secondary.hover.background", t.hover),
        ("secondary.active.background", t.pressed),
        ("secondary.foreground", t.ink),
        ("button.background", t.control),
        ("button.hover.background", t.control_hover),
        ("button.active.background", t.control_active),
        ("button.foreground", t.ink),
        ("list.background", t.layer_1),
        ("list.hover.background", t.hover),
        ("list.active.background", t.layer_2),
        ("list.active.border", t.lagoon),
        ("popover.background", t.popover),
        ("popover.foreground", t.ink),
        ("slider.background", t.lagoon),
        ("slider.thumb.background", t.thumb),
        ("switch.background", CLEAR),
        ("switch.thumb.background", t.ink_2),
        ("ring", t.ink),
        ("caret", t.ink),
        ("selection.background", t.lagoon),
        ("overlay", t.overlay),
        ("scrollbar.background", CLEAR),
        ("scrollbar.thumb.background", t.track),
        ("warning.background", t.amber),
        ("warning.foreground", t.on_lagoon),
        ("danger.background", t.red),
        ("danger.foreground", t.on_lagoon),
        ("success.background", t.lagoon),
        ("link", t.lagoon),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), hex(v).into()))
    .collect();
    serde_json::json!({
        "name": name,
        "mode": mode,
        // No "font.family": `apply_fonts` sets the chosen family, and a theme reload keeps a
        // family the file does not name.
        // rem = 16 px, so gpui-kit's `text_sm` controls render at the 14 px body size.
        // `apply_fonts` scales it with the text size after every reload.
        "font.size": 16,
        "radius": 4,
        "radius.lg": 8,
        // The only shadows are the window's and the slider thumb's.
        "shadow": false,
        "colors": colors,
    })
}

/// The light and dark themes with `accent`, as a gpui-kit `ThemeSet` JSON document.
pub fn theme_set_json(accent: &Accent) -> String {
    serde_json::json!({
        "name": "Zephon",
        "themes": [
            theme_config(LIGHT_THEME, "light", &LIGHT.with_accent(&accent.light)),
            theme_config(DARK_THEME, "dark", &DARK.with_accent(&accent.dark)),
        ]
    })
    .to_string()
}

/// Makes both themes (with `accent`) gpui-kit's light/dark pair. Call after
/// `gpui_kit::init` (which installs gpui-kit's own default themes), then load the mode
/// with `Theme::change`.
pub fn install_themes(accent: &Accent, cx: &mut App) {
    cx.set_global(ActiveAccent(*accent));
    // Parsed here, not through `ThemeRegistry::load_themes_from_str`: the registry keeps the
    // first theme of a name, so a new accent would never reach gpui-kit's controls.
    let set: ThemeSet = match serde_json::from_str(&theme_set_json(accent)) {
        Ok(set) => set,
        Err(e) => {
            tracing::error!("Could not load the app themes: {e:#}");
            return;
        }
    };
    let find = |name: &str| {
        let theme = set.themes.iter().find(|t| t.name.as_ref() == name);
        theme.cloned().map(Rc::new)
    };
    let (Some(light), Some(dark)) = (find(LIGHT_THEME), find(DARK_THEME)) else {
        return;
    };
    Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
}

/// Records the window material in use; the root tint follows it.
pub fn set_material(backdrop: Backdrop, cx: &mut App) {
    if cx.try_global::<Material>().is_none_or(|m| m.0 != backdrop) {
        cx.set_global(Material(backdrop));
    }
}

/// Makes gpui-kit use the UI font and scales its rem (and so its controls) with the text
/// size. Call after every `Theme::change`: a theme reload resets the size to the file's.
pub fn apply_fonts(fonts: &Fonts, cx: &mut App) {
    let size = px(16. * fonts.scale);
    if cx.theme().font_family != fonts.text || cx.theme().font_size != size {
        let family = fonts.text.clone();
        Theme::update(cx, |theme| {
            theme.font_family = family;
            theme.font_size = size;
        });
    }
}

// ---------- type ----------

/// The default face, as the settings window names it.
pub const DEFAULT_FONT_NAME: &str = "Segoe UI Variable";
const FONT_TEXT: &str = "Segoe UI Variable Text";
const FONT_DISPLAY: &str = "Segoe UI Variable Display";
const FONT_SMALL: &str = "Segoe UI Variable Small";
const FONT_FALLBACK: &str = "Segoe UI";

/// The UI's font families and text size. By default Segoe UI Variable's optical sizes (Small
/// for 12 px, Text for 14 px, Display for room names and lag figures), or plain Segoe UI for
/// all three on systems without it. A family chosen in the settings is used for all three.
#[derive(Clone, Debug, PartialEq)]
pub struct Fonts {
    pub text: SharedString,
    pub display: SharedString,
    pub small: SharedString,
    /// Text size factor (1.0 = the 12 / 14 / 15 scale).
    pub scale: f32,
}

impl Global for Fonts {}

impl Fonts {
    /// The fonts for the saved `family` among the `installed` families. A family that is not
    /// installed falls back to the default chain; the flag says it did.
    pub fn resolve(family: Option<&str>, installed: &[String], scale: f32) -> (Self, bool) {
        let has = |n: &str| installed.iter().any(|f| f == n);
        if let Some(family) = family
            && has(family)
        {
            let f: SharedString = family.to_string().into();
            return (
                Self {
                    text: f.clone(),
                    display: f.clone(),
                    small: f,
                    scale,
                },
                false,
            );
        }
        let pick = |n: &'static str| -> SharedString {
            if has(n) {
                n.into()
            } else {
                FONT_FALLBACK.into()
            }
        };
        let fonts = Self {
            text: pick(FONT_TEXT),
            display: pick(FONT_DISPLAY),
            small: pick(FONT_SMALL),
            scale,
        };
        (fonts, family.is_some())
    }

    /// The fonts in use (set by `appearance`).
    pub fn of(cx: &App) -> Self {
        cx.global::<Fonts>().clone()
    }

    /// A length that grows with the text size (boxes sized to hold text).
    pub fn px(&self, v: f32) -> Pixels {
        px(v * self.scale)
    }
}

/// The type scale (12 / 14 / 15, times the text size) as text styles on any element.
pub trait TypeExt: Styled + Sized {
    /// Meta, status, loss, captions, title bar: 12/16, optical size Small.
    fn t_small(self, fonts: &Fonts) -> Self {
        self.font_family(fonts.small.clone())
            .text_size(fonts.px(12.))
            .line_height(fonts.px(16.))
    }
    /// Body, controls, buttons: 14/20, optical size Text.
    fn t_body(self, fonts: &Fonts) -> Self {
        self.font_family(fonts.text.clone())
            .text_size(fonts.px(14.))
            .line_height(fonts.px(20.))
    }
    /// Room names: 15/22 semibold, optical size Display.
    fn t_name(self, fonts: &Fonts) -> Self {
        self.font_family(fonts.display.clone())
            .text_size(fonts.px(15.))
            .line_height(fonts.px(22.))
            .font_weight(FontWeight::SEMIBOLD)
    }
}

impl<E: Styled> TypeExt for E {}

/// "**Lead** rest" as one wrapping run, the lead semibold in `lead_color`
/// ("**Didn't connect.** No answer…", "**Glitchy?** Stable adds…", "**Playing** in 3 rooms").
pub fn lead_text(lead: &str, rest: &str, lead_color: Hsla) -> StyledText {
    let text = if rest.is_empty() {
        lead.to_string()
    } else {
        format!("{lead} {rest}")
    };
    StyledText::new(text).with_highlights([(
        0..lead.len(),
        HighlightStyle {
            color: Some(lead_color),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        },
    )])
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would bring GPUI's `test` macro in over Rust's `#[test]`.
    use super::{
        Accent, AccentSet, DARK, DARK_THEME, Fonts, LAGOON, LIGHT, LIGHT_THEME, Tokens, over,
        paint, theme_set_json,
    };
    use crate::settings::Backdrop;
    use crate::ui::accent::contrast;

    #[test]
    fn paint_adds_alpha_like_gpui() {
        // One layer is plain "source over" the backdrop.
        assert_eq!(paint(&[0xFFFFFF80], 0x00), 0x808080FF);
        assert_eq!(paint(&[0x00000080], 0xFF), 0x7F7F7FFF);
        // Two half-alpha layers add up to opaque, so no backdrop shows: white over black
        // comes out darker than "source over" (0xBF) over a white backdrop would.
        assert_eq!(paint(&[0x00000080, 0xFFFFFF80], 0xFF), 0x808080FF);
        assert_eq!(over(0xFFFFFF80, over(0x00000080, 0xFFFFFFFF)), 0xBFBFBFFF);
    }

    /// WCAG 2.2 AA: 4.5:1 for text (all of the app's text is under the large-text size), 3:1
    /// for the parts that show a control or its state.
    const TEXT: f32 = 4.5;
    const PART: f32 = 3.0;

    /// The surfaces colors are drawn on in one theme over `material`, as the layers GPUI
    /// paints, bottom first.
    fn surfaces(t: &Tokens, material: Backdrop, dark: bool) -> Vec<(&'static str, Vec<u32>)> {
        let (root, off, on) = (
            t.root(material),
            t.group_off(material),
            t.group_on(material),
        );
        // gpui-kit's ghost button hovers with Layer 2, at half its alpha in dark.
        let ghost = if dark {
            (t.layer_2 & !0xFF) | ((t.layer_2 & 0xFF) / 2)
        } else {
            t.layer_2
        };
        vec![
            ("root", vec![root]),
            ("group off", vec![off]),
            ("group on", vec![on]),
            ("control", vec![off, t.control]),
            ("control hover", vec![off, t.control_hover]),
            ("control pressed", vec![off, t.control_active]),
            ("control on root", vec![t.control_on_root(material)]),
            ("ghost hover on root", vec![root, ghost]),
            ("ghost pressed on root", vec![root, t.control_active]),
            ("ghost hover on group on", vec![on, ghost]),
            ("ghost pressed on group on", vec![on, t.control_active]),
            ("amber wash on root", vec![root, t.amber_wash]),
            ("amber wash on group on", vec![on, t.amber_wash]),
            (
                "button on amber wash on root",
                vec![root, t.amber_wash, t.control],
            ),
            (
                "button on amber wash on group on",
                vec![on, t.amber_wash, t.control],
            ),
            (
                "button hover on amber wash on root",
                vec![root, t.amber_wash, t.control_hover],
            ),
            (
                "button hover on amber wash on group on",
                vec![on, t.amber_wash, t.control_hover],
            ),
            (
                "button pressed on amber wash on root",
                vec![root, t.amber_wash, t.control_active],
            ),
            (
                "button pressed on amber wash on group on",
                vec![on, t.amber_wash, t.control_active],
            ),
            // A room that is off keeps its error.
            ("red wash on group off", vec![off, t.red_wash]),
            ("red wash on group on", vec![on, t.red_wash]),
            // A profile stop's own fill, painted over the room (see `INSIDE`).
            ("profile stop", vec![on, on]),
            ("popover", vec![t.popover]),
            ("popover hover", vec![t.popover, t.hover]),
            ("popover selected", vec![t.popover, t.layer_2]),
            ("slider thumb", vec![t.thumb]),
        ]
    }

    /// Small surfaces drawn on another, with the one around them. A color at their edge (the
    /// ring of a profile stop) borders both, and the stop's fill adds alpha, so the ring is
    /// not just the track on the room.
    const INSIDE: [(&str, &str); 1] = [("profile stop", "group on")];

    /// Each color, the contrast it needs and the surfaces the UI draws it on.
    fn uses(t: &Tokens) -> Vec<(&'static str, u32, f32, Vec<&'static str>)> {
        let walls = ["root", "group off", "group on"];
        let popover = ["popover", "popover hover", "popover selected"];
        let controls = [
            "control",
            "control hover",
            "control pressed",
            "control on root",
        ];
        let banners = [
            "ghost hover on group on",
            "amber wash on root",
            "amber wash on group on",
            "button on amber wash on root",
            "button on amber wash on group on",
            "button hover on amber wash on root",
            "button hover on amber wash on group on",
            "button pressed on amber wash on root",
            "button pressed on amber wash on group on",
            "red wash on group off",
            "red wash on group on",
        ];
        let all = [&walls[..], &popover, &controls, &banners].concat();
        vec![
            // Also the focus ring.
            ("Ink", t.ink, TEXT, all),
            (
                "Ink 2",
                t.ink_2,
                TEXT,
                [
                    &walls[..],
                    &popover,
                    &["control", "control on root"],
                    &[
                        "ghost hover on group on",
                        "ghost pressed on group on",
                        "amber wash on root",
                    ],
                ]
                .concat(),
            ),
            // The settings and refresh icons.
            (
                "Ink 2 icon",
                t.ink_2,
                PART,
                vec!["ghost hover on root", "ghost pressed on root"],
            ),
            // Never on the root tint, where Light Ink 3 over acrylic is about 4:1: the IP,
            // "Offline", dimmed names and an unknown volume sit in rooms, menu hints in menus
            // (a select shows its item's title).
            ("Ink 3", t.ink_3, TEXT, [&walls[1..], &popover].concat()),
            (
                "Amber",
                t.amber,
                TEXT,
                vec!["group off", "group on", "amber wash on group on"],
            ),
            // The firewall banner's warning sign.
            ("Amber icon", t.amber, PART, vec!["amber wash on root"]),
            (
                "Red",
                t.red,
                TEXT,
                vec!["root", "red wash on group off", "red wash on group on"],
            ),
            // Rails, the outline of a switch that is off, the ring of a profile stop.
            (
                "Track",
                t.track,
                PART,
                vec!["group off", "group on", "profile stop"],
            ),
            // The slider thumb's inner dot (`accent` checks the accent on the glass).
            ("Lagoon dot", t.lagoon, PART, vec!["slider thumb"]),
        ]
    }

    #[test]
    fn every_color_is_readable_over_any_backdrop() {
        let mut failures = Vec::new();
        for (theme, t, dark) in [("Light", LIGHT, false), ("Dark", DARK, true)] {
            for material in [Backdrop::Mica, Backdrop::Acrylic, Backdrop::Solid] {
                let surfaces = surfaces(&t, material, dark);
                let layers_of = |name: &str| {
                    let (_, layers) = surfaces.iter().find(|(s, _)| *s == name).unwrap();
                    layers
                };
                for (name, color, min, on) in uses(&t) {
                    for surface in on {
                        let layers = layers_of(surface);
                        let around = INSIDE
                            .iter()
                            .find(|(s, _)| *s == surface)
                            .map(|(_, a)| layers_of(a));
                        // The color is one more layer: text and icons are opaque, the track
                        // is a tint that must stand out from the surface around it.
                        let with = [&layers[..], &[color]].concat();
                        let worst = t
                            .backdrops(material)
                            .map(|b| {
                                let painted = paint(&with, b);
                                let beside = [Some(layers), around].into_iter().flatten();
                                let ratio = beside
                                    .map(|l| contrast(painted, paint(l, b)))
                                    .fold(f32::MAX, f32::min);
                                (ratio, b)
                            })
                            .min_by(|a, b| a.0.total_cmp(&b.0))
                            .unwrap();
                        if worst.0 < min {
                            failures.push(format!(
                                "{theme} {name} on {surface} over {material:?} (grey {:02X}): \
                                 {:.2}, needs {min}",
                                worst.1, worst.0
                            ));
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn over_matches_css_source_over() {
        // Opaque top wins; a clear top leaves the bottom.
        assert_eq!(over(0x112233FF, 0xFFFFFFFF), 0x112233FF);
        assert_eq!(over(0x11223300, 0xEEF1F6BD), 0xEEF1F6BD);
        // Light Layer 1 over the Mica root tint: 0.36 + 0.35 * 0.64 = 0.58 alpha (149/255
        // with the byte-rounded inputs), and whiter than the root.
        let v = over(LIGHT.layer_1, LIGHT.root_mica);
        assert_eq!(v & 0xFF, 149);
        assert!((v >> 24) > 0xEE);
        // 0.6 alpha over 0.5 white: alpha 204.2, channels 102.1, 140.4 and 216.8 (to nearest).
        assert_eq!(over(0x3366CC99, 0xFFFFFF80), 0x668CD9CC);
    }

    #[test]
    fn theme_set_parses_as_gpui_kit_themes() {
        let blue = Accent {
            light: AccentSet {
                base: 0x0067C0FF,
                hover: 0x005BAAFF,
                active: 0x004F94FF,
                on: 0xFFFFFFFF,
            },
            dark: AccentSet {
                base: 0x4CC2FFFF,
                hover: 0x6BCDFFFF,
                active: 0x3AB0EEFF,
                on: 0x002A40FF,
            },
        };
        let set: gpui_kit::component::ThemeSet =
            serde_json::from_str(&theme_set_json(&blue)).unwrap();
        assert_eq!(set.themes.len(), 2);
        let (light, dark) = (&set.themes[0], &set.themes[1]);
        assert_eq!(light.name.as_ref(), LIGHT_THEME);
        assert!(!light.mode.is_dark());
        assert_eq!(dark.name.as_ref(), DARK_THEME);
        assert!(dark.mode.is_dark());
        let colors = |t: &gpui_kit::component::ThemeConfig| {
            let c = &t.colors;
            [
                &c.foreground,
                &c.primary,
                &c.primary_hover,
                &c.primary_active,
                &c.primary_foreground,
            ]
            .map(|v| v.as_deref().unwrap_or("missing").to_string())
        };
        // Text is the palette's ink, whatever the ink is; the accent tokens come from `blue`.
        let ink = |t: &Tokens| format!("#{:08X}", t.ink);
        assert_eq!(
            colors(light),
            [
                &ink(&LIGHT),
                "#0067C0FF",
                "#005BAAFF",
                "#004F94FF",
                "#FFFFFFFF"
            ]
        );
        assert_eq!(
            colors(dark),
            [
                &ink(&DARK),
                "#4CC2FFFF",
                "#6BCDFFFF",
                "#3AB0EEFF",
                "#002A40FF"
            ]
        );
        // Lagoon's own themes keep the teal.
        let lagoon: gpui_kit::component::ThemeSet =
            serde_json::from_str(&theme_set_json(&LAGOON)).unwrap();
        assert_eq!(colors(&lagoon.themes[0])[1], "#00708AFF");
        assert_eq!(colors(&lagoon.themes[1])[1], "#5CC8D6FF");
    }

    #[test]
    fn fonts_fall_back_to_segoe_when_the_saved_family_is_gone() {
        let installed: Vec<String> = [
            "Bahnschrift",
            "Segoe UI",
            "Segoe UI Variable Display",
            "Segoe UI Variable Small",
            "Segoe UI Variable Text",
        ]
        .map(String::from)
        .to_vec();
        let roles = |f: &Fonts| {
            (
                f.small.to_string(),
                f.text.to_string(),
                f.display.to_string(),
            )
        };
        let (f, missing) = Fonts::resolve(None, &installed, 1.0);
        assert!(!missing);
        assert_eq!(
            roles(&f),
            (
                "Segoe UI Variable Small".into(),
                "Segoe UI Variable Text".into(),
                "Segoe UI Variable Display".into()
            )
        );
        // A chosen family is used for every role, at the chosen size.
        let (f, missing) = Fonts::resolve(Some("Bahnschrift"), &installed, 1.15);
        assert!(!missing);
        assert_eq!(
            roles(&f),
            (
                "Bahnschrift".into(),
                "Bahnschrift".into(),
                "Bahnschrift".into()
            )
        );
        assert!((f32::from(f.px(20.)) - 23.).abs() < 1e-4);
        // Uninstalled: the default chain, flagged.
        let (f, missing) = Fonts::resolve(Some("Fixel Text"), &installed, 1.0);
        assert!(missing);
        assert_eq!(f.text.as_ref(), "Segoe UI Variable Text");
        // Without Segoe UI Variable: plain Segoe UI.
        let (f, _) = Fonts::resolve(None, &["Segoe UI".to_string()], 1.0);
        assert_eq!(
            roles(&f),
            ("Segoe UI".into(), "Segoe UI".into(), "Segoe UI".into())
        );
    }
}
