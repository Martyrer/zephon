//! The log file (`%APPDATA%\Zephon\log.txt`, debug level) and the console log (stderr).

use std::path::Path;

use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

use crate::settings;

/// The log is append-only and gets a stats line every 5 s per playing room, so past this
/// size it is started afresh at launch.
const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Renames a log over `max` bytes to `log.old.txt`, replacing an older one.
fn rotate_log(path: &Path, max: u64) {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max) {
        let _ = std::fs::rename(path, path.with_extension("old.txt"));
    }
}

pub(crate) fn init_logging(console_level: &str) {
    rotate_log(&settings::log_path(), LOG_MAX_BYTES);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(settings::log_path())
        .ok();
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(f))
            .with_filter(EnvFilter::new("info,zephon=debug"))
    });
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(console_level));
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_filter(console_filter);
    tracing_subscriber::registry()
        .with(file_layer)
        .with(console_layer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_log_is_rotated() {
        let dir = std::env::temp_dir().join(format!("zephon-test-{}-log", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (log, old) = (dir.join("log.txt"), dir.join("log.old.txt"));
        // Exactly at the limit is kept; one byte over is rotated.
        std::fs::write(&log, "ten bytes!").unwrap();
        rotate_log(&log, 10);
        assert!(log.exists() && !old.exists());
        std::fs::write(&old, "older").unwrap();
        std::fs::write(&log, "eleven byte").unwrap();
        rotate_log(&log, 10);
        assert!(!log.exists());
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "eleven byte");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
