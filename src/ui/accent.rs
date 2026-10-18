//! The Windows accent color, made safe for the app's glass.
//!
//! Windows keeps its accent palette in `HKCU\...\Explorer\Accent\AccentPalette`: eight RGBA
//! colors, Light 3 to Light 1, the accent, Dark 1 to Dark 3 (the same values as
//! `UISettings.GetColorValue(AccentLight3..AccentDark3)`). Like Windows' own controls, the
//! light theme starts from Dark 1 and the dark theme from Light 2. When the palette is
//! missing, the DWM `AccentColor` (ABGR) seeds both.
//!
//! The seed is then moved in lightness until text on it and the accent on the glass reach
//! [`MIN_CONTRAST`] (WCAG 2), checked against the footer and room fills over the darkest and
//! the brightest backdrop each material allows (see `style`). The text on it is white in the
//! light theme and a near-black of the same hue in the dark theme, as with Lagoon. Hover and
//! pressed fills step away from the base and are checked the same way.
//!
//! A thread blocks in `RegNotifyChangeKeyValue` on the Accent key, so a new Windows accent
//! applies live.

use gpui_kit::{Hsla, Rgba, rgba};

use super::style::{Accent, AccentSet, DARK, LIGHT, Tokens, paint};
use crate::settings::Backdrop;
use crate::winreg;

const ACCENT_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
const DWM_KEY: &str = r"Software\Microsoft\Windows\DWM";

/// Text on the accent, and the accent as text or a mark on the glass ("Playing", a switch
/// that is on, the slider fill). 4.5:1 is the WCAG AA level for body text, which the 12 px
/// "Playing" is; it is above the 3:1 needed for the accent as a graphic.
pub const MIN_CONTRAST: f32 = 4.5;

/// The Windows accent, derived for both themes; `None` when it cannot be read.
pub fn windows_accent() -> Option<Accent> {
    let (light_seed, dark_seed) = match winreg::read_binary(ACCENT_KEY, "AccentPalette") {
        Some(p) if p.len() >= 28 => {
            let color = |i: usize| u32::from_be_bytes([p[i * 4], p[i * 4 + 1], p[i * 4 + 2], 0xFF]);
            (color(4), color(1))
        }
        _ => {
            let [r, g, b, _] = winreg::read_dword(DWM_KEY, "AccentColor")?.to_le_bytes();
            let base = u32::from_be_bytes([r, g, b, 0xFF]);
            (base, base)
        }
    };
    Some(derive(light_seed, dark_seed))
}

/// Calls `on_change` after each change under the Accent key, until it returns false.
pub fn watch(on_change: impl Fn() -> bool + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("accent-watch".into())
        .spawn(move || while winreg::wait_for_change(ACCENT_KEY) && on_change() {});
    if let Err(e) = spawned {
        tracing::warn!("Windows accent changes won't apply live: {e}");
    }
}

/// The app accent from Windows' seeds for the light and dark themes.
pub fn derive(light_seed: u32, dark_seed: u32) -> Accent {
    Accent {
        light: derive_light(light_seed),
        dark: derive_dark(dark_seed),
    }
}

fn derive_light(seed: u32) -> AccentSet {
    let on = 0xFFFFFFFF;
    let glass = glass(&LIGHT);
    // Darker raises both contrasts (white text, light glass).
    let ok = |c: u32| contrast(c, on) >= MIN_CONTRAST && min_contrast(c, &glass) >= MIN_CONTRAST;
    let base = adjust(seed, -1., ok);
    AccentSet {
        base,
        hover: shift(base, -0.04),
        active: shift(base, -0.08),
        on,
    }
}

fn derive_dark(seed: u32) -> AccentSet {
    // Near-black of the accent's hue, like Lagoon's #062A31.
    let hue = Hsla::from(rgba(seed));
    let on = pack(Hsla {
        s: hue.s.min(0.8),
        l: 0.11,
        ..hue
    });
    let glass = glass(&DARK);
    // Lighter raises both contrasts (dark text, dark glass).
    let ok = |c: u32| contrast(c, on) >= MIN_CONTRAST && min_contrast(c, &glass) >= MIN_CONTRAST;
    let base = adjust(seed, 1., ok);
    let hover = adjust(shift(base, 0.06), 1., ok);
    // Pressed is darker, but the text on it must stay readable.
    let active = adjust(shift(base, -0.05), 1., |c| contrast(c, on) >= MIN_CONTRAST);
    AccentSet {
        base,
        hover,
        active,
        on,
    }
}

/// The surfaces the accent sits on in one theme: Layer 1 (the footer, rooms that are off,
/// the settings groups) and Layer 2 (rooms that are on) over each material, each over the
/// darkest and the brightest backdrop the material allows. Painting is affine in the
/// backdrop, so those two bound every shade between. The root tint carries only the glyph, a
/// logo.
fn glass(t: &Tokens) -> Vec<u32> {
    [Backdrop::Mica, Backdrop::Acrylic, Backdrop::Solid]
        .into_iter()
        .flat_map(|m| {
            let ends = t.backdrops(m);
            [t.group_off(m), t.group_on(m)]
                .into_iter()
                .flat_map(move |s| [paint(&[s], *ends.start()), paint(&[s], *ends.end())])
        })
        .collect()
}

