//! "Start with Windows": the `Zephon` value under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, holding this exe's quoted path.
//! The registry is the only record; the GUI reads it each time the settings window opens.
//! Demo mode never reads or writes it: the switch shows off and changing it does nothing.

use crate::firewall::exe_path;
use crate::winreg;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE_NAME: &str = "Zephon";

#[derive(Debug, Clone, PartialEq)]
pub enum Autostart {
    Off,
    /// Starts this exe.
    On,
    /// Starts another copy (for example `target\release` while this is `target\debug`).
    OtherCopy(String),
}

/// The Run value for this exe: its path in quotes.
pub fn command_line() -> String {
    format!("\"{}\"", exe_path().display())
}

pub fn state() -> Autostart {
    if crate::settings::demo_mode() {
        return Autostart::Off;
    }
    match winreg::read_string(RUN_KEY, VALUE_NAME) {
        None => Autostart::Off,
        Some(v) if v.trim().eq_ignore_ascii_case(&command_line()) => Autostart::On,
        Some(v) => Autostart::OtherCopy(v.trim().trim_matches('"').to_string()),
    }
}

/// Adds (pointing at this exe) or removes the Run value. Does nothing in demo mode.
pub fn set(on: bool) -> Result<(), String> {
    if crate::settings::demo_mode() {
        return Ok(());
    }
    let r = if on {
        winreg::set_string(RUN_KEY, VALUE_NAME, &command_line())
    } else {
        winreg::delete_value(RUN_KEY, VALUE_NAME)
    };
    match &r {
        Ok(()) => tracing::info!("Start with Windows: {}", if on { "on" } else { "off" }),
        Err(e) => tracing::warn!("Could not change Start with Windows: {e}"),
    }
    r
}
