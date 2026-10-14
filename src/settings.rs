//! Settings remembered between runs, stored as JSON in `%APPDATA%\Zephon\settings.json`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::de::{DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize};

use crate::session::LatencyProfile;

/// Demo mode, for screenshots: `ZEPHON_DEMO` is set to anything but "" or "0". The app then
/// keeps its files in a scratch folder (see [`app_dir`]) and shows made-up rooms.
pub fn demo_mode() -> bool {
    static DEMO: OnceLock<bool> = OnceLock::new();
    *DEMO.get_or_init(|| std::env::var_os("ZEPHON_DEMO").is_some_and(|v| !v.is_empty() && v != "0"))
}

/// `%APPDATA%\Zephon`, created on first use. In demo mode `%TEMP%\zephon-demo` instead, so
/// screenshots never read or change the real settings, receiver cache or log.
pub fn app_dir() -> PathBuf {
    let dir = if demo_mode() {
        std::env::temp_dir().join("zephon-demo")
    } else {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Zephon")
    };
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn settings_path() -> PathBuf {
    app_dir().join("settings.json")
}

pub fn log_path() -> PathBuf {
    app_dir().join("log.txt")
}

/// Our own receiver cache (not Nyx's `%APPDATA%\nyx-refrain\devices.toml`).
pub fn receiver_cache_path() -> PathBuf {
    app_dir().join("receivers.toml")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Capture endpoint ID (`{0.0.0.00000000}.{GUID}`); opened first.
    pub capture_device_id: Option<String>,
    /// Capture endpoint friendly name; fallback when the ID no longer exists.
    pub capture_device_name: Option<String>,
    /// Latency profile for receivers without their own choice (new receivers). Pinned onto a
    /// receiver the first time it is switched on.
    #[serde(deserialize_with = "lenient")]
    pub default_profile: LatencyProfile,
    /// Start volume (0-100) of a receiver switched on for the first time.
    #[serde(deserialize_with = "default_volume_or_default")]
    pub default_volume: f32,
    /// The global profile of older settings files (before per-receiver profiles). Read once,
    /// moved onto the ticked receivers by [`Settings::migrate`], never written back.
    #[serde(rename = "profile", skip_serializing, deserialize_with = "lenient")]
    legacy_profile: Option<LatencyProfile>,
    /// Pin sockets and mDNS to this network interface (e.g. "Ethernet"); `None` = automatic.
    pub interface: Option<String>,
    /// Per-receiver choices, keyed by receiver name.
    pub receivers: BTreeMap<String, ReceiverPrefs>,
    /// The GUI was playing when it last stopped or closed (for `resume_on_launch`).
    pub was_playing: bool,
    /// GUI: play again on launch when `was_playing`, once the rooms are found.
    pub resume_on_launch: bool,
    /// Volume changes made on the receiver (phone, Home app) move the slider. Off: they are
    /// ignored and the app's slider is the only control.
    pub volume_follows_receiver: bool,
    /// GUI color theme.
    #[serde(deserialize_with = "lenient")]
    pub theme: ThemeChoice,
    /// GUI window material.
    #[serde(deserialize_with = "lenient")]
    pub backdrop: Backdrop,
    /// GUI font family; `None` = Segoe UI Variable (or Segoe UI). A family that is no longer
    /// installed falls back to the default.
    pub font_family: Option<String>,
    /// GUI text size.
    #[serde(deserialize_with = "lenient")]
    pub text_size: TextSize,
    /// GUI accent color.
    #[serde(deserialize_with = "lenient")]
    pub accent: AccentChoice,
    /// The file exists but could not be read: these are defaults, and saving them would
    /// overwrite the user's real settings, so nothing is saved this run.
    #[serde(skip)]
    save_blocked: bool,
}

/// [`Settings::default_volume`] unless changed.
pub const DEFAULT_NEW_ROOM_VOLUME: f32 = 50.0;

impl Default for Settings {
    fn default() -> Self {
        Self {
            capture_device_id: None,
            capture_device_name: None,
            default_profile: LatencyProfile::default(),
            default_volume: DEFAULT_NEW_ROOM_VOLUME,
            legacy_profile: None,
            interface: None,
            receivers: BTreeMap::new(),
            was_playing: false,
            resume_on_launch: false,
            volume_follows_receiver: true,
            theme: ThemeChoice::default(),
            backdrop: Backdrop::default(),
            font_family: None,
            text_size: TextSize::default(),
            accent: AccentChoice::default(),
            save_blocked: false,
        }
    }
}

/// GUI color theme. `System` follows the Windows light/dark setting live.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

/// GUI window material. Mica needs Windows 11 22H2 (build 22621); the GUI falls back to
/// acrylic, then to an opaque background, when the requested one is not available.
/// Acrylic is the default: it is the only one that shows the windows behind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backdrop {
    Mica,
    #[default]
    Acrylic,
    Solid,
}

