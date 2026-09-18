//! The Safety net: the input-blocking event tap (`docs/ARCHITECTURE.md` "Safety net", goal 1).
//!
//! [`InputBlocker::start`] refuses to start (returning a [`BlockError`]) unless input blocking
//! can actually work; the caller is expected to show the matching error alert and never show the
//! cleaning overlay in that case. Once started, the returned [`BlockerGuard`] guarantees inputs
//! are released by `deadline` even if the caller hangs, panics, or simply forgets: a watchdog
//! thread and a global panic hook both act independently of whatever called `start`.
//!
//! ## What is blocked
//!
//! Key down/up, modifier-flag changes, every mouse button down/up/dragged, the scroll wheel, and
//! `NX_SYSDEFINED` (media/volume/brightness keys) are all captured and swallowed. Trackpad
//! gesture events (pinch/rotate/swipe, plus the generic "gesture" envelope — undocumented
//! `NSEventType` values not in the public `CGEventType` enum, see
//! `src/platform/ffi/event_tap.rs`) are captured and swallowed too, mapped to
//! [`InputEvent::KeyDown`] (`KeyKind::Other`) like system-defined keys, since the session state
//! machine has no separate "gesture" concept. **Plain pointer movement (no button held) is
//! deliberately left alone** — the hold-to-unlock button's hover reveal reads it via winit's
//! `CursorMoved` instead (`src/app/`), and blocking `mouseMoved` at the tap would also block the
//! OS's own cursor rendering for no benefit.
//!
//! Key down and key up are both forwarded now (DESIGN.md §8, the Esc+Return unlock combo):
//! a key-down event carries whether it is an OS-generated autorepeat
//! (`kCGKeyboardEventAutorepeat`), and a modifier-flag change (`kCGEventFlagsChanged`) is
//! forwarded too, as [`InputEvent::ModifierChange`] — neither carries a character or, for
//! `ModifierChange`, even which modifier changed; see `core::combo`'s module docs for why.
//!
//! ## Threading
//!
//! The tap runs on its own thread with its own `CFRunLoop`, per `docs/ARCHITECTURE.md`. A second,
//! independent watchdog thread wakes at `deadline + 2s` and force-disables the tap and stops its
//! run loop, regardless of what the main thread or the `Session` state machine are doing. A
//! process-wide panic hook (installed once) flips every live blocker's `active` flag to `false`
//! before running the previous hook, so a panic anywhere still lets input through immediately
//! (the watchdog is the backstop for everything else, including a hang).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, Once, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use core_foundation::runloop::CFRunLoop;
use core_graphics::event::EventField;

use crate::core::session::{InputEvent, KeyKind};
use crate::platform::ffi::event_tap::{
    self, CGEventRef, RawTap, TAP_LOCATION_SESSION, TAP_OPTION_DEFAULT, TAP_PLACEMENT_HEAD_INSERT,
    TapAction, TapHandle, build_mask, event_type,
};
use crate::platform::ffi::{accessibility, secure_input};

/// Extra time after `deadline` before the watchdog thread forces the tap off, independent of the
/// `Session` state machine (`docs/ARCHITECTURE.md` "Safety net").
const WATCHDOG_GRACE: Duration = Duration::from_secs(2);

/// How often the watchdog and the guard's teardown check for an early-stop signal, so `Drop`
/// never has to wait the full grace period.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Virtual key code of the Escape key (`kVK_Escape`), constant across keyboard layouts.
const KEYCODE_ESCAPE: i64 = 53;
/// Virtual key code of the Return key (`kVK_Return`), constant across keyboard layouts.
const KEYCODE_RETURN: i64 = 36;
/// Virtual key code of the numeric keypad's Enter key (`kVK_ANSI_KeypadEnter`), constant across
/// keyboard layouts; counts as Return for the Esc+Return unlock combo (DESIGN.md §8).
const KEYCODE_KEYPAD_ENTER: i64 = 76;
/// Virtual key code of the Space bar (`kVK_Space`), constant across keyboard layouts.
const KEYCODE_SPACE: i64 = 49;

