//! The `zephon` executable. Everything but the subsystem setting lives in the library
//! (`src/lib.rs`); [`zephon::run`] parses the command line.
//!
//! The exe uses the Windows GUI subsystem, so a no-argument launch opens no console window.
//! CLI subcommands attach to the parent terminal's console to print their output.

#![windows_subsystem = "windows"]

use std::process::ExitCode;

fn main() -> ExitCode {
    zephon::run()
}
