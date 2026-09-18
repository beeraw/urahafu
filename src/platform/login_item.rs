//! "Open at Login" (DESIGN.md §6, `docs/ARCHITECTURE.md` "Settings"): writes/removes the
//! `LaunchAgent` plist at [`crate::core::login_item::launch_agent_path`]. The plist content
//! itself is built by the pure `core::login_item` module; this file only touches the filesystem.
//!
//! Login item state is not a setting — it *is* whether this file exists — so there is no
//! `enabled`/`disabled` value to persist anywhere else, and no `launchctl` call is needed either:
//! `RunAtLoad` starts the app the next time this `LaunchAgent` is loaded, which happens at the
//! next login on its own.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::login_item::{launch_agent_path, launch_agent_plist};

/// Errors from enabling/disabling/checking the login item.
#[derive(Debug, thiserror::Error)]
pub enum LoginItemError {
    /// Creating the `LaunchAgents` directory failed.
    #[error("failed to create {path}: {source}")]
    CreateDir {
        /// Directory that failed to be created.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Writing the temporary plist (for an atomic install) failed.
    #[error("failed to write {path}: {source}")]
    Write {
        /// Path that failed to be written.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Renaming the temporary plist into place failed.
    #[error("failed to finalize {path}: {source}")]
    Rename {
        /// Final path the rename target was.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Removing the plist failed for a reason other than "it doesn't exist".
    #[error("failed to remove {path}: {source}")]
    Remove {
        /// Path that failed to be removed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// Whether the login item is currently enabled: whether the `LaunchAgent` plist exists at `home`.
#[must_use]
pub fn is_enabled(home: &Path) -> bool {
    launch_agent_path(home).is_file()
}

/// Enables the login item: writes the `LaunchAgent` plist that will launch `executable_path` at
/// the next login, atomically (write to a sibling temp file, then rename into place).
///
/// # Errors
///
/// Returns a [`LoginItemError`] if creating the `LaunchAgents` directory, writing the temporary
/// file, or renaming it fails.
pub fn enable(home: &Path, executable_path: &Path) -> Result<(), LoginItemError> {
    let path = launch_agent_path(home);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir).map_err(|source| LoginItemError::CreateDir {
        path: dir.to_path_buf(),
        source,
    })?;

    let tmp_path = tmp_path_for(&path);
    fs::write(&tmp_path, launch_agent_plist(executable_path)).map_err(|source| {
        LoginItemError::Write {
            path: tmp_path.clone(),
            source,
        }
    })?;

    fs::rename(&tmp_path, &path).map_err(|source| LoginItemError::Rename { path, source })
}

/// Disables the login item: removes the `LaunchAgent` plist, if any. Not being enabled in the
/// first place is not an error.
///
/// # Errors
///
/// Returns [`LoginItemError::Remove`] if the plist exists but could not be removed.
pub fn disable(home: &Path) -> Result<(), LoginItemError> {
    let path = launch_agent_path(home);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(LoginItemError::Remove { path, source }),
    }
}

/// Sibling temporary-file path for an atomic write of `path`.
fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().map_or_else(
        || "login_item.plist".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    name.push_str(".tmp");
    path.with_file_name(name)
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
                "urahafu-test-login-item-{tag}-{nanos}-{:?}",
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
    fn disabled_by_default() {
        let dir = TempDir::new("disabled-default");
        assert!(!is_enabled(dir.path()));
    }

    #[test]
    fn enable_then_is_enabled() {
        let dir = TempDir::new("enable");
        enable(
            dir.path(),
            Path::new("/Applications/Urahafu.app/Contents/MacOS/urahafu"),
        )
        .expect("enable should succeed");
        assert!(is_enabled(dir.path()));
        let path = launch_agent_path(dir.path());
        assert!(path.is_file());
        let contents = fs::read_to_string(&path).expect("read plist");
        assert!(contents.contains("RunAtLoad"));
        assert!(!tmp_path_for(&path).exists());
    }

    #[test]
    fn enable_then_disable_removes_plist() {
        let dir = TempDir::new("enable-disable");
        let exe = Path::new("/Applications/Urahafu.app/Contents/MacOS/urahafu");
        enable(dir.path(), exe).expect("enable should succeed");
        assert!(is_enabled(dir.path()));

        disable(dir.path()).expect("disable should succeed");
        assert!(!is_enabled(dir.path()));
    }

    #[test]
    fn disable_when_not_enabled_is_not_an_error() {
        let dir = TempDir::new("disable-noop");
        disable(dir.path()).expect("disabling a never-enabled login item should be a no-op");
    }

    #[test]
    fn enable_overwrites_existing_plist() {
        let dir = TempDir::new("enable-overwrite");
        enable(dir.path(), Path::new("/Applications/Old.app/urahafu")).expect("first enable");
        enable(
            dir.path(),
            Path::new("/Applications/Urahafu.app/Contents/MacOS/urahafu"),
        )
        .expect("second enable");

        let contents = fs::read_to_string(launch_agent_path(dir.path())).expect("read plist");
        assert!(contents.contains("/Applications/Urahafu.app/Contents/MacOS/urahafu"));
        assert!(!contents.contains("/Applications/Old.app/urahafu"));
    }
}
