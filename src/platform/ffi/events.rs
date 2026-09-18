//! Modifier-flag state (`CGEventSourceFlagsState`), used by
//! [`crate::platform::system::option_key_held`] to detect the Option key held at launch
//! (DESIGN.md §6: hold Option while opening Urahafu to get the menu back after hiding the icon).

/// `kCGEventSourceStateCombinedSessionState`: the combined state of all event sources for the
/// current login session (as opposed to just this process's own synthesized events).
const COMBINED_SESSION_STATE: u32 = 0;

/// `kCGEventFlagMaskAlternate`: the bit set in the combined flags state while any Option/Alt key
/// is held.
pub const ALTERNATE_MASK: u64 = 0x0008_0000;

/// The combined modifier-flags bitmask for the current session, straight from
/// `CGEventSourceFlagsState`. Callers test individual bits (e.g. [`ALTERNATE_MASK`]).
#[must_use]
pub fn combined_session_flags() -> u64 {
    // SAFETY: `CGEventSourceFlagsState` takes a plain enum value and has no preconditions; it is
    // safe to call from any thread at any time.
    unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) }
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state_id: u32) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternate_mask_matches_core_graphics_cgeventflags() {
        use core_graphics::event::CGEventFlags;
        assert_eq!(ALTERNATE_MASK, CGEventFlags::CGEventFlagAlternate.bits());
    }
}
