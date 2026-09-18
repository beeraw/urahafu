//! Persisted user settings (DESIGN.md §6, `docs/ARCHITECTURE.md` "Settings").
//!
//! Stored as `key = value` lines in
//! `~/Library/Application Support/Urahafu/settings.conf`. Parsing is designed to never fail:
//! unknown keys are ignored, and any invalid value for a known key falls back to that key's
//! default rather than rejecting the whole file. This means a hand-edited or corrupted settings
//! file can never prevent the app from starting.
//!
//! Login item state is deliberately not stored here: per `docs/ARCHITECTURE.md`, it is derived
//! from whether the `LaunchAgent` plist exists on disk (see [`crate::core::login_item`]).

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::core::color::CleaningColor;
use crate::core::failsafe::FailsafeDelay;

/// Directory (relative to the user's home) that holds the settings file.
const APP_SUPPORT_SUBPATH: &str = "Library/Application Support/Urahafu";
/// File name of the settings file within that directory.
const SETTINGS_FILE_NAME: &str = "settings.conf";

/// Errors that can occur while loading or saving the settings file.
///
/// Note that a malformed *file* never produces an error — see the module docs — this type only
/// covers actual I/O failures (permissions, disk full, etc).
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// Reading the settings file failed for a reason other than "it doesn't exist yet".
    #[error("failed to read settings file at {path}: {source}")]
    Read {
        /// Path that failed to be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Creating the settings directory failed.
    #[error("failed to create settings directory at {path}: {source}")]
    CreateDir {
        /// Directory path that failed to be created.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Writing the temporary file (for an atomic save) failed.
    #[error("failed to write settings file at {path}: {source}")]
    Write {
        /// Path that failed to be written.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Renaming the temporary file into place failed.
    #[error("failed to finalize settings file at {path}: {source}")]
    Rename {
        /// Final path the rename target was.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

/// The user-configurable settings, all with documented defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Cleaning screen color. Default: [`CleaningColor::Black`].
    pub color: CleaningColor,
    /// Fail-safe auto-unlock delay. Default: 60 s.
    pub failsafe: FailsafeDelay,
    /// Whether keyboard-only mode is active. Default: `false`.
    pub keyboard_only: bool,
    /// Whether to show the menu bar icon. Default: `true`.
    pub show_menu_bar_icon: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            color: CleaningColor::Black,
            failsafe: FailsafeDelay::Seconds60,
            keyboard_only: false,
            show_menu_bar_icon: true,
        }
    }
}

impl Settings {
    /// The default settings file path given the user's home directory. Pure: the platform layer
    /// is responsible for actually locating `home` (e.g. via `$HOME` or `NSHomeDirectory`).
    #[must_use]
    pub fn default_path(home: &Path) -> PathBuf {
        home.join(APP_SUPPORT_SUBPATH).join(SETTINGS_FILE_NAME)
    }

    /// Parses settings from file contents. Never fails: a missing/invalid value for a known key
    /// falls back to that key's default, and unknown keys are ignored. Lines are `key = value`;
    /// blank lines, lines without `=`, and lines starting with `#` are ignored. Whitespace around
    /// keys and values is trimmed, and both `\n` and `\r\n` line endings are accepted.
    #[must_use]
    pub fn parse(contents: &str) -> Self {
        let mut settings = Self::default();

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match key {
                "color" => {
                    if let Some(color) = CleaningColor::from_setting_str(value) {
                        settings.color = color;
                    }
                }
                "failsafe_seconds" => {
                    if let Some(delay) = value
                        .parse::<u64>()
                        .ok()
                        .and_then(FailsafeDelay::from_seconds)
                    {
                        settings.failsafe = delay;
                    }
                }
                "keyboard_only" => {
                    if let Some(b) = parse_bool(value) {
                        settings.keyboard_only = b;
                    }
                }
                "show_menu_bar_icon" => {
                    if let Some(b) = parse_bool(value) {
                        settings.show_menu_bar_icon = b;
                    }
                }
                _ => {}
            }
        }

        settings
    }

    /// Serializes settings to the `key = value` file format.
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        // `write!` to a String never fails; the `unwrap_used` lint would flag `.unwrap()`, so
        // this helper absorbs the (unreachable) error instead.
        let _ = writeln!(out, "color = {}", self.color.as_setting_str());
        let _ = writeln!(out, "failsafe_seconds = {}", self.failsafe.seconds());
        let _ = writeln!(out, "keyboard_only = {}", self.keyboard_only);
        let _ = writeln!(out, "show_menu_bar_icon = {}", self.show_menu_bar_icon);
        out
    }

    /// Loads settings from `path`. A missing file is treated as "use defaults", not an error.
    /// Any content present is parsed with [`Settings::parse`] (which never fails).
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::Read`] if the file exists but cannot be read (e.g. permissions).
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        match fs::read_to_string(path) {
            Ok(contents) => Ok(Self::parse(&contents)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(SettingsError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Saves settings to `path` atomically: writes to a temporary file in the same directory,
    /// then renames it into place, so a concurrent reader (or a crash mid-write) never observes
    /// a half-written file. Creates the parent directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns a [`SettingsError`] variant if creating the directory, writing the temporary
    /// file, or renaming it fails.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(dir).map_err(|source| SettingsError::CreateDir {
            path: dir.to_path_buf(),
            source,
        })?;

        let tmp_path = tmp_path_for(path);
        fs::write(&tmp_path, self.serialize()).map_err(|source| SettingsError::Write {
            path: tmp_path.clone(),
            source,
        })?;

        fs::rename(&tmp_path, path).map_err(|source| SettingsError::Rename {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Builds a sibling temporary-file path for an atomic write of `path`, e.g.
/// `settings.conf` → `settings.conf.tmp`.
fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().map_or_else(
        || SETTINGS_FILE_NAME.to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    name.push_str(".tmp");
    path.with_file_name(name)
}

/// Parses a `true`/`false` boolean setting value, accepting only the exact lowercase forms.
fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A unique temp directory under `std::env::temp_dir()`, cleaned up on drop.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "urahafu-test-{tag}-{nanos}-{:?}",
                std::thread::current().id()
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn default_settings_match_design() {
        let settings = Settings::default();
        assert_eq!(settings.color, CleaningColor::Black);
        assert_eq!(settings.failsafe, FailsafeDelay::Seconds60);
        assert!(!settings.keyboard_only);
        assert!(settings.show_menu_bar_icon);
    }

    #[test]
    fn parse_round_trips_through_serialize() {
        let settings = Settings {
            color: CleaningColor::White,
            failsafe: FailsafeDelay::Seconds30,
            keyboard_only: true,
            show_menu_bar_icon: false,
        };
        let parsed = Settings::parse(&settings.serialize());
        assert_eq!(parsed, settings);
    }

    #[test]
    fn parse_falls_back_to_defaults_for_garbage_values() {
        let contents = "color = purple\nfailsafe_seconds = 12\nkeyboard_only = maybe\nshow_menu_bar_icon = nope\n";
        let settings = Settings::parse(contents);
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn parse_ignores_missing_keys() {
        let settings = Settings::parse("color = white\n");
        assert_eq!(settings.color, CleaningColor::White);
        assert_eq!(settings.failsafe, FailsafeDelay::Seconds60);
    }

    #[test]
    fn parse_ignores_unknown_keys() {
        let settings = Settings::parse("color = white\nfrobnicate = yes\n");
        assert_eq!(settings.color, CleaningColor::White);
    }

    #[test]
    fn parse_empty_file_gives_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
    }

    #[test]
    fn parse_accepts_crlf_line_endings() {
        let contents = "color = white\r\nkeyboard_only = true\r\n";
        let settings = Settings::parse(contents);
        assert_eq!(settings.color, CleaningColor::White);
        assert!(settings.keyboard_only);
    }

    #[test]
    fn parse_tolerates_surrounding_whitespace_and_comments() {
        let contents = "  color   =   white  \n# a comment\n\nkeyboard_only=true\n";
        let settings = Settings::parse(contents);
        assert_eq!(settings.color, CleaningColor::White);
        assert!(settings.keyboard_only);
    }

    #[test]
    fn parse_ignores_lines_without_equals() {
        let settings = Settings::parse("this is not a valid line\ncolor = white\n");
        assert_eq!(settings.color, CleaningColor::White);
    }

    #[test]
    fn default_path_joins_home() {
        let home = Path::new("/Users/example");
        let path = Settings::default_path(home);
        assert_eq!(
            path,
            Path::new("/Users/example/Library/Application Support/Urahafu/settings.conf")
        );
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = TempDir::new("load-missing");
        let path = dir.path().join("nonexistent").join("settings.conf");
        let settings = Settings::load(&path).expect("load should not fail for a missing file");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new("save-load");
        let path = dir.path().join("nested").join("settings.conf");

        let settings = Settings {
            color: CleaningColor::White,
            failsafe: FailsafeDelay::Seconds90,
            keyboard_only: true,
            show_menu_bar_icon: false,
        };
        settings
            .save(&path)
            .expect("save should succeed, creating parent dirs");
        assert!(path.exists());

        let loaded = Settings::load(&path).expect("load should succeed");
        assert_eq!(loaded, settings);
    }

    #[test]
    fn save_creates_parent_directory() {
        let dir = TempDir::new("save-creates-dir");
        let path = dir
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("settings.conf");
        Settings::default()
            .save(&path)
            .expect("save should create nested parent dirs");
        assert!(path.parent().unwrap().is_dir());
    }

    #[test]
    fn save_does_not_leave_a_temp_file_behind() {
        let dir = TempDir::new("save-no-tmp-leftover");
        let path = dir.path().join("settings.conf");
        Settings::default()
            .save(&path)
            .expect("save should succeed");
        assert!(!tmp_path_for(&path).exists());
        assert!(path.exists());
    }

    #[test]
    fn parse_ignores_the_removed_welcome_seen_key() {
        // A settings file written by an older Urahafu still has this key; the loader must keep
        // ignoring it rather than failing (DESIGN.md "Settings window" › "Removed": the loader
        // silently ignores it, so old files keep loading).
        let settings = Settings::parse("color = white\nwelcome_seen = true\n");
        assert_eq!(settings.color, CleaningColor::White);
    }

    #[test]
    fn save_overwrites_existing_file_atomically() {
        let dir = TempDir::new("save-overwrite");
        let path = dir.path().join("settings.conf");

        Settings::default().save(&path).expect("first save");
        let updated = Settings {
            color: CleaningColor::White,
            ..Settings::default()
        };
        updated.save(&path).expect("second save");

        let loaded = Settings::load(&path).expect("load");
        assert_eq!(loaded.color, CleaningColor::White);
    }
}
