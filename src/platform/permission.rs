//! Accessibility permission: checking it (no prompt) and sending the user to the right System
//! Settings pane (DESIGN.md §10-§11).

use std::process::Command;

use crate::platform::ffi::accessibility;
use crate::platform::system::SystemError;

/// Whether Urahafu is currently trusted for Accessibility. A pure read that never prompts: the
/// system prompt comes from [`open_accessibility_settings`], when the user asks for access.
#[must_use]
pub fn is_trusted() -> bool {
    accessibility::is_trusted()
}

/// Opens System Settings directly on the Accessibility privacy pane (DESIGN.md §10 "Open System
/// Settings"), after asking macOS to list Urahafu there
/// ([`crate::platform::ffi::accessibility::request_trust`]).
///
/// # Errors
///
/// Returns [`SystemError::Open`] if `/usr/bin/open` could not be spawned.
pub fn open_accessibility_settings() -> Result<(), SystemError> {
    const URL: &str =
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

    // Makes sure Urahafu is listed in the pane about to open (with the system's own prompt), so
    // there is always a switch to turn on.
    let _ = accessibility::request_trust();

    Command::new("/usr/bin/open")
        .arg(URL)
        .status()
        .map_err(|source| SystemError::Open {
            url: URL.to_owned(),
            source,
        })?;
    Ok(())
}
