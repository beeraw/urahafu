//! A single native `NSObject` subclass wired as the target/delegate for every `AppKit` callback the
//! settings window needs (DESIGN.md "Settings window"): button/checkbox/popup actions
//! (`performAction:`, decoding `sender.tag()`), the window's close button/Cmd-W
//! (`windowShouldClose:`), and the Dock/Finder "reopen" Apple Event
//! (`handleReopen:withReplyEvent:`, registered via [`register_reopen_handler`] since winit
//! exposes none of this — there is no `ApplicationHandler` hook for it).
//!
//! This is one of the designated FFI submodules (`docs/ARCHITECTURE.md` goal 4: every `unsafe`
//! block lives under `src/platform/ffi/`, documented with `// SAFETY:`); the crate root denies
//! `unsafe_code` everywhere else.

#![allow(
    unsafe_code,
    reason = "this module wraps objc2's define_class! machinery (building a custom NSObject \
              subclass) and NSAppleEventManager's selector-registration API, both of which \
              require unsafe; see docs/ARCHITECTURE.md goal 4"
)]

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSBackingStoreType, NSControl, NSWindow, NSWindowDelegate, NSWindowStyleMask};
use objc2_core_services::{kAEReopenApplication, kCoreEventClass};
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleEventManager, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString,
};

/// One decoded action this target can report: a tagged control's `performAction:`, the window
/// being asked to close (`windowShouldClose:`), or the Dock/Finder "reopen" Apple Event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionId {
    /// A control's `tag` (`NSInteger`), as set when it was built; see
    /// `crate::platform::settings_window` for what each tag means.
    Tag(isize),
    /// `windowShouldClose:` was called on the window this target is the delegate of.
    Close,
    /// `kAEReopenApplication` was received (the user reopened the app — Dock, Spotlight, Finder —
    /// while it was already running).
    Reopen,
}

/// Ivars for [`ActionTarget`]: a boxed Rust closure invoked for every decoded [`ActionId`].
/// `RefCell` rather than a plain field since `AppKit` may call back into this object reentrantly
/// (e.g. `performAction:` triggering a settings save that itself touches the window); the
/// closure body never needs a mutable borrow held across a call into `AppKit`.
///
/// `pub` (not private): it is [`ActionTarget`]'s `DefinedClass::Ivars` associated type, and that
/// impl is itself public, so the type it names must be reachable too; there is nothing sensitive
/// in it and no code outside this module has any use for it.
pub struct ActionTargetIvars {
    /// Invoked for every decoded [`ActionId`].
    callback: RefCell<Box<dyn Fn(ActionId)>>,
}

define_class!(
    // SAFETY:
    // - The superclass NSObject does not have any subclassing requirements.
    // - `ActionTarget` does not implement `Drop`.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ActionTargetIvars]
    #[name = "UrahafuActionTarget"]
    /// The single native `NSObject` subclass wired as target/delegate for the settings window
    /// (see this module's own docs).
    pub struct ActionTarget;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for ActionTarget {}

    // SAFETY: `NSWindowDelegate` has no safety requirements.
    unsafe impl NSWindowDelegate for ActionTarget {
        // SAFETY: The signature (`(&NSWindow) -> bool`) matches `windowShouldClose:`.
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            (self.ivars().callback.borrow())(ActionId::Close);
            // The app decides hide-vs-quit itself (DESIGN.md "Settings window", "Closing the
            // window"); never let AppKit close (and, per `setReleasedWhenClosed(false)`, release)
            // the window on its own.
            false
        }
    }

    impl ActionTarget {
        // SAFETY: `performAction:` is this type's own selector (not an AppKit-declared one); the
        // sender is always an `NSControl` subclass (`NSButton`/`NSPopUpButton`), since this
        // method is only ever wired via `setAction:` on those.
        #[unsafe(method(performAction:))]
        fn perform_action(&self, sender: &NSControl) {
            let tag = sender.tag();
            (self.ivars().callback.borrow())(ActionId::Tag(tag));
        }

        // SAFETY: `(NSAppleEventDescriptor *, NSAppleEventDescriptor *) -> void` is the exact
        // signature Apple Event dispatch requires for a handler registered via
        // `setEventHandler:andSelector:forEventClass:andEventID:` (see
        // `register_reopen_handler` below).
        #[unsafe(method(handleReopen:withReplyEvent:))]
        fn handle_reopen(
            &self,
            _event: &NSAppleEventDescriptor,
            _reply_event: &NSAppleEventDescriptor,
        ) {
            (self.ivars().callback.borrow())(ActionId::Reopen);
        }
    }
);

