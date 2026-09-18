//! Accessibility trust check (`ApplicationServices`/`HIServices`), used by the Safety net
//! preflight (`docs/ARCHITECTURE.md`) before ever installing an event tap.

use core_foundation::base::TCFType;
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::CFString;

/// Whether the process is currently trusted for Accessibility (i.e. `AXIsProcessTrusted`).
/// Never prompts the user; [`crate::platform::permission::open_accessibility_settings`] is
/// responsible for sending them to the right System Settings pane.
#[must_use]
pub fn is_trusted() -> bool {
    // SAFETY: `AXIsProcessTrusted` takes no arguments and has no preconditions; it is safe to
    // call from any thread at any time.
    unsafe { AXIsProcessTrusted() }
}

/// Asks macOS to list Urahafu under System Settings › Privacy & Security › Accessibility (switched
/// off) and to show its own "allow this app?" prompt, by calling `AXIsProcessTrustedWithOptions`
/// with `kAXTrustedCheckOptionPrompt`. A plain `AXIsProcessTrusted` never adds the app to that list,
/// so without this call a user who removed a stale entry would find nothing to switch on. Returns
/// the current trust state, like [`is_trusted`].
#[must_use]
pub fn request_trust() -> bool {
    // The documented value of `kAXTrustedCheckOptionPrompt`; building the key from its string
    // avoids reading the framework's `extern` static.
    let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
    let options = CFDictionary::from_CFType_pairs(&[(key, CFBoolean::true_value())]);
    // SAFETY: `options` is a valid, live `CFDictionary` for the whole call (it is dropped only
    // after it returns); the function reads it without taking ownership and has no other
    // precondition.
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
}