/// GUI text size: scales the type scale and gpui-kit's rem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    Small,
    #[default]
    Default,
    Large,
}

impl TextSize {
    pub fn scale(self) -> f32 {
        match self {
            Self::Small => 0.92,
            Self::Default => 1.0,
            Self::Large => 1.15,
        }
    }
}

/// GUI accent color: the app's Lagoon, or the Windows accent (adjusted for contrast).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccentChoice {
    #[default]
    Lagoon,
    Windows,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReceiverPrefs {
    /// Included in streaming (the checkbox).
    pub selected: bool,
    /// This receiver's latency profile; `None` = [`Settings::default_profile`].
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub profile: Option<LatencyProfile>,
    /// Last volume (0-100) set from this app, or reported by the receiver while followed;
    /// `None` until the receiver is first switched on.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "volume_or_unset"
    )]
    pub volume: Option<f32>,
    /// Left out of the GUI's Rooms list (and never playing).
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// A field value we do not understand (a choice added by a newer version, a hand edit) falls
/// back to the field's default instead of failing the whole file, which would reset every
/// room's preferences on the next save.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(d)?;
    Ok(T::deserialize(&value).unwrap_or_else(|e| {
        tracing::warn!("Settings: ignoring {value} ({e})");
        T::default()
    }))
}

/// A usable volume: 0-100. NaN would pass every clamp and play at full volume.
fn is_volume(pct: f32) -> bool {
    (0.0..=100.0).contains(&pct)
}

fn volume_or_unset<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    Ok(lenient::<_, Option<f32>>(d)?.filter(|v| is_volume(*v)))
}

/// serde_json writes a non-finite float as `null`; that, too, reads back as the default.
fn default_volume_or_default<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    Ok(volume_or_unset(d)?.unwrap_or(DEFAULT_NEW_ROOM_VOLUME))
}

/// Wait before reading the settings file a second time.
const READ_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);

/// Reads `path` with `read`, once more after [`READ_RETRY_DELAY`] when that fails for any
/// reason but the file not existing.
fn read_retrying(
    path: &Path,
    mut read: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> std::io::Result<Vec<u8>> {
    read(path).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            return Err(e);
        }
        std::thread::sleep(READ_RETRY_DELAY);
        read(path)
    })
}

/// `path` with `suffix` added to the file name (`settings.json` -> `settings.json.tmp`).
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = OsString::from(path.as_os_str());
    s.push(suffix);
    PathBuf::from(s)
}

/// Writes a temporary file next to `path`, then renames it over `path` (`rename` replaces
/// the target on Windows too). A crash mid-write leaves the old file or the new one, never
/// half of one.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = with_suffix(path, ".tmp");
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

impl Settings {
    /// Loads settings; a missing or unreadable file gives defaults (logged).
    pub fn load() -> Self {
        Self::load_from(&settings_path())
    }

