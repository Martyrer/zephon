//! Design tokens (design/DESIGN.md): the two palettes, the gpui-kit theme built from them,
//! and the Segoe UI Variable type roles.
//!
//! The accent (Lagoon, or one derived from the Windows accent in `accent`) and the type
//! scale (text size, font family) are chosen in the settings window, so the palettes, the
//! theme JSON and [`Fonts`] are built at run time from these constants.
//!
//! Every surface is a translucent tint over the window material, so no color here is an
//! opaque panel except the select menu. gpui-kit components read the theme JSON; the app's
//! own elements (rooms, switch, slider, lag line) read [`Palette`] directly, because some
//! tokens have no gpui-kit field (amber/red washes) and gpui-kit clamps `list_active` to 20 %.
//!
//! GPUI's renderer adds alpha when it blends (D3D11 `SrcBlendAlpha = DestBlendAlpha = ONE`),
//! so a translucent layer painted over the translucent root comes out more opaque than
//! "source over" and hides the material (light Layer 1 over the root turned grey). The large
//! surfaces are therefore painted once each with the precomposited color ([`Palette::root`],
//! [`Palette::group_off`], [`Palette::group_on`]); the theme `background` is transparent so
//! gpui-kit's root does not paint under them.

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
}

pub const LIGHT: Tokens = Tokens {
    ink: 0x1B1E25FF,
    ink_2: 0x565C69FF,
    ink_3: 0x868C99FF,
    lagoon: 0x00708AFF,
    lagoon_hover: 0x00627AFF,
    lagoon_active: 0x005466FF,
    on_lagoon: 0xFFFFFFFF,
    amber: 0x8F5300FF,
    red: 0xB4232CFF,
    root_mica: 0xEEF1F659, // 0.35: Mica is calm, so a light tint keeps its color
    root_acrylic: 0xEEF1F6DB, // 0.86
    root_solid: 0xEEF1F6FF,
    layer_1: 0xFFFFFF5C,        // 0.36
    layer_2: 0xFFFFFFA8,        // 0.66
    control: 0xFFFFFFB8,        // 0.72
    control_hover: 0xFFFFFFD9,  // 0.85
    control_active: 0xFFFFFF8C, // 0.55
    stroke: 0x161C2D16,         // 0.085
    stroke_strong: 0x161C2D2E,  // 0.18
    track: 0x161C2D3D,          // 0.24
    thumb: 0xFFFFFFFF,
    amber_wash: 0x8F530016, // 0.085
    red_wash: 0xB4232C13,   // 0.075
    hover: 0x1B1E250F,
    pressed: 0x1B1E251A,
    popover: 0xF9FAFBFF,
    window_border: 0x00000024, // 0.14
    overlay: 0x00000033,       // 0.20
    material: 0xF3F3F3FF,
};

pub const DARK: Tokens = Tokens {
    ink: 0xF1F3F6FF,
    ink_2: 0xA9AFBBFF,
    ink_3: 0x6F7682FF,
    lagoon: 0x5CC8D6FF,
    lagoon_hover: 0x79D4E0FF,
    lagoon_active: 0x4AB4C2FF,
    on_lagoon: 0x062A31FF,
    amber: 0xF0B455FF,
    red: 0xFF9494FF,
    root_mica: 0x1B1E2566,    // 0.40
    root_acrylic: 0x1B1E25E0, // 0.88
    root_solid: 0x1B1E25FF,
    layer_1: 0xFFFFFF0A,        // 0.040
    layer_2: 0xFFFFFF13,        // 0.075
    control: 0xFFFFFF11,        // 0.065
    control_hover: 0xFFFFFF1A,  // 0.10
    control_active: 0xFFFFFF0B, // 0.045
    stroke: 0xFFFFFF13,         // 0.075
    stroke_strong: 0xFFFFFF26,  // 0.15
    track: 0xFFFFFF4D,          // 0.30
    thumb: 0x454B56FF,
    amber_wash: 0xF0B4551A, // 0.10
    red_wash: 0xFF949417,   // 0.09
    hover: 0xFFFFFF0F,
    pressed: 0xFFFFFF0A,
    popover: 0x2A2E36FF,
    window_border: 0xFFFFFF1A, // 0.10
    overlay: 0x00000073,       // 0.45
    material: 0x202020FF,
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

/// CSS "source over" of two `0xRRGGBBAA` colors, as a straight-alpha color.
pub fn over(top: u32, bottom: u32) -> u32 {
    let ch = |v: u32, shift: u32| ((v >> shift) & 0xFF) as f32 / 255.;
    let (ta, ba) = (ch(top, 0), ch(bottom, 0));
    let a = ta + ba * (1. - ta);
    if a <= 0. {
        return 0;
    }
    let mix = |shift: u32| {
        let v = (ch(top, shift) * ta + ch(bottom, shift) * ba * (1. - ta)) / a;
        (v * 255.).round() as u32
    };
    (mix(24) << 24) | (mix(16) << 16) | (mix(8) << 8) | (a * 255.).round() as u32
}

/// The window material in use (after fallbacks); the root tint depends on it.
pub struct Material(pub Backdrop);

impl Global for Material {}

impl Palette {
    pub fn of(cx: &App) -> Self {
        let dark = cx.theme().is_dark();
        let accent = cx.try_global::<ActiveAccent>().map_or(LAGOON, |a| a.0);
        let t = &(if dark { DARK } else { LIGHT }).with_accent(accent.for_theme(dark));
        let backdrop = cx
            .try_global::<Material>()
            .map_or(Backdrop::default(), |m| m.0);
        let root = match backdrop {
            Backdrop::Mica => t.root_mica,
            Backdrop::Acrylic => t.root_acrylic,
            Backdrop::Solid => t.root_solid,
        };
        let group_off = over(t.layer_1, root);
        Self {
            root: c(root),
            group_off: c(group_off),
            group_on: c(over(t.layer_2, group_off)),
            control_on_root: c(over(t.control, over(root, t.material))),
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
    /// Text size factor (1.0 = the 12 / 14 / 15 / 20 scale).
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

/// The type scale (12 / 14 / 15 / 20, times the text size) as text styles on any element.
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
    /// Lag figures: 20/24 semibold, optical size Display.
    fn t_figure(self, fonts: &Fonts) -> Self {
        self.font_family(fonts.display.clone())
            .text_size(fonts.px(20.))
            .line_height(fonts.px(24.))
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
        theme_set_json,
    };

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
