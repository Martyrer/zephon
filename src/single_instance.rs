//! One GUI at a time: a named mutex marks the running instance, and a second launch brings
//! its window forward instead of opening another.

use crate::{APP_TITLE, controller, settings};

/// GUI only: a second GUI (Start with Windows plus a manual launch, say) would fight the
/// first over the same receivers and settings file. If one is running, brings its window
/// forward and returns true. Demo mode is exempt: it has its own files and made-up rooms.
///
/// The running instance may have no window yet (still starting) or no longer (closed, and
/// tearing its sessions down for up to [`controller::SHUTDOWN_TIMEOUT`]). Then this waits:
/// when its window appears it is brought forward; when the instance exits, this one takes
/// over and launches. An instance that stays windowless past the wait is left alone.
pub(crate) fn focus_running_instance() -> bool {
    #[cfg(windows)]
    {
        use std::ffi::c_void;
        use std::time::{Duration, Instant};
        const ERROR_ACCESS_DENIED: u32 = 5;
        const ERROR_ALREADY_EXISTS: u32 = 183;
        const SW_RESTORE: i32 = 9;
        const POLL: Duration = Duration::from_millis(200);
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateMutexW(
                attributes: *const c_void,
                initial_owner: i32,
                name: *const u16,
            ) -> isize;
            fn GetLastError() -> u32;
            fn CloseHandle(handle: isize) -> i32;
        }
        #[link(name = "user32")]
        unsafe extern "system" {
            fn FindWindowW(class: *const u16, title: *const u16) -> isize;
            fn IsIconic(window: isize) -> i32;
            fn ShowWindow(window: isize, cmd: i32) -> i32;
            fn SetForegroundWindow(window: isize) -> i32;
        }
        if settings::demo_mode() {
            return false;
        }
        let wide = |s: &str| {
            s.encode_utf16()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>()
        };
        let name = wide("Local\\Zephon.SingleInstance");
        // GPUI's window class (`WINDOW_CLASS_NAME` in gpui-pre-windows' window.rs), so a
        // folder open in Explorer under the same title is never taken for the app.
        let class = wide("Zed::Window");
        let title = wide(APP_TITLE);
        let wait = controller::SHUTDOWN_TIMEOUT + Duration::from_secs(2);
        let give_up = Instant::now() + wait;
        loop {
            // SAFETY: NUL-terminated name, default security. GetLastError is read right
            // after, before any other call can change it.
            let (mutex, err) = unsafe {
                let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
                (h, GetLastError())
            };
            match (mutex, err) {
                // Ours: it marks this process as the running instance until it exits, so the
                // handle is never closed.
                (h, e) if h != 0 && e != ERROR_ALREADY_EXISTS => return false,
                // Another instance holds it. Close our handle, or the mutex would outlive that
                // instance and this loop would never see it exit.
                (h, _) if h != 0 => {
                    // SAFETY: `h` is the handle CreateMutexW just gave us, closed once.
                    unsafe { CloseHandle(h) };
                }
                // An elevated instance's mutex cannot be opened from here: it is running.
                (_, ERROR_ACCESS_DENIED) => {}
                // No mutex at all (should not happen): launch rather than refuse to start.
                _ => return false,
            }
            // SAFETY: NUL-terminated class and title; the calls only act on a window handle
            // Windows gave us.
            unsafe {
                let window = FindWindowW(class.as_ptr(), title.as_ptr());
                if window != 0 {
                    if IsIconic(window) != 0 {
                        ShowWindow(window, SW_RESTORE);
                    }
                    SetForegroundWindow(window);
                    return true;
                }
            }
            if Instant::now() >= give_up {
                return true;
            }
            std::thread::sleep(POLL);
        }
    }
    #[cfg(not(windows))]
    false
}
