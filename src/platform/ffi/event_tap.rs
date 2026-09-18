//! Raw `CGEventTap` bindings (`docs/ARCHITECTURE.md` "Safety net").
//!
//! `core-graphics`'s own [`core_graphics::event::CGEventTap`] wrapper is close but not quite
//! enough for what [`crate::platform::input_blocker`] needs: it has no way to *disable* an
//! existing tap (only `enable()` and `Drop`, which invalidates), and its event mask can only be
//! built from the public [`core_graphics::event::CGEventType`] enum, which does not include
//! `NX_SYSDEFINED` (media/volume/brightness keys) or the undocumented trackpad gesture event
//! types. This module talks to `CGEventTapCreate`/`CGEventTapEnable`/`CGEventGetLocation`
//! directly to get that control, while still exposing only safe functions to the rest of the
//! crate.
//!
//! Every raw event type number below is documented where it comes from; the ones with a public
//! `CGEventType` counterpart are cross-checked in tests against `core_graphics::event::CGEventType`.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use core_foundation::base::TCFType;
use core_foundation::mach_port::{CFMachPort, CFMachPortRef};
use core_foundation::runloop::CFRunLoopSource;
use core_graphics::geometry::CGPoint;

/// Opaque `CGEventRef` as seen by an event tap callback.
pub type CGEventRef = *mut c_void;
/// Opaque `CGEventTapProxy` handed to the callback; only ever passed back to
/// `CGEventTapPostEvent`, which this crate never calls.
pub type CGEventTapProxy = *const c_void;

/// Raw CoreGraphics/`NSEvent` event type numbers used to build a tap's event mask.
///
/// Values with a public `core_graphics::event::CGEventType` counterpart are the same integer;
/// the system-defined and gesture ones are not part of that enum (see the module docs) but are
/// stable across macOS releases in practice (they come from the same `NSEventType` C enum Apple
/// has shipped unchanged for over a decade).
pub mod event_type {
    /// `kCGEventLeftMouseDown`.
    pub const LEFT_MOUSE_DOWN: u32 = 1;
    /// `kCGEventLeftMouseUp`.
    pub const LEFT_MOUSE_UP: u32 = 2;
    /// `kCGEventRightMouseDown`.
    pub const RIGHT_MOUSE_DOWN: u32 = 3;
    /// `kCGEventRightMouseUp`.
    pub const RIGHT_MOUSE_UP: u32 = 4;
    /// `kCGEventLeftMouseDragged`.
    pub const LEFT_MOUSE_DRAGGED: u32 = 6;
    /// `kCGEventRightMouseDragged`.
    pub const RIGHT_MOUSE_DRAGGED: u32 = 7;
    /// `kCGEventKeyDown`.
    pub const KEY_DOWN: u32 = 10;
    /// `kCGEventKeyUp`.
    pub const KEY_UP: u32 = 11;
    /// `kCGEventFlagsChanged`.
    pub const FLAGS_CHANGED: u32 = 12;
    /// `kCGEventScrollWheel`.
    pub const SCROLL_WHEEL: u32 = 22;
    /// `kCGEventOtherMouseDown` (middle/extra buttons).
    pub const OTHER_MOUSE_DOWN: u32 = 25;
    /// `kCGEventOtherMouseUp`.
    pub const OTHER_MOUSE_UP: u32 = 26;
    /// `kCGEventOtherMouseDragged`.
    pub const OTHER_MOUSE_DRAGGED: u32 = 27;
    /// `NX_SYSDEFINED`: media/volume/brightness keys and other system-defined events. Not part of
    /// the public `CGEventType` enum.
    pub const SYSTEM_DEFINED: u32 = 14;
    /// `NSEventTypeRotate`, undocumented trackpad rotation gesture.
    pub const ROTATE: u32 = 18;
    /// `NSEventTypeGesture`, undocumented generic trackpad gesture envelope.
    pub const GESTURE: u32 = 29;
    /// `NSEventTypeMagnify`, undocumented pinch-to-zoom trackpad gesture.
    pub const MAGNIFY: u32 = 30;
    /// `NSEventTypeSwipe`, undocumented three/four-finger swipe trackpad gesture.
    pub const SWIPE: u32 = 31;