/// Steps the lightness of `color` in `direction` (+1 lighter, -1 darker) until `ok`, or the
/// end of the range.
fn adjust(color: u32, direction: f32, ok: impl Fn(u32) -> bool) -> u32 {
    let mut c = color;
    for _ in 0..100 {
        if ok(c) {
            return c;
        }
        let next = shift(c, 0.01 * direction);
        if next == c {
            break;
        }
        c = next;
    }
    c
}

/// `color` with its HSL lightness moved by `dl`.
fn shift(color: u32, dl: f32) -> u32 {
    let mut h = Hsla::from(rgba(color));
    h.l = (h.l + dl).clamp(0., 1.);
    pack(h)
}

fn pack(h: Hsla) -> u32 {
    let c = Rgba::from(h);
    let ch = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    (ch(c.r) << 24) | (ch(c.g) << 16) | (ch(c.b) << 8) | 0xFF
}

/// WCAG relative luminance of an opaque `0xRRGGBBAA` color.
fn luminance(c: u32) -> f32 {
    let lin = |shift: u32| {
        let v = ((c >> shift) & 0xFF) as f32 / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(24) + 0.7152 * lin(16) + 0.0722 * lin(8)
}

/// WCAG contrast ratio of two opaque colors (1 to 21).
pub fn contrast(a: u32, b: u32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn min_contrast(c: u32, surfaces: &[u32]) -> f32 {
    surfaces
        .iter()
        .map(|&s| contrast(c, s))
        .fold(f32::MAX, f32::min)
}

#[cfg(test)]
mod tests {
    use super::{DARK, LIGHT, MIN_CONTRAST, contrast, derive, glass, min_contrast};
    use crate::ui::style::{AccentSet, LAGOON};

    fn assert_readable(set: &AccentSet, surfaces: &[u32]) {
        for (name, fill) in [
            ("base", set.base),
            ("hover", set.hover),
            ("active", set.active),
        ] {
            let c = contrast(fill, set.on);
            assert!(c >= MIN_CONTRAST, "text on {name} {fill:08X}: {c:.2}");
        }
        for (name, fill) in [("base", set.base), ("hover", set.hover)] {
            let c = min_contrast(fill, surfaces);
            assert!(c >= MIN_CONTRAST, "{name} {fill:08X} on glass: {c:.2}");
        }
    }

    #[test]
    fn contrast_matches_wcag() {
        assert!((contrast(0x000000FF, 0xFFFFFFFF) - 21.).abs() < 0.01);
        assert!((contrast(0x777777FF, 0xFFFFFFFF) - 4.48).abs() < 0.01);
        // The primaries on white weigh each channel differently.
        for (color, ratio) in [
            (0xFF0000FF, 3.998),
            (0x00FF00FF, 1.372),
            (0x0000FFFF, 8.592),
        ] {
            let c = contrast(color, 0xFFFFFFFF);
            assert!((c - ratio).abs() < 0.01, "{color:08X}: {c:.3}");
        }
    }

    #[test]
    fn lagoon_meets_the_same_bar() {
        assert_readable(&LAGOON.light, &glass(&LIGHT));
        assert_readable(&LAGOON.dark, &glass(&DARK));
    }

    #[test]
    fn windows_blue_keeps_its_palette_colors() {
        // The default Windows 11 palette: Dark 1 #0067C0 and Light 2 #4CC2FF already pass.
        let a = derive(0x0067C0FF, 0x4CC2FFFF);
        assert_eq!((a.light.base, a.dark.base), (0x0067C0FF, 0x4CC2FFFF));
        assert_eq!(a.light.on, 0xFFFFFFFF);
        assert_readable(&a.light, &glass(&LIGHT));
        assert_readable(&a.dark, &glass(&DARK));
    }

    #[test]
    fn hard_accents_are_moved_until_readable() {
        // Pale yellow in the light theme, near-black red in the dark theme, mid grey in both.
        for seed in [
            0xFFF100FF, 0xFFFFFFFF, 0x3A0000FF, 0x000000FF, 0x808080FF, 0x00CC6AFF,
        ] {
            let a = derive(seed, seed);
            assert_readable(&a.light, &glass(&LIGHT));
            assert_readable(&a.dark, &glass(&DARK));
        }
        // The light theme darkens yellow; the dark theme lightens dark red.
        let a = derive(0xFFF100FF, 0x3A0000FF);
        assert!(contrast(a.light.base, 0xFFFFFFFF) >= MIN_CONTRAST);
        assert_ne!(a.dark.base, 0x3A0000FF);
    }
}
