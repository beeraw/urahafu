//! Wiring for the app's main menu bar's `NSMenuItem`s (DESIGN.md "App main menu"): the two
//! `unsafe` calls (`setTarget`/`setAction`) `objc2-app-kit` 0.3.2 requires to give
//! an item an action, for the two ways this app's menu items are wired — through the settings
//! window's own [`ActionTarget`] (same target/action mechanism `action_target::wire_control`
//! already uses for every settings-window control), or directly to `NSWindow`'s standard
//! `performClose:` so "Window > Close" behaves like any other Mac app's, following the responder
//! chain to whichever window is key instead of a specific one this crate would have to track.
//!
//! This is one of the designated FFI submodules (`docs/ARCHITECTURE.md` goal 4: every `unsafe`
//! block lives under `src/platform/ffi/`, documented with `// SAFETY:`); the crate root denies
//! `unsafe_code` everywhere else. [`crate::platform::app_menu`] (safe code only) is the only
//! caller.

#![allow(
    unsafe_code,
    reason = "this module wraps the two NSMenuItem setters (setTarget:/setAction:) objc2-app-kit \
              0.3.2 marks unsafe; see docs/ARCHITECTURE.md goal 4"
)]

use objc2::sel;
use objc2_app_kit::NSMenuItem;

use crate::platform::ffi::action_target::ActionTarget;

/// Wires `item`'s target/action to `target`'s `performAction:` (`ActionTarget::perform_action_selector`),
/// exactly like every settings-window control (`action_target::wire_control`). `item`'s `tag`
/// (set separately, with the always-safe `NSMenuItem::setTag`) is how the callback tells menu
/// items apart, same as every other tagged control.
pub fn wire_to_action_target(item: &NSMenuItem, target: &ActionTarget) {
    // SAFETY: `target` is a live `ActionTarget`, which implements `performAction:` with the
    // `(id sender) -> void` signature `NSMenuItem`'s target/action mechanism requires — the same
    // contract `action_target::wire_control` relies on for `NSControl`.
    unsafe {
        item.setTarget(Some(target));
        item.setAction(Some(ActionTarget::perform_action_selector()));
    }
}

/// Wires `item`'s action to `NSWindow`'s standard `performClose:` selector, leaving its target
/// `nil` (never set) so `AppKit`'s own responder-chain routing decides which window it acts on —
/// the standard "Window > Close" pattern every Mac app uses, rather than this crate hard-coding a
/// specific window (and rather than routing through [`ActionTarget`], which has no `performClose:`
/// method of its own to receive it).
pub fn wire_to_perform_close(item: &NSMenuItem) {
    // SAFETY: `performClose:` is `NSWindow`'s own standard selector
    // (`-[NSWindow performClose:]`), not one this crate declares; leaving `target` `nil` is itself
    // safe and is exactly what tells `AppKit` to resolve it through the responder chain (the key
    // window, if any implements it) at the moment the item is validated/used, not before.
    unsafe {
        item.setAction(Some(sel!(performClose:)));
    }
}
