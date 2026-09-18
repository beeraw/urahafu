//! Secure Input check (Carbon/HIToolbox), used by the Safety net preflight
//! (`docs/ARCHITECTURE.md`) before ever installing an event tap: while Secure Input is on (e.g. a
//! password field has focus somewhere), `CGEventTapCreate` for keyboard events silently receives
//! nothing useful, so Urahafu must detect this up front and refuse to start rather than lock the
//! screen with a keyboard block that doesn't actually work.

/// Whether Secure Input is currently enabled system-wide (`IsSecureEventInputEnabled`).
#[must_use]
pub fn is_secure_input_enabled() -> bool {
    // SAFETY: `IsSecureEventInputEnabled` takes no arguments and has no preconditions; it is safe
    // to call from any thread at any time.
    unsafe { IsSecureEventInputEnabled() }
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}
