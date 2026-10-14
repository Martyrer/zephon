//! The look both windows share: theme, window material, font, text size and accent.
//!
//! [`Appearance`] is a GPUI global. [`update`] edits it and re-applies everything app-wide
//! (the gpui-kit theme with the accent, its font and rem, [`Fonts`]); each window observes the
//! global and sets what belongs to its own HWND (material, dark frame) with [`WindowGlass`].

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::*;

use super::accent;
use super::glass;
use super::style::{self, Accent, Fonts, LAGOON};
use crate::settings::{AccentChoice, Backdrop, Settings, TextSize, ThemeChoice};

pub struct Appearance {
    pub theme: ThemeChoice,
    /// The material asked for (each window may fall back, see [`WindowGlass`]).
    pub backdrop: Backdrop,
    /// The saved font family (`None` = default).
    pub font: Option<String>,
    /// The saved family is not installed, so the default is in use.
    pub font_missing: bool,
    pub text_size: TextSize,
    pub accent: AccentChoice,
    /// The Windows accent, derived for the glass; `None` when it could not be read.
    pub windows_accent: Option<Accent>,
    /// Installed font families, sorted.
    pub font_names: Vec<String>,
}

impl Global for Appearance {}

impl Appearance {
    /// The accent in use: Windows' when chosen and readable, else Lagoon.
    pub fn accent_colors(&self) -> Accent {
        match (self.accent, self.windows_accent) {
            (AccentChoice::Windows, Some(a)) => a,
            _ => LAGOON,
        }
    }
}

/// Installs the global from the saved settings and applies it. Call after `gpui_kit::init`.
pub fn init(saved: &Settings, cx: &mut App) {
    let mut a = Appearance {
        theme: saved.theme,
        backdrop: saved.backdrop,
        font: saved.font_family.clone(),
        font_missing: false,
        text_size: saved.text_size,
        accent: saved.accent,
        windows_accent: accent::windows_accent(),
        font_names: cx.text_system().all_font_names(),
    };
    if a.windows_accent.is_none() {
        tracing::info!("Windows accent color not readable; the Windows accent choice uses Lagoon");
    }
    apply(&mut a, cx);
    let f = Fonts::of(cx);
    tracing::info!(
        "UI fonts: {} / {} / {} at {:.2}x",
        f.small,
        f.text,
        f.display,
        f.scale
    );
    cx.set_global(a);
}

/// Edits the appearance, re-applies it app-wide and lets both windows know.
pub fn update(cx: &mut App, edit: impl FnOnce(&mut Appearance)) {
    cx.update_global::<Appearance, _>(|a, cx| {
        edit(a);
        apply(a, cx);
    });
}

/// Re-reads the Windows accent (after a change in Windows) and applies it if it changed.
pub fn refresh_windows_accent(cx: &mut App) {
    let now = accent::windows_accent();
    if cx.global::<Appearance>().windows_accent != now {
        tracing::info!("Windows accent changed");
        update(cx, |a| a.windows_accent = now);
    }
}

fn apply(a: &mut Appearance, cx: &mut App) {
    let (fonts, missing) = Fonts::resolve(a.font.as_deref(), &a.font_names, a.text_size.scale());
    if missing && !a.font_missing {
        tracing::warn!("Font {:?} is not installed; using the default", a.font);
    }
    a.font_missing = missing;
    style::install_themes(&a.accent_colors(), cx);
    match a.theme {
        ThemeChoice::System => Theme::sync_system_appearance(None, cx),
        ThemeChoice::Light => Theme::change(ThemeMode::Light, None, cx),
        ThemeChoice::Dark => Theme::change(ThemeMode::Dark, None, cx),
    }
    style::apply_fonts(&fonts, cx);
    cx.set_global(fonts);
}

/// One window's material and frame.
pub struct WindowGlass {
    /// The material last applied to this window, as asked for and as in use.
    requested: Option<Backdrop>,
    pub in_use: Backdrop,
}

impl WindowGlass {
    pub fn new() -> Self {
        Self {
            requested: None,
            in_use: Backdrop::default(),
        }
    }

    /// Brings the window in line with the appearance: its material (only when the choice
    /// changed: re-applying flashes), the root tint for it, and a light or dark frame.
    pub fn sync(&mut self, window: &mut Window, cx: &mut App) {
        let requested = cx.global::<Appearance>().backdrop;
        if self.requested != Some(requested) {
            self.requested = Some(requested);
            self.in_use = glass::apply_backdrop(window, requested);
        }
        style::set_material(self.in_use, cx);
        let dark = cx.theme().is_dark();
        glass::set_dark_frame(window, dark);
        // After an OS theme change GPUI sets the frame from the OS setting once the
        // observers have run, so set it again on the next frame.
        window.on_next_frame(move |window, _| glass::set_dark_frame(window, dark));
        window.refresh();
    }

    /// For the window's appearance observer: "System" follows Windows' light/dark setting.
    pub fn on_os_appearance(&mut self, window: &mut Window, cx: &mut App) {
        if cx.global::<Appearance>().theme == ThemeChoice::System {
            // Both windows observe the global and sync.
            update(cx, |_| {});
        } else {
            // GPUI resets the frame to the OS setting after this callback.
            let dark = cx.theme().is_dark();
            window.on_next_frame(move |window, _| glass::set_dark_frame(window, dark));
        }
    }
}
