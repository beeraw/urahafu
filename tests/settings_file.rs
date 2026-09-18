//! Integration tests for settings file I/O against real temporary files (never mocked), driving
//! [`urahafu::core::settings::Settings`] the way the platform layer will: load, mutate, save,
//! reload.

// This whole file is test code (an integration test binary), so the crate's `unwrap`/`expect`
// ban — meant to keep failure handling explicit in the library — does not apply: a panic here is
// exactly the right way to fail a test on an unexpected I/O error.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use urahafu::core::color::CleaningColor;
use urahafu::core::failsafe::FailsafeDelay;
use urahafu::core::settings::Settings;

/// A unique temp directory under `std::env::temp_dir()`, removed on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("urahafu-integration-{tag}-{nanos}"));
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
fn default_path_layout_matches_documented_location() {
    let dir = TempDir::new("default-path-layout");
    let path = Settings::default_path(dir.path());
    assert_eq!(
        path,
        dir.path()
            .join("Library/Application Support/Urahafu/settings.conf")
    );
}

#[test]
fn full_first_run_flow_creates_defaults_then_persists_changes() {
    let dir = TempDir::new("first-run-flow");
    let path = Settings::default_path(dir.path());

    // First run: nothing on disk yet, the app should just use defaults without erroring.
    assert!(!path.exists());
    let settings = Settings::load(&path).expect("load should succeed even without a file");
    assert_eq!(settings, Settings::default());

    // The user flips a couple of settings; the app saves them.
    let updated = Settings {
        color: CleaningColor::White,
        failsafe: FailsafeDelay::Seconds30,
        keyboard_only: true,
        show_menu_bar_icon: false,
    };
    updated
        .save(&path)
        .expect("save should succeed, creating the Urahafu directory");
    assert!(path.exists());

    // A later launch reloads exactly what was saved.
    let reloaded = Settings::load(&path).expect("load should succeed");
    assert_eq!(reloaded, updated);
}

#[test]
fn hand_edited_file_with_one_bad_line_keeps_the_rest() {
    let dir = TempDir::new("hand-edited");
    let path = dir.path().join("settings.conf");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "color = white\nfailsafe_seconds = not-a-number\nkeyboard_only = true\n",
    )
    .unwrap();

    let settings = Settings::load(&path).expect("a malformed value must never fail the whole load");
    assert_eq!(settings.color, CleaningColor::White);
    assert_eq!(
        settings.failsafe,
        FailsafeDelay::Seconds60,
        "invalid failsafe_seconds falls back to the default"
    );
    assert!(settings.keyboard_only);
}

#[test]
fn concurrent_style_overwrite_never_leaves_a_torn_file() {
    let dir = TempDir::new("concurrent-overwrite");
    let path = dir.path().join("settings.conf");

    // Simulate several rapid successive saves (e.g. the user flipping several menu toggles in a
    // row); each one must fully replace the previous file, never partially.
    for (color, keyboard_only) in [
        (CleaningColor::Black, false),
        (CleaningColor::White, true),
        (CleaningColor::Black, true),
        (CleaningColor::White, false),
    ] {
        let settings = Settings {
            color,
            keyboard_only,
            ..Settings::default()
        };
        settings.save(&path).expect("save should succeed");
        let reloaded = Settings::load(&path).expect("load should succeed");
        assert_eq!(reloaded, settings);
    }
}

#[test]
fn save_is_atomic_no_leftover_temp_file_survives() {
    let dir = TempDir::new("atomic-no-leftover");
    let path = dir.path().join("nested").join("settings.conf");

    Settings::default()
        .save(&path)
        .expect("save should succeed, creating nested parent dirs");

    let entries: Vec<_> = fs::read_dir(path.parent().unwrap())
        .expect("read parent dir")
        .map(|entry| entry.expect("dir entry").file_name())
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from("settings.conf")],
        "no .tmp file should remain after save"
    );
}