/// Why [`InputBlocker::start`] refused to start. In every case, no event tap was installed; the
/// caller must show the matching alert (DESIGN.md §11) and never show the cleaning overlay.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum BlockError {
    /// Accessibility permission is not granted (`AXIsProcessTrusted` is false).
    #[error("accessibility permission not granted")]
    PermissionDenied,
    /// Secure Input is active system-wide (e.g. a password field has focus somewhere), which
    /// would prevent the tap from ever seeing keystrokes.
    #[error("secure input is active")]
    SecureInputActive,
    /// `CGEventTapCreate` (or setting up its run loop source, or one of the supporting threads)
    /// failed for a reason other than the two above.
    #[error("failed to create the input event tap")]
    TapCreationFailed,
}

/// Installs (or refuses to install) the input-blocking event tap.
pub struct InputBlocker;

impl InputBlocker {
    /// Starts blocking keyboard, mouse, scroll and media-key input until `deadline`, forwarding
    /// decoded [`InputEvent`]s to `events`. See the module docs for what is/isn't covered and the
    /// threading model.
    ///
    /// # Errors
    ///
    /// Returns a [`BlockError`] — and installs nothing — if the preflight checks fail or the tap
    /// can't be created; see [`BlockError`]'s variants.
    pub fn start(
        deadline: Instant,
        events: Sender<InputEvent>,
    ) -> Result<BlockerGuard, BlockError> {
        if !accessibility::is_trusted() {
            return Err(BlockError::PermissionDenied);
        }
        if secure_input::is_secure_input_enabled() {
            return Err(BlockError::SecureInputActive);
        }

        let active = Arc::new(AtomicBool::new(true));
        register_active_flag(&active);

        let handle = TapHandle::empty();
        let mask = build_mask(&[
            event_type::KEY_DOWN,
            event_type::KEY_UP,
            event_type::FLAGS_CHANGED,
            event_type::LEFT_MOUSE_DOWN,
            event_type::LEFT_MOUSE_UP,
            event_type::RIGHT_MOUSE_DOWN,
            event_type::RIGHT_MOUSE_UP,
            event_type::OTHER_MOUSE_DOWN,
            event_type::OTHER_MOUSE_UP,
            event_type::LEFT_MOUSE_DRAGGED,
            event_type::RIGHT_MOUSE_DRAGGED,
            event_type::OTHER_MOUSE_DRAGGED,
            event_type::SCROLL_WHEEL,
            event_type::SYSTEM_DEFINED,
            event_type::ROTATE,
            event_type::GESTURE,
            event_type::MAGNIFY,
            event_type::SWIPE,
        ]);

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<CFRunLoop, ()>>();
        let tap_thread = {
            let active = Arc::clone(&active);
            let handle = handle.clone();
            thread::Builder::new()
                .name("urahafu-input-tap".to_owned())
                .spawn(move || tap_thread_main(&active, deadline, events, mask, &handle, &ready_tx))
                .map_err(|_| BlockError::TapCreationFailed)?
        };

        let Ok(Ok(run_loop)) = ready_rx.recv() else {
            // The tap thread failed to create the tap (or died before reporting); it has
            // already returned, so joining cannot block long.
            let _ = tap_thread.join();
            return Err(BlockError::TapCreationFailed);
        };

        let watchdog_stop = Arc::new(AtomicBool::new(false));
        let watchdog_thread = {
            let handle = handle.clone();
            let run_loop = run_loop.clone();
            let watchdog_stop = Arc::clone(&watchdog_stop);
            thread::Builder::new()
                .name("urahafu-input-watchdog".to_owned())
                .spawn(move || watchdog_main(deadline, &handle, &run_loop, &watchdog_stop))
                .map_err(|_| BlockError::TapCreationFailed)
        };

        let watchdog_thread = match watchdog_thread {
            Ok(t) => t,
            Err(err) => {
                active.store(false, Ordering::SeqCst);
                handle.disable();
                run_loop.stop();
                let _ = tap_thread.join();
                return Err(err);
            }
        };

        Ok(BlockerGuard {
            active,
            handle,
            run_loop,
            watchdog_stop,
            tap_thread: Some(tap_thread),
            watchdog_thread: Some(watchdog_thread),
        })
    }
}