    /// `kCGEventTapDisabledByTimeout`, delivered to the callback instead of a real event.
    pub const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    /// `kCGEventTapDisabledByUserInput`, delivered to the callback instead of a real event.
    pub const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
}

/// `kCGSessionEventTap`.
pub const TAP_LOCATION_SESSION: u32 = 1;
/// `kCGHeadInsertEventTap`.
pub const TAP_PLACEMENT_HEAD_INSERT: u32 = 0;
/// `kCGEventTapOptionDefault` (an active filter, as opposed to a passive listener).
pub const TAP_OPTION_DEFAULT: u32 = 0;

/// Builds a `CGEventMask` bit set from raw event type numbers (see [`event_type`]).
#[must_use]
pub fn build_mask(types: &[u32]) -> u64 {
    types.iter().fold(0u64, |mask, &t| mask | (1u64 << t))
}

/// What the tap callback tells the OS to do with an event: pass it through untouched, or drop it
/// (swallow it, so no other app or the rest of the system ever sees it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapAction {
    /// Let the event continue to whatever would normally receive it.
    Pass,
    /// Consume the event; nothing downstream (including the target app) sees it.
    Swallow,
}

type BoxedCallback = Box<dyn Fn(u32, CGEventRef) -> TapAction + Send + Sync + 'static>;

/// A raw, running `CGEventTap`. Must be created and dropped on the thread whose `CFRunLoop` it is
/// attached to; see [`RawTap::create`] and [`RawTap::add_to_current_run_loop`].
pub struct RawTap {
    mach_port: CFMachPort,
    // Kept alive for as long as the tap can still be called back into; the raw pointer handed to
    // `CGEventTapCreate` as `userInfo` points inside this box.
    _callback: Box<BoxedCallback>,
    /// Raw `CFMachPortRef` address, published for cross-thread enable/disable (see
    /// [`TapHandle`]). Cleared (set to 0) once this `RawTap` is dropped.
    shared_addr: Arc<AtomicUsize>,
}

/// A cheap, `Send + Sync` handle that can enable/disable a [`RawTap`] from any thread — used by
/// the watchdog thread and by [`crate::platform::input_blocker::BlockerGuard::drop`], neither of
/// which run on the tap's own thread.
///
/// SAFETY: this only ever stores a `CFMachPortRef`'s numeric address and passes it back to
/// `CGEventTapEnable`, which Apple documents as safe to call from any thread (it is the mechanism
/// accessibility tools are expected to use to react to `kCGEventTapDisabledByTimeout` from a
/// watchdog independent of the tap's own run loop). The pointee itself is never dereferenced here.
#[derive(Clone)]
pub struct TapHandle {
    addr: Arc<AtomicUsize>,
}

impl TapHandle {
    /// A handle not yet bound to any tap (every call is a no-op) — used to give a tap's own
    /// callback a way to re-enable itself (e.g. on `kCGEventTapDisabledByTimeout`) before the
    /// [`RawTap`] it will belong to exists yet. [`RawTap::create`] binds it.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            addr: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Enables the tap (a no-op if it has already been invalidated).
    pub fn enable(&self) {
        self.with_port(|port| {
            // SAFETY: `port` is a live `CFMachPortRef` created by `CGEventTapCreate` and not yet
            // invalidated (checked by `with_port`); `CGEventTapEnable` only flips an internal
            // flag and is safe to call from any thread.
            unsafe { CGEventTapEnable(port, true) }
        });
    }

    /// Disables the tap (a no-op if it has already been invalidated).
    pub fn disable(&self) {
        self.with_port(|port| {
            // SAFETY: see `enable`.
            unsafe { CGEventTapEnable(port, false) }
        });
    }

    fn with_port(&self, f: impl FnOnce(CFMachPortRef)) {
        let raw = self.addr.load(Ordering::SeqCst);
        if raw != 0 {
            f(raw as CFMachPortRef);
        }
    }
}