impl ActionTarget {
    /// Builds a new target forwarding every decoded [`ActionId`] to `callback`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, callback: impl Fn(ActionId) + 'static) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ActionTargetIvars {
            callback: RefCell::new(Box::new(callback)),
        });
        // SAFETY: `NSObject`'s `init` takes no arguments and always returns a valid instance.
        unsafe { msg_send![super(this), init] }
    }

    /// The Objective-C selector for `performAction:`, for wiring a control's `setAction:`.
    #[must_use]
    pub fn perform_action_selector() -> Sel {
        sel!(performAction:)
    }
}

/// Builds the settings window's `NSWindow` itself: titled, closable, miniaturizable, not
/// resizable, `title` as its title, `content_size` as its content rect (points), and
/// `releasedWhenClosed = false` so the app can reuse (rather than recreate) it for its whole
/// lifetime — the same pattern `objc2`'s own `hello_world_app` example uses for a window built
/// outside a window controller. This is the one safe wrapper `src/platform/settings_window.rs`
/// (a safe-code-only module) needs for the two calls objc2-app-kit 0.3.2 marks `unsafe`
/// (`initWithContentRect:styleMask:backing:defer:` and `setReleasedWhenClosed:`); every other
/// window/control call it makes (title, delegate, center, ordering, stack view layout, ...) is
/// already safe in this crate's objc2-app-kit version.
#[must_use]
pub fn new_settings_window(
    mtm: MainThreadMarker,
    content_size: NSSize,
    title: &str,
) -> Retained<NSWindow> {
    let style =
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), content_size);
    // SAFETY: `NSWindow::alloc(mtm)` is a freshly allocated, uninitialized instance, exactly what
    // `initWithContentRect_styleMask_backing_defer` requires; the style mask and backing store
    // are both plain, well-defined enum values with no additional preconditions.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: this window is never created via a window controller, and the app (`SettingsWindow`
    // in `src/platform/settings_window.rs`) keeps it alive and reuses it for its own lifetime
    // instead of relying on AppKit's close-time release, exactly the case this setter documents.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(title));
    window
}

/// Wires `control`'s target/action (`NSControl::setTarget:`/`setAction:`, both `unsafe`: `AppKit`
/// trusts the caller that `target` actually implements `action`) to `target`'s
/// [`ActionTarget::perform_action_selector`]. Every interactive control in the settings window
/// uses this same target/selector pair; [`ActionId::Tag`] (via the control's own `tag`, set
/// separately with the always-safe `NSControl::setTag:`) is how the callback tells them apart.
pub fn wire_control(control: &NSControl, target: &ActionTarget) {
    // SAFETY: `target` is a live `ActionTarget`, which implements `performAction:` with the
    // `(id sender) -> void` signature `NSControl`'s target/action mechanism requires.
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(ActionTarget::perform_action_selector()));
    }
}

/// Registers `target` as the handler for the Dock/Finder "reopen" Apple Event
/// (`kAEReopenApplication`) — the only way to observe "the user opened the app again while it was
/// already running". winit exposes no such hook, and layering a second
/// `NSApplicationDelegate` on top of winit's own (which would otherwise carry
/// `applicationShouldHandleReopen:hasVisibleWindows:`) is not an option since winit already
/// installs and owns one.
pub fn register_reopen_handler(target: &ActionTarget) {
    let manager = NSAppleEventManager::sharedAppleEventManager();
    // SAFETY: `target` is a live `ActionTarget`, which implements `handleReopen:withReplyEvent:`
    // with the exact `(NSAppleEventDescriptor *, NSAppleEventDescriptor *) -> void` signature
    // Apple Event dispatch requires for a handler registered this way (see the method's own
    // SAFETY comment above); `kCoreEventClass`/`kAEReopenApplication` are the well-known
    // four-char codes ('aevt'/'rapp') Apple's documentation specifies for this exact event.
    unsafe {
        manager.setEventHandler_andSelector_forEventClass_andEventID(
            target,
            sel!(handleReopen:withReplyEvent:),
            kCoreEventClass,
            kAEReopenApplication,
        );
    }
}