/// Body of the tap thread: creates the tap, attaches it to this thread's run loop, reports
/// success (with the run loop, so [`BlockerGuard`] can stop it later) or failure back over
/// `ready_tx`, then runs the run loop until something (`BlockerGuard::drop` or the watchdog)
/// stops it.
fn tap_thread_main(
    active: &Arc<AtomicBool>,
    deadline: Instant,
    events_tx: Sender<InputEvent>,
    mask: u64,
    handle: &TapHandle,
    ready_tx: &Sender<Result<CFRunLoop, ()>>,
) {
    let callback = {
        let active = Arc::clone(active);
        let handle = handle.clone();
        move |etype: u32, event: CGEventRef| -> TapAction {
            if !active.load(Ordering::SeqCst) || Instant::now() >= deadline {
                return TapAction::Pass;
            }
            if matches!(
                etype,
                event_type::TAP_DISABLED_BY_TIMEOUT | event_type::TAP_DISABLED_BY_USER_INPUT
            ) {
                handle.enable();
                return TapAction::Pass;
            }
            dispatch_event(etype, event, &events_tx)
        }
    };

    let Ok(tap) = RawTap::create(
        TAP_LOCATION_SESSION,
        TAP_PLACEMENT_HEAD_INSERT,
        TAP_OPTION_DEFAULT,
        mask,
        handle,
        callback,
    ) else {
        let _ = ready_tx.send(Err(()));
        return;
    };

    let Ok(_source) = tap.add_to_current_run_loop() else {
        let _ = ready_tx.send(Err(()));
        return;
    };

    if ready_tx.send(Ok(CFRunLoop::get_current())).is_err() {
        // The starter already gave up (e.g. it errored out while we were setting up); let `tap`
        // and `_source` drop, which tears the port down.
        return;
    }

    CFRunLoop::run_current();
    // `tap`/`_source` drop here once the run loop returns.
}

/// Translates one tap event into an [`InputEvent`] (if any) and sends it, always swallowing the
/// event itself. Only reachable for the event types in the tap's mask.
///
/// Pointer events carry their location (`CGEventGetLocation`: global points, origin at the
/// top-left of the main display — `core::session::InputEvent`'s documented coordinate
/// convention) so the session's hold-to-unlock button can tell whether the pointer is inside its
/// hit target.
fn dispatch_event(etype: u32, event: CGEventRef, tx: &Sender<InputEvent>) -> TapAction {
    let mapped = match etype {
        event_type::KEY_DOWN => {
            let keycode = event_tap::integer_value_field(event, EventField::KEYBOARD_EVENT_KEYCODE);
            let repeat =
                event_tap::integer_value_field(event, EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0;
            Some(InputEvent::KeyDown {
                kind: map_key_kind(keycode),
                repeat,
            })
        }
        event_type::KEY_UP => {
            let keycode = event_tap::integer_value_field(event, EventField::KEYBOARD_EVENT_KEYCODE);
            Some(InputEvent::KeyUp(map_key_kind(keycode)))
        }
        event_type::FLAGS_CHANGED => Some(InputEvent::ModifierChange),
        event_type::LEFT_MOUSE_DOWN
        | event_type::RIGHT_MOUSE_DOWN
        | event_type::OTHER_MOUSE_DOWN => {
            let (x, y) = event_tap::event_location(event);
            Some(InputEvent::PointerDown { x, y })
        }
        event_type::LEFT_MOUSE_UP | event_type::RIGHT_MOUSE_UP | event_type::OTHER_MOUSE_UP => {
            let (x, y) = event_tap::event_location(event);
            Some(InputEvent::PointerUp { x, y })
        }
        event_type::LEFT_MOUSE_DRAGGED
        | event_type::RIGHT_MOUSE_DRAGGED
        | event_type::OTHER_MOUSE_DRAGGED => {
            let (x, y) = event_tap::event_location(event);
            Some(InputEvent::PointerDragged { x, y })
        }
        event_type::SCROLL_WHEEL => Some(InputEvent::Scroll),
        event_type::SYSTEM_DEFINED
        | event_type::ROTATE
        | event_type::GESTURE
        | event_type::MAGNIFY
        | event_type::SWIPE => Some(InputEvent::KeyDown {
            kind: KeyKind::Other,
            repeat: false,
        }),
        // Unreachable given the tap's mask, but swallowed defensively all the same.
        _ => None,
    };
    if let Some(input_event) = mapped {
        // Ignore send errors: if the receiver is gone, the session is already over and there is
        // nothing useful to do; the event is still swallowed either way.
        let _ = tx.send(input_event);
    }
    TapAction::Swallow
}

/// Maps a raw virtual key code to a [`KeyKind`]. No character is ever decoded or kept — the
/// hold-to-unlock button and the Esc+Return combo need none (`docs/ARCHITECTURE.md` goal 2). Pure,
/// so it is unit-tested without a real event tap. The numeric keypad's Enter key
/// (`kVK_ANSI_KeypadEnter`) maps to [`KeyKind::Return`] too — it counts as Return for the combo.
#[must_use]
pub fn map_key_kind(keycode: i64) -> KeyKind {
    match keycode {
        KEYCODE_ESCAPE => KeyKind::Escape,
        KEYCODE_RETURN | KEYCODE_KEYPAD_ENTER => KeyKind::Return,
        KEYCODE_SPACE => KeyKind::Space,
        _ => KeyKind::Other,
    }
}

/// Body of the watchdog thread: waits until `deadline + WATCHDOG_GRACE` (or an early-stop signal
/// from `BlockerGuard::drop`), then unconditionally disables the tap and stops its run loop —
/// independent of the `Session` state machine and of whatever the main thread is doing.
fn watchdog_main(deadline: Instant, handle: &TapHandle, run_loop: &CFRunLoop, stop: &AtomicBool) {
    let fire_at = deadline + WATCHDOG_GRACE;
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let now = Instant::now();
        if now >= fire_at {
            break;
        }
        thread::sleep((fire_at - now).min(POLL_INTERVAL));
    }
    handle.disable();
    run_loop.stop();
}