impl RawTap {
    /// Creates and enables an event tap at `location`/`placement`/`options` (see the
    /// `TAP_LOCATION_*`/`TAP_PLACEMENT_*`/`TAP_OPTION_*` constants) listening for `mask`
    /// (built with [`build_mask`]). `callback` is invoked for every matching event, on whatever
    /// thread later calls [`RawTap::add_to_current_run_loop`]'s run loop; it must never block.
    ///
    /// `handle` is bound to the created tap on success (its `enable`/`disable` start working);
    /// pass [`TapHandle::empty`] cloned ahead of time if `callback` needs to re-enable the tap
    /// itself (e.g. on `kCGEventTapDisabledByTimeout`/`ByUserInput`), or [`RawTap::handle`] to get
    /// one after the fact.
    ///
    /// # Errors
    ///
    /// Returns `Err(())` if `CGEventTapCreate` fails (typically: the process is not trusted for
    /// Accessibility).
    #[allow(
        clippy::result_unit_err,
        reason = "this is a thin, crate-internal FFI wrapper: the only caller \
                  (input_blocker::InputBlocker::start) immediately maps failure to the typed, \
                  documented BlockError it returns to the rest of the crate"
    )]
    pub fn create(
        location: u32,
        placement: u32,
        options: u32,
        mask: u64,
        handle: &TapHandle,
        callback: impl Fn(u32, CGEventRef) -> TapAction + Send + Sync + 'static,
    ) -> Result<Self, ()> {
        let boxed: BoxedCallback = Box::new(callback);
        let mut callback_box = Box::new(boxed);
        let user_info = std::ptr::addr_of_mut!(*callback_box).cast::<c_void>();

        // SAFETY: `trampoline` matches `CGEventTapCallBack`'s C signature exactly; `user_info`
        // points at `callback_box`, which this `RawTap` keeps alive for as long as the mach port
        // (and thus the ability for the OS to call back into it) is alive. `CGEventTapCreate`
        // returns either null or a valid, owned (`create rule`) `CFMachPortRef`.
        let raw_port =
            unsafe { CGEventTapCreate(location, placement, options, mask, trampoline, user_info) };

        if raw_port.is_null() {
            return Err(());
        }

        // SAFETY: `raw_port` was just returned non-null by `CGEventTapCreate`, which follows the
        // "create rule" (we own one reference).
        let mach_port = unsafe { CFMachPort::wrap_under_create_rule(raw_port) };
        let shared_addr = Arc::clone(&handle.addr);
        shared_addr.store(raw_port as usize, Ordering::SeqCst);

        Ok(Self {
            mach_port,
            _callback: callback_box,
            shared_addr,
        })
    }

    /// A cross-thread handle to enable/disable this tap (shares state with any [`TapHandle`]
    /// already bound to it via [`RawTap::create`]).
    #[must_use]
    pub fn handle(&self) -> TapHandle {
        TapHandle {
            addr: Arc::clone(&self.shared_addr),
        }
    }

    /// Creates a run loop source for this tap and adds it to the calling thread's current
    /// `CFRunLoop` in the common modes, then enables the tap. The caller is expected to run that
    /// run loop afterwards (e.g. `CFRunLoop::run_current()`).
    ///
    /// # Errors
    ///
    /// Returns `Err(())` if creating the run loop source fails.
    #[allow(
        clippy::result_unit_err,
        reason = "see RawTap::create: the only caller maps failure to a typed BlockError"
    )]
    pub fn add_to_current_run_loop(&self) -> Result<CFRunLoopSource, ()> {
        use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes};

        let source = self.mach_port.create_runloop_source(0)?;
        // SAFETY: `kCFRunLoopCommonModes` is a valid CoreFoundation constant; `add_source` only
        // registers the source, it does not take ownership beyond a CF retain.
        CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
        self.handle().enable();
        Ok(source)
    }
}

impl Drop for RawTap {
    fn drop(&mut self) {
        self.shared_addr.store(0, Ordering::SeqCst);
        // SAFETY: `self.mach_port`'s underlying `CFMachPortRef` is still valid (we hold the only
        // `RawTap`, and `shared_addr` was just cleared so no other thread will touch it after
        // this point); invalidating it detaches the tap from the event stream and any run loop.
        unsafe { CFMachPortInvalidate(self.mach_port.as_concrete_TypeRef()) };
    }
}

