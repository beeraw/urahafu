//! Accessibility permission: checking it (no prompt) and sending the user to the right System
//! Settings pane (DESIGN.md §10-§11).

use std::process::Command;

use crate::platform::ffi::accessibility;
use crate::platform::system::SystemError;

/// Whether Urahafu is currently trusted for Accessibility. Never prompts the user — the OS-level
/// "allow this app?" prompt is only shown once, the first time an event tap is actually created;
/// this is purely a read.
#[must_use]
pub fn is_trusted() -> bool {
    accessibility::is_trusted()
}

/// Opens System Settings directly on the Accessibility privacy pane (DESIGN.md §10 "Open System
/// Settings").
///
/// # Errors
///
/// Returns [`SystemError::Open`] if `/usr/bin/open` could not be spawned.
pub fn open_accessibility_settings() -> Result<(), SystemError> {
    const URL: &str =
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
    Command::new("/usr/bin/open")
        .arg(URL)
        .status()
        .map_err(|source| SystemError::Open {
            url: URL.to_owned(),
            source,
        })?;
    Ok(())
}