    /// A file that does not parse is moved aside to `settings.json.bad` (replacing an older
    /// one), so the next save cannot destroy what may still be recovered by hand; if it cannot
    /// be moved, saves are blocked for this run instead. A file that
    /// cannot be read at all (locked by a backup or antivirus scan, say) is tried once more;
    /// if it still fails, these defaults are never saved, so the file is left as it is.
    fn load_from(path: &Path) -> Self {
        let bytes = match read_retrying(path, |p| std::fs::read(p)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!(
                    "Could not read settings {}: {e}; using defaults and not saving this run, so the file is kept",
                    path.display()
                );
                return Self {
                    save_blocked: true,
                    ..Self::default()
                };
            }
        };
        match serde_json::from_slice::<Self>(&bytes) {
            Ok(mut s) => {
                if s.migrate() {
                    s.save_to(path);
                }
                s
            }
            Err(e) => {
                let bad = with_suffix(path, ".bad");
                match std::fs::rename(path, &bad) {
                    Ok(()) => {
                        tracing::warn!(
                            "Unreadable settings {} ({e}); moved to {}, using defaults",
                            path.display(),
                            bad.display()
                        );
                        Self::default()
                    }
                    // The file is still in place and the only copy, so it must not be
                    // overwritten by the defaults.
                    Err(move_err) => {
                        tracing::warn!(
                            "Unreadable settings {} ({e}); using defaults and not saving this run. \
                             Could not move it aside: {move_err}",
                            path.display()
                        );
                        Self {
                            save_blocked: true,
                            ..Self::default()
                        }
                    }
                }
            }
        }
    }

    /// Older files had one global `profile`. It was last used with the receivers that are
    /// ticked, so they keep it as their own profile; every other receiver gets the default
    /// (`balanced`). Returns true if anything changed.
    fn migrate(&mut self) -> bool {
        let Some(old) = self.legacy_profile.take() else {
            return false;
        };
        let mut moved = Vec::new();
        for (name, r) in self.receivers.iter_mut() {
            if r.selected && r.profile.is_none() {
                r.profile = Some(old);
                moved.push(name.clone());
            }
        }
        tracing::info!(
            "Settings: global profile {} moved to ticked receivers {moved:?}; default for others is {}",
            old.name(),
            self.default_profile.name()
        );
        true
    }

    pub fn save(&self) {
        self.save_to(&settings_path());
    }

    fn save_to(&self, path: &Path) {
        if self.save_blocked {
            return;
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = write_atomic(path, text.as_bytes()) {
                    tracing::warn!("Could not save settings to {}: {e}", path.display());
                }
            }
            Err(e) => tracing::warn!("Could not serialize settings: {e}"),
        }
    }

    pub fn is_selected(&self, receiver: &str) -> bool {
        self.receivers.get(receiver).is_some_and(|r| r.selected)
    }

    pub fn set_selected(&mut self, receiver: &str, selected: bool) {
        self.receivers
            .entry(receiver.to_string())
            .or_default()
            .selected = selected;
    }

    /// The receiver's own latency profile, else the default.
    pub fn profile_for(&self, receiver: &str) -> LatencyProfile {
        self.receivers
            .get(receiver)
            .and_then(|r| r.profile)
            .unwrap_or(self.default_profile)
    }

    pub fn set_profile(&mut self, receiver: &str, profile: LatencyProfile) {
        self.receivers
            .entry(receiver.to_string())
            .or_default()
            .profile = Some(profile);
    }

    pub fn is_hidden(&self, receiver: &str) -> bool {
        self.receivers.get(receiver).is_some_and(|r| r.hidden)
    }

    /// Hides or shows a receiver; hiding also switches it off.
    pub fn set_hidden(&mut self, receiver: &str, hidden: bool) {
        let r = self.receivers.entry(receiver.to_string()).or_default();
        r.hidden = hidden;
        if hidden {
            r.selected = false;
        }
    }

    /// The receiver's saved volume (0-100), if it has one.
    pub fn volume_of(&self, receiver: &str) -> Option<f32> {
        self.receivers.get(receiver).and_then(|r| r.volume)
    }

    /// Ignores a non-finite `pct` (clamping would keep a NaN).
    pub fn set_volume(&mut self, receiver: &str, pct: f32) {
        if !pct.is_finite() {
            return;
        }
        let r = self.receivers.entry(receiver.to_string()).or_default();
        r.volume = Some(pct.clamp(0.0, 100.0));
    }

    /// Switching a receiver on for the first time pins the new-room defaults onto it: the
    /// default profile (unless it has its own) and the default volume, so later changes to
    /// the defaults leave it alone. Returns the volume to start at when it had none saved.
    pub fn first_switch_on(&mut self, receiver: &str) -> Option<f32> {
        let volume = if self.default_volume.is_finite() {
            self.default_volume.clamp(0.0, 100.0)
        } else {
            DEFAULT_NEW_ROOM_VOLUME
        };
        let profile = self.default_profile;
        let r = self.receivers.entry(receiver.to_string()).or_default();
        r.profile.get_or_insert(profile);
        if r.volume.is_some() {
            return None;
        }
        r.volume = Some(volume);
        Some(volume)
    }

    /// Changes the default profile. Receivers that are on and follow the default keep the
    /// profile they play with now.
    pub fn set_default_profile(&mut self, profile: LatencyProfile) {
        let old = self.default_profile;
        for r in self.receivers.values_mut() {
            if r.selected && r.profile.is_none() {
                r.profile = Some(old);
            }
        }
        self.default_profile = profile;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_per_receiver_falls_back_to_default() {
        let mut s = Settings {
            default_profile: LatencyProfile::Low,
            ..Settings::default()
        };
        assert_eq!(s.profile_for("Kitchen"), LatencyProfile::Low);
        s.set_profile("Living Room", LatencyProfile::Stable);
        assert_eq!(s.profile_for("Living Room"), LatencyProfile::Stable);
        assert_eq!(s.profile_for("Kitchen"), LatencyProfile::Low);
    }

    #[test]
    fn old_global_profile_moves_to_ticked_receivers() {
        let mut s: Settings = serde_json::from_str(
            r#"{"profile":"stable","receivers":{"Kitchen":{"selected":false},"Living Room":{"selected":true},
                "Office":{"selected":true,"profile":"low"}}}"#,
        )
        .unwrap();
        assert!(s.migrate());
        assert_eq!(s.profile_for("Living Room"), LatencyProfile::Stable);
        assert_eq!(s.profile_for("Office"), LatencyProfile::Low);
        assert_eq!(s.profile_for("Kitchen"), LatencyProfile::Balanced);
        assert_eq!(s.profile_for("New Receiver"), LatencyProfile::Balanced);
        // The legacy key is not written back, so the migration runs once.
        let saved = serde_json::to_string(&s).unwrap();
        let json: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(json["default_profile"], "balanced");
        assert!(json.get("profile").is_none());
        let mut again: Settings = serde_json::from_str(&saved).unwrap();
        assert!(!again.migrate());
        assert_eq!(again.profile_for("Living Room"), LatencyProfile::Stable);
    }

    #[test]
    fn old_files_load_with_the_new_defaults() {
        // A file from before the settings window: every new key takes its default.
        let s: Settings = serde_json::from_str(
            r#"{"default_profile":"balanced","receivers":{"Kitchen":{"selected":true}},"theme":"dark","backdrop":"mica"}"#,
        )
        .unwrap();
        assert_eq!(s.default_volume, 50.0);
        assert!(!s.resume_on_launch && !s.was_playing);
        assert!(s.volume_follows_receiver);
        assert_eq!(
            (s.font_family.as_deref(), s.text_size, s.accent),
            (None, TextSize::Default, AccentChoice::Lagoon)
        );
        assert!(!s.is_hidden("Kitchen") && s.volume_of("Kitchen").is_none());
    }

    #[test]
    fn a_missing_file_gives_defaults_that_save() {
        // The first run: nothing to protect, so the first save creates the file.
        let dir = scratch_dir("missing");
        let path = dir.join("settings.json");
        let mut s = Settings::load_from(&path);
        assert_eq!((s.default_volume, s.volume_follows_receiver), (50.0, true));
        assert!(s.receivers.is_empty());
        s.set_selected("Kitchen", true);
        s.save_to(&path);
        assert!(Settings::load_from(&path).is_selected("Kitchen"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_keys_round_trip() {
        let mut s = Settings {
            default_volume: 35.0,
            resume_on_launch: true,
            was_playing: true,
            volume_follows_receiver: false,
            font_family: Some("Bahnschrift".into()),
            text_size: TextSize::Large,
            accent: AccentChoice::Windows,
            ..Settings::default()
        };
        s.set_hidden("Bedroom TV", true);
        s.set_volume("Kitchen", 120.0);
        let json: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(json["text_size"], "large");
        assert_eq!(json["accent"], "windows");
        assert_eq!(json["receivers"]["Bedroom TV"]["hidden"], true);
        // Defaults stay out of the per-receiver entries.
        assert!(json["receivers"]["Kitchen"].get("hidden").is_none());
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(back.volume_of("Kitchen"), Some(100.0));
        assert!(back.is_hidden("Bedroom TV"));
        assert_eq!(back.default_volume, 35.0);
        assert!(back.resume_on_launch && back.was_playing && !back.volume_follows_receiver);
        assert_eq!(back.font_family.as_deref(), Some("Bahnschrift"));
        assert_eq!(
            (back.text_size, back.accent),
            (TextSize::Large, AccentChoice::Windows)
        );
    }

    #[test]
    fn hiding_a_receiver_switches_it_off() {
        let mut s = Settings::default();
        s.set_selected("Kitchen", true);
        s.set_hidden("Kitchen", true);
        assert!(s.is_hidden("Kitchen") && !s.is_selected("Kitchen"));
        s.set_hidden("Kitchen", false);
        assert!(!s.is_hidden("Kitchen") && !s.is_selected("Kitchen"));
    }

    #[test]
    fn first_switch_on_pins_the_new_room_defaults() {
        let mut s = Settings {
            default_volume: 40.0,
            default_profile: LatencyProfile::Stable,
            ..Settings::default()
        };
        assert_eq!(s.first_switch_on("Kitchen"), Some(40.0));
        assert_eq!(s.volume_of("Kitchen"), Some(40.0));
        assert_eq!(s.profile_for("Kitchen"), LatencyProfile::Stable);
        // Once pinned, new defaults and later switch-ons leave the room alone.
        s.default_profile = LatencyProfile::Low;
        s.default_volume = 20.0;
        assert_eq!(s.first_switch_on("Kitchen"), None);
        assert_eq!(s.profile_for("Kitchen"), LatencyProfile::Stable);
        assert_eq!(s.volume_of("Kitchen"), Some(40.0));
        // A room with its own profile keeps it.
        s.set_profile("Living Room", LatencyProfile::Balanced);
        assert_eq!(s.first_switch_on("Living Room"), Some(20.0));
        assert_eq!(s.profile_for("Living Room"), LatencyProfile::Balanced);
    }

    #[test]
    fn changing_the_default_profile_keeps_rooms_that_are_on() {
        let mut s = Settings::default();
        s.set_selected("Kitchen", true); // on, follows the default (balanced)
        s.set_selected("Soundbar", false);
        s.set_default_profile(LatencyProfile::Stable);
        assert_eq!(s.profile_for("Kitchen"), LatencyProfile::Balanced);
        assert_eq!(s.profile_for("Soundbar"), LatencyProfile::Stable);
        assert_eq!(s.profile_for("New Room"), LatencyProfile::Stable);
    }

    #[test]
    fn appearance_round_trips() {
        let s: Settings = serde_json::from_str(r#"{"theme":"dark","backdrop":"mica"}"#).unwrap();
        assert_eq!((s.theme, s.backdrop), (ThemeChoice::Dark, Backdrop::Mica));
        let json: serde_json::Value = serde_json::to_value(&s).unwrap();
        assert_eq!(
            (json["theme"].as_str(), json["backdrop"].as_str()),
            (Some("dark"), Some("mica"))
        );
    }

    #[test]
    fn unknown_values_fall_back_per_field() {
        // As written by a newer version: unknown choices cost only those fields.
        let s: Settings = serde_json::from_str(
            r#"{"theme":"neon","backdrop":"dark","text_size":"huge","accent":5,"default_profile":"ultra",
                "interface":"Ethernet","receivers":{"Kitchen":{"selected":true,"profile":"ultra","volume":40}}}"#,
        )
        .unwrap();
        assert_eq!(
            (s.theme, s.backdrop),
            (ThemeChoice::System, Backdrop::Acrylic)
        );
        assert_eq!(
            (s.text_size, s.accent),
            (TextSize::Default, AccentChoice::Lagoon)
        );
        assert_eq!(s.default_profile, LatencyProfile::Balanced);
        assert_eq!(s.interface.as_deref(), Some("Ethernet"));
        assert!(s.is_selected("Kitchen"));
        assert_eq!(s.receivers["Kitchen"].profile, None);
        assert_eq!(s.volume_of("Kitchen"), Some(40.0));
    }

    #[test]
    fn bad_volumes_load_as_unset() {
        // `null` is how serde_json writes a NaN.
        let s: Settings = serde_json::from_str(
            r#"{"default_volume":null,"receivers":{"A":{"volume":150},"B":{"volume":-1},"C":{"volume":null},
                "D":{"volume":"loud"},"E":{"volume":1e300},"F":{"volume":0}}}"#,
        )
        .unwrap();
        assert_eq!(s.default_volume, DEFAULT_NEW_ROOM_VOLUME);
        for name in ["A", "B", "C", "D", "E"] {
            assert_eq!(s.volume_of(name), None, "{name}");
        }
        assert_eq!(s.volume_of("F"), Some(0.0));
        let s: Settings = serde_json::from_str(r#"{"default_volume":101}"#).unwrap();
        assert_eq!(s.default_volume, DEFAULT_NEW_ROOM_VOLUME);
    }

    #[test]
    fn non_finite_volumes_are_never_stored() {
        let mut s = Settings::default();
        s.set_volume("Kitchen", f32::NAN);
        s.set_volume("Kitchen", f32::INFINITY);
        assert_eq!(s.volume_of("Kitchen"), None);
        s.default_volume = f32::NAN;
        assert_eq!(s.first_switch_on("Kitchen"), Some(DEFAULT_NEW_ROOM_VOLUME));
    }

    /// A fresh scratch folder under `%TEMP%` (never the real `%APPDATA%`).
    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zephon-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unreadable_file_is_moved_aside() {
        let dir = scratch_dir("bad-file");
        let path = dir.join("settings.json");
        let bad = dir.join("settings.json.bad");
        std::fs::write(&bad, "older bad file").unwrap();
        std::fs::write(&path, r#"{"receivers":{"Kitchen":{"selected":tr"#).unwrap();
        let s = Settings::load_from(&path);
        assert!(s.receivers.is_empty());
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(&bad).unwrap(),
            r#"{"receivers":{"Kitchen":{"selected":tr"#
        );
        // A missing file is just defaults; nothing is moved.
        let s = Settings::load_from(&path);
        assert!(s.receivers.is_empty() && bad.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bad_file_that_cannot_be_moved_aside_is_never_overwritten() {
        let dir = scratch_dir("bad-file-stuck");
        let path = dir.join("settings.json");
        // A folder named settings.json.bad, with a file in it, makes the rename fail.
        std::fs::create_dir(dir.join("settings.json.bad")).unwrap();
        std::fs::write(dir.join("settings.json.bad").join("keep"), "x").unwrap();
        let broken = r#"{"receivers":{"Kitchen":{"selected":tr"#;
        std::fs::write(&path, broken).unwrap();
        let mut s = Settings::load_from(&path);
        assert!(s.save_blocked && s.receivers.is_empty());
        s.set_selected("Kitchen", true);
        s.save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_never_overwritten() {
        let dir = scratch_dir("read-error");
        // A folder where the file should be: reading fails, and not with NotFound.
        let path = dir.join("settings.json");
        std::fs::create_dir(&path).unwrap();
        let mut s = Settings::load_from(&path);
        assert!(s.save_blocked && s.receivers.is_empty());
        // Changes stay in memory; nothing is written, wherever it would go.
        s.set_selected("Kitchen", true);
        let other = dir.join("other.json");
        s.save_to(&other);
        assert!(!other.exists());
        assert!(path.is_dir());
        // A readable file saves as usual.
        std::fs::write(&other, "{}").unwrap();
        let mut s = Settings::load_from(&other);
        s.set_selected("Kitchen", true);
        s.save_to(&other);
        assert!(Settings::load_from(&other).is_selected("Kitchen"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_read_that_fails_once_is_tried_again() {
        let mut calls = 0;
        let read = read_retrying(Path::new("settings.json"), |_| {
            calls += 1;
            if calls == 1 {
                Err(std::io::ErrorKind::PermissionDenied.into())
            } else {
                Ok(b"{}".to_vec())
            }
        });
        assert_eq!(read.unwrap(), b"{}");
        assert_eq!(calls, 2);
    }

    #[test]
    fn a_missing_file_is_read_only_once() {
        let mut calls = 0;
        let read = read_retrying(Path::new("settings.json"), |_| {
            calls += 1;
            Err(std::io::ErrorKind::NotFound.into())
        });
        assert_eq!(read.unwrap_err().kind(), std::io::ErrorKind::NotFound);
        assert_eq!(calls, 1);
    }

    #[test]
    fn loading_an_old_file_rewrites_it_without_the_global_profile() {
        let dir = scratch_dir("migrate-on-load");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"profile":"stable","receivers":{"Living Room":{"selected":true}}}"#,
        )
        .unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.profile_for("Living Room"), LatencyProfile::Stable);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(json.get("profile").is_none());
        assert_eq!(json["receivers"]["Living Room"]["profile"], "stable");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_save_that_cannot_write_its_temp_file_leaves_the_old_file_whole() {
        let dir = scratch_dir("atomic-save-fails");
        let path = dir.join("settings.json");
        let old = r#"{"receivers":{"Kitchen":{"selected":true}}}"#;
        std::fs::write(&path, old).unwrap();
        // A folder named settings.json.tmp, with a file in it, makes the temp file fail.
        std::fs::create_dir(dir.join("settings.json.tmp")).unwrap();
        std::fs::write(dir.join("settings.json.tmp").join("keep"), "x").unwrap();
        let mut s = Settings::load_from(&path);
        s.set_selected("Kitchen", false);
        s.save_to(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_replaces_the_file_and_leaves_no_temp_file() {
        let dir = scratch_dir("atomic-save");
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        s.set_selected("Kitchen", true);
        s.save_to(&path);
        s.set_volume("Kitchen", 30.0);
        s.save_to(&path);
        let back = Settings::load_from(&path);
        assert!(back.is_selected("Kitchen"));
        assert_eq!(back.volume_of("Kitchen"), Some(30.0));
        assert!(!dir.join("settings.json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