/// Global registry of every live blocker's `active` flag, so the panic hook can flip them all to
/// `false` regardless of which one (if any) is involved in the panic.
static ACTIVE_FLAGS: Mutex<Vec<Weak<AtomicBool>>> = Mutex::new(Vec::new());
static PANIC_HOOK_INSTALLED: Once = Once::new();

fn register_active_flag(flag: &Arc<AtomicBool>) {
    PANIC_HOOK_INSTALLED.call_once(install_panic_hook);
    let mut guard = ACTIVE_FLAGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.retain(|w| w.strong_count() > 0);
    guard.push(Arc::downgrade(flag));
}

/// Installs a panic hook (once, process-wide) that flips every live blocker's `active` flag to
/// `false` before running whatever hook was previously installed. This is a fast, best-effort
/// backstop for a panic on the main thread; the watchdog thread ([`watchdog_main`]) is the
/// backstop for everything else, including a hang that never panics at all.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let guard = ACTIVE_FLAGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for weak in guard.iter() {
            if let Some(flag) = weak.upgrade() {
                flag.store(false, Ordering::SeqCst);
            }
        }
        drop(guard);
        previous(info);
    }));
}

/// Handle returned by [`InputBlocker::start`]. Dropping it stops blocking input: it disables the
/// tap, stops its run loop, and joins both the tap thread and the watchdog thread. Never call
/// this from the tap or watchdog thread itself — joining your own thread deadlocks; in practice
/// this is only ever dropped from the thread that called `start` (the main/app thread).
#[must_use = "dropping this immediately stops blocking input"]
pub struct BlockerGuard {
    active: Arc<AtomicBool>,
    handle: TapHandle,
    run_loop: CFRunLoop,
    watchdog_stop: Arc<AtomicBool>,
    tap_thread: Option<JoinHandle<()>>,
    watchdog_thread: Option<JoinHandle<()>>,
}

impl Drop for BlockerGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        self.watchdog_stop.store(true, Ordering::SeqCst);
        self.handle.disable();
        self.run_loop.stop();
        if let Some(t) = self.tap_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.watchdog_thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_keycode_maps_to_escape() {
        assert_eq!(map_key_kind(KEYCODE_ESCAPE), KeyKind::Escape);
    }

    #[test]
    fn space_keycode_maps_to_space() {
        assert_eq!(map_key_kind(KEYCODE_SPACE), KeyKind::Space);
    }

    #[test]
    fn return_keycode_maps_to_return() {
        assert_eq!(map_key_kind(KEYCODE_RETURN), KeyKind::Return);
    }

    #[test]
    fn keypad_enter_keycode_also_maps_to_return() {
        assert_eq!(map_key_kind(KEYCODE_KEYPAD_ENTER), KeyKind::Return);
    }

    #[test]
    fn every_other_keycode_maps_to_other() {
        assert_eq!(map_key_kind(0), KeyKind::Other); // 'a' on QWERTY
        assert_eq!(map_key_kind(122), KeyKind::Other); // e.g. F1
        assert_eq!(map_key_kind(-1), KeyKind::Other);
    }
}
