//! The window material (Mica, acrylic or opaque), rounded corners and the dark frame,
//! set through DWM on the window's HWND.
//!
//! GPUI clears the frame to transparent for any non-opaque `WindowBackgroundAppearance`, so
//! the material shows through the theme's translucent root tint. Mica is DWM's system
//! backdrop (`DWMWA_SYSTEMBACKDROP_TYPE`, Windows 11 build 22621+); acrylic is GPUI's
//! `Blurred` (the accent-policy blur, build 17763+). DWM tints Mica light or dark from
//! `DWMWA_USE_IMMERSIVE_DARK_MODE`, which must follow the app theme, not only the OS.

use gpui_kit::{Window, WindowBackgroundAppearance};

use crate::settings::Backdrop;

/// Applies `requested`, falling back Mica -> acrylic -> opaque when the OS cannot do it.
/// Returns the material in use.
pub fn apply_backdrop(window: &Window, requested: Backdrop) -> Backdrop {
    let build = sys::os_build();
    let mut backdrop = requested;
    if backdrop == Backdrop::Mica && build < sys::MICA_BUILD {
        backdrop = Backdrop::Acrylic;
    }
    if backdrop == Backdrop::Acrylic && build < sys::ACRYLIC_BUILD {
        backdrop = Backdrop::Solid;
    }
    let hwnd = sys::hwnd(window);

    // Clear the previous material first: GPUI's Opaque resets the accent policy (acrylic),
    // and the system backdrop is ours to reset.
    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
    if let Some(hwnd) = hwnd {
        let kind = if backdrop == Backdrop::Mica {
            sys::DWMSBT_MAINWINDOW
        } else {
            sys::DWMSBT_NONE
        };
        if !sys::set_system_backdrop(hwnd, kind) && backdrop == Backdrop::Mica {
            backdrop = if build >= sys::ACRYLIC_BUILD {
                Backdrop::Acrylic
            } else {
                Backdrop::Solid
            };
        }
    } else if backdrop == Backdrop::Mica {
        backdrop = Backdrop::Acrylic;
    }
    window.set_background_appearance(match backdrop {
        // GPUI's MicaBackdrop sets the same DWM attribute; it also tells its renderer to
        // clear to transparent.
        Backdrop::Mica => WindowBackgroundAppearance::MicaBackdrop,
        Backdrop::Acrylic => WindowBackgroundAppearance::Blurred,
        Backdrop::Solid => WindowBackgroundAppearance::Opaque,
    });
    if backdrop != requested {
        tracing::info!(
            "Window material {requested:?} not available (build {build}); using {backdrop:?}"
        );
    }
    backdrop
}

/// Dark or light window frame and Mica tint.
pub fn set_dark_frame(window: &Window, dark: bool) {
    if let Some(hwnd) = sys::hwnd(window) {
        sys::set_dark_mode(hwnd, dark);
    }
}

/// Windows 11 rounded corners.
///
/// The frame is not extended into the client area (`DwmExtendFrameIntoClientArea`): with
/// GPUI's composition swap chain the system backdrop already shows behind transparent
/// pixels, and an extended frame makes DWM draw a second set of caption buttons.
pub fn init_frame(window: &Window) {
    if let Some(hwnd) = sys::hwnd(window) {
        sys::round_corners(hwnd);
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    pub const MICA_BUILD: u32 = 22621;
    pub const ACRYLIC_BUILD: u32 = 17763;

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    const DWMWCP_ROUND: u32 = 2;
    pub const DWMSBT_NONE: u32 = 1;
    pub const DWMSBT_MAINWINDOW: u32 = 2;

    #[repr(C)]
    struct OsVersionInfoW {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        csd_version: [u16; 128],
    }

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }

    /// The real OS build (RtlGetVersion is not subject to the manifest version lie).
    pub fn os_build() -> u32 {
        let mut info = OsVersionInfoW {
            size: size_of::<OsVersionInfoW>() as u32,
            major: 0,
            minor: 0,
            build: 0,
            platform: 0,
            csd_version: [0; 128],
        };
        // SAFETY: `info` is a correctly sized OSVERSIONINFOW.
        if unsafe { RtlGetVersion(&mut info) } == 0 {
            info.build
        } else {
            0
        }
    }

    pub fn hwnd(window: &Window) -> Option<isize> {
        // `Window::window_handle` (GPUI's own handle) shadows the raw-window-handle trait.
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }

    fn set_u32(hwnd: isize, attribute: u32, value: u32) -> bool {
        // SAFETY: a 4-byte value for a DWORD/BOOL/enum attribute of a live window.
        let hr = unsafe {
            DwmSetWindowAttribute(hwnd, attribute, &value as *const u32 as *const c_void, 4)
        };
        hr >= 0
    }

    pub fn set_system_backdrop(hwnd: isize, kind: u32) -> bool {
        set_u32(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, kind)
    }

    pub fn set_dark_mode(hwnd: isize, dark: bool) {
        set_u32(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, dark as u32);
    }

    pub fn round_corners(hwnd: isize) {
        set_u32(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);
    }
}

#[cfg(not(windows))]
mod sys {
    use gpui_kit::Window;

    pub const MICA_BUILD: u32 = u32::MAX;
    pub const ACRYLIC_BUILD: u32 = u32::MAX;
    pub const DWMSBT_NONE: u32 = 1;
    pub const DWMSBT_MAINWINDOW: u32 = 2;

    pub fn os_build() -> u32 {
        0
    }
    pub fn hwnd(_: &Window) -> Option<isize> {
        None
    }
    pub fn set_system_backdrop(_: isize, _: u32) -> bool {
        false
    }
    pub fn set_dark_mode(_: isize, _: bool) {}
    pub fn round_corners(_: isize) {}
}
