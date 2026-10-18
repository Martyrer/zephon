//! Zephon (alpha): streams Windows system audio to AirPlay 2 receivers.
//!
//! The headless core is public: `settings`, `capture`, `level`, `discovery`, `session`,
//! `firewall`, `advice` and `autostart`, with `controller` between the core and the gpui-kit window in
//! the private `ui`. The executable (`src/main.rs`) only calls [`run`].

pub mod advice;
pub mod autostart;
pub mod capture;
pub mod controller;
pub mod discovery;
pub mod firewall;
pub mod level;
pub mod session;
pub mod settings;

mod cli;
mod logging;
mod single_instance;
mod ui;
mod winreg;

/// The executable's `main`: reads the process arguments, may exit the process (`--help`,
/// argument errors) and installs the global logger, so call it once and only from `main`.
#[doc(hidden)]
pub use cli::run;

/// The main window's title. A second launch finds the running copy's window by it.
pub const APP_TITLE: &str = "Zephon (alpha)";