/// Reads a pointer event's location, in global points with the origin at the top-left of the
/// main display (`CGEventGetLocation`'s own documented convention) — the coordinate space
/// `core::session::InputEvent`'s pointer variants use.
///
/// `event` must be a live `CGEventRef`, i.e. one currently being handed to a [`RawTap`] callback
/// (the only place this crate ever obtains one); nothing in this crate keeps a `CGEventRef`
/// beyond the callback that received it, so this holds for every caller (same precondition as
/// [`integer_value_field`]).
#[must_use]
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "the precondition (a CGEventRef only outlives the tap callback that owns it) is a \
              crate-wide invariant, not something the caller decides per call; every call site in \
              this crate is inside that callback, so this is a false positive for us"
)]
pub fn event_location(event: CGEventRef) -> (f32, f32) {
    // SAFETY: `event` is a valid `CGEventRef` for the duration of the tap callback that owns it
    // (never stored beyond that call); `CGEventGetLocation` only reads the event, it never
    // mutates or retains it.
    let point = unsafe { CGEventGetLocation(event) };
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a display coordinate comfortably fits in f32's precision range"
    )]
    (point.x as f32, point.y as f32)
}

/// Reads an integer field (see `core_graphics::event::EventField`) from a raw event. Same
/// liveness precondition on `event` as [`event_location`].
#[must_use]
#[allow(
    clippy::not_unsafe_ptr_arg_deref,
    reason = "see event_location: every call site in this crate is inside the tap callback that \
              owns the event"
)]
pub fn integer_value_field(event: CGEventRef, field: u32) -> i64 {
    // SAFETY: `event` is a valid `CGEventRef` for the duration of the call, as above.
    unsafe { CGEventGetIntegerValueField(event, field) }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CFMachPortInvalidate(port: CFMachPortRef);
}

/// C-ABI trampoline handed to `CGEventTapCreate`; dispatches to the boxed Rust closure stored at
/// `user_info` and translates [`TapAction`] into the null-to-swallow convention `CGEventTapCreate`
/// expects.
///
/// # Safety
///
/// Called only by the OS, on the thread running the tap's `CFRunLoop`, with `user_info` pointing
/// at the `BoxedCallback` a live [`RawTap`] keeps allocated; `event` is a valid `CGEventRef` for
/// the duration of the call.
unsafe extern "C" fn trampoline(
    _proxy: CGEventTapProxy,
    etype: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    // SAFETY: `user_info` was set from a `&mut BoxedCallback` inside `RawTap::create` and stays
    // valid for as long as the `RawTap` (and thus the tap that can call back into this function)
    // is alive.
    let callback = unsafe { &*user_info.cast::<BoxedCallback>() };
    match callback(etype, event) {
        TapAction::Pass => event,
        TapAction::Swallow => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_mask_sets_expected_bits() {
        let mask = build_mask(&[event_type::KEY_DOWN, event_type::KEY_UP]);
        assert_eq!(mask, (1u64 << 10) | (1u64 << 11));
    }

    #[test]
    fn build_mask_of_empty_slice_is_zero() {
        assert_eq!(build_mask(&[]), 0);
    }

    #[test]
    fn event_type_constants_match_core_graphics_cgeventtype() {
        use core_graphics::event::CGEventType;
        assert_eq!(event_type::KEY_DOWN, CGEventType::KeyDown as u32);
        assert_eq!(event_type::KEY_UP, CGEventType::KeyUp as u32);
        assert_eq!(event_type::FLAGS_CHANGED, CGEventType::FlagsChanged as u32);
        assert_eq!(event_type::SCROLL_WHEEL, CGEventType::ScrollWheel as u32);
        assert_eq!(
            event_type::LEFT_MOUSE_DOWN,
            CGEventType::LeftMouseDown as u32
        );
        assert_eq!(
            event_type::TAP_DISABLED_BY_TIMEOUT,
            CGEventType::TapDisabledByTimeout as u32
        );
        assert_eq!(
            event_type::TAP_DISABLED_BY_USER_INPUT,
            CGEventType::TapDisabledByUserInput as u32
        );
    }
}
