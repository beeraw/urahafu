//! Accessibility trust check (`ApplicationServices`/`HIServices`), used by the Safety net
//! preflight (`docs/ARCHITECTURE.md`) before ever installing an event tap.

/// Whether the process is currently trusted for Accessibility (i.e. `AXIsProcessTrusted`).
/// Never prompts the user; [`crate::platform::permission::open_accessibility_settings`] is
/// responsible for sending them to the right System Settings pane.
#[must_use]
pub fn is_trusted() -> bool {
    // SAFETY: `AXIsProcessTrusted` takes no arguments and has no preconditions; it is safe to
    // call from any thread at any time.
    unsafe { AXIsProcessTrusted() }
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}
