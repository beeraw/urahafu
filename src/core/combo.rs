//! The Esc+Return keyboard hold-to-unlock combo (DESIGN.md §8): holding Escape and Return
//! together, and no other key, for the same duration as the button hold unlocks a session exactly
//! like it. Pure, deterministic, driven by explicit `Instant`s like the rest of `core::session`;
//! [`ComboTracker`] carries no character and no physical virtual key code at all — only the
//! narrow [`ComboKey`] identity `core::session` has already reduced a real key event to.
//!
//! ## What counts as "no other key"
//!
//! [`ComboKey::Other`] collapses every key that isn't Escape or Return (including Space, which
//! keeps its own `core::session` behavior and also counts as "another key" for the combo, per
//! `core::session`'s own mapping) into one identity: the tracker only needs to know *whether* any
//! such key is currently held, not which one, so it keeps a count rather than a set.
//!
//! A modifier-only key press/release (`kCGEventFlagsChanged`, [`ComboEvent::ModifierChange`])
//! carries no down/up information at all — macOS reports a flags snapshot, not a press/release
//! pair per key — so it cannot feed a persistent "held" count the way a real key's down/up pair
//! can. Each occurrence is instead treated as an instantaneous interference pulse: any other key
//! going down while the combo is held, including a modifier key, resets its progress (DESIGN.md
//! §8).
//!
//! ## Why a modifier pulse "poisons" the combo until a fresh press
//!
//! A plain key's interference (e.g. pressing `a` while holding Esc+Return) clears itself the
//! moment that key is released: the tracker sees its [`ComboEvent::Up`] and the held-"other"
//! count returns to zero, at which point Escape and Return alone are down again — a state the
//! tracker can plainly observe, so the hold restarts immediately. A modifier pulse has no such
//! observable "released" moment (no matching `Up` is ever sent for it), so if the hold were
//! allowed to restart the instant the pulse passes, a stray Shift tap during an otherwise-clean
//! hold would cost at most one animation frame — not the full reset that DESIGN.md §8 calls for,
//! where the combo only (re)starts once only Esc + Return are down. Instead, a modifier pulse
//! poisons the combo: no new hold starts until Escape or Return is actually released and
//! re-pressed, i.e. until the tracker observes a fresh transition into "only Esc + Return are
//! down" from a state where that wasn't true. This is the one place this module's behavior goes
//! beyond a literal reading of DESIGN.md §8: a modifier pulse has no natural "released" signal to
//! key a reset off, so treating it as poisoning the combo until a fresh key-down is the
//! interpretation that keeps a stray modifier tap from cheapening the 2-second hold into an
//! effectively instant gesture.
//!
//! ## Key-repeat
//!
//! Autorepeat is filtered out one level up, in `core::session::InputEvent` -> [`ComboEvent`]
//! translation (`core::session`'s own `combo_event_for`): a repeated `KeyDown` for an
//! already-held key carries nothing new for this tracker (the key was already counted as held by
//! its first, non-repeat `KeyDown`), so it is never turned into a [`ComboEvent`] at all.

use std::time::Instant;

use crate::core::hold_ring::HoldRing;

/// A layout-independent key identity the combo tracker distinguishes — see the module docs for
/// why [`ComboKey::Other`] collapses everything but Escape/Return into one identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboKey {
    /// The Escape key.
    Escape,
    /// The Return key (including the numeric keypad's Enter key, which the platform layer maps
    /// to Return before this point — see `core::session::KeyKind::Return`).
    Return,
    /// Every other key, Space included.
    Other,
}

/// One event fed to [`ComboTracker::handle_event`]; see the module docs for what does and does
/// not reach this type (autorepeat is filtered out before this point).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComboEvent {
    /// A key went down.
    Down(ComboKey),
    /// A key was released.
    Up(ComboKey),
    /// A modifier key's flags changed; see the module docs for why this carries no key identity
    /// or down/up information.
    ModifierChange,
}

/// Tracks whether Escape and Return, and only Escape and Return, are currently held down, and
/// drives a [`HoldRing`] accordingly — the pure state machine behind the Esc+Return unlock combo.
#[derive(Debug, Clone, Copy, Default)]
pub struct ComboTracker {
    escape_down: bool,
    return_down: bool,
    /// How many non-Escape/Return keys are currently held down (see the module docs: Space counts
    /// as "other" here too). A count, not a set — [`ComboKey::Other`] collapses every such key to
    /// one identity, so this only ever needs to distinguish "zero" from "more than zero".
    other_down: u32,
    /// Set by a modifier pulse while the combo was clean; see the module docs. Cleared the next
    /// time the combo becomes unclean (Escape or Return released, or another key held), so the
    /// next transition back to "only Esc + Return down" is a fresh, unpoisoned start.
    poisoned: bool,
    ring: HoldRing,
}

impl ComboTracker {
    /// A tracker with nothing held and no progress — the state at the start of every
    /// [`crate::core::session::SessionPhase::Locked`] period.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one event at `now`, updating the held-key bookkeeping and syncing the hold ring.
    pub fn handle_event(&mut self, event: ComboEvent, now: Instant) {
        match event {
            ComboEvent::Down(ComboKey::Escape) => self.escape_down = true,
            ComboEvent::Up(ComboKey::Escape) => self.escape_down = false,
            ComboEvent::Down(ComboKey::Return) => self.return_down = true,
            ComboEvent::Up(ComboKey::Return) => self.return_down = false,
            ComboEvent::Down(ComboKey::Other) => self.other_down += 1,
            ComboEvent::Up(ComboKey::Other) => {
                self.other_down = self.other_down.saturating_sub(1);
            }
            ComboEvent::ModifierChange => {
                if self.is_clean() {
                    self.poisoned = true;
                }
            }
        }
        self.sync(now);
    }

    /// Whether exactly Escape and Return are currently down, and nothing else.
    fn is_clean(&self) -> bool {
        self.escape_down && self.return_down && self.other_down == 0
    }

    /// Starts, drains or leaves the ring alone to match the current held-key state, per the
    /// module docs.
    fn sync(&mut self, now: Instant) {
        if !self.is_clean() {
            self.poisoned = false;
            self.ring.release(now);
        } else if self.poisoned {
            self.ring.release(now);
        } else {
            self.ring.start(now);
        }
    }

    /// Advances the ring's time-based transitions; call this on every
    /// [`crate::core::session::Session::tick`] while [`crate::core::session::SessionPhase::Locked`].
    pub fn advance(&mut self, now: Instant) {
        self.ring.advance(now);
    }

    /// A copy of the underlying [`HoldRing`], for callers that need its full state (e.g. deciding
    /// whether the unlock button should render at full opacity, or whether anything is animating).
    #[must_use]
    pub fn ring(&self) -> HoldRing {
        self.ring
    }

    /// Whether the combo's completion scale pulse is currently playing.
    #[must_use]
    pub fn is_completing(&self) -> bool {
        self.ring.is_completing()
    }

    /// Whether the combo's completion scale pulse has fully played out at `now` — the instant the
    /// caller should unlock, exactly like [`HoldRing::completion_elapsed`].
    #[must_use]
    pub fn completion_elapsed(&self, now: Instant) -> bool {
        self.ring.completion_elapsed(now)
    }

    /// The combo ring's fill fraction, `[0.0, 1.0]`, at `now`.
    #[must_use]
    pub fn progress(&self, now: Instant) -> f32 {
        self.ring.progress(now)
    }

    /// The combo's control scale multiplier at `now`, exactly like [`HoldRing::scale`].
    #[must_use]
    pub fn scale(&self, now: Instant) -> f32 {
        self.ring.scale(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hold_ring::{COMPLETION_SCALE_PULSE, HOLD_DRAIN, HOLD_DURATION};
    use std::time::Duration;

    #[test]
    fn both_keys_down_starts_the_hold() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        assert!(combo.progress(now).abs() < 1e-6, "one key alone: no hold");
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        let mid = now + HOLD_DURATION / 2;
        assert!((combo.progress(mid) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn order_of_the_two_keys_does_not_matter() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        let mid = now + HOLD_DURATION / 2;
        assert!((combo.progress(mid) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn full_hold_completes() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        combo.advance(now + HOLD_DURATION);
        assert!(combo.is_completing());
        assert!(combo.completion_elapsed(now + HOLD_DURATION + COMPLETION_SCALE_PULSE));
    }

    #[test]
    fn releasing_either_key_early_drains_the_progress() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        let mid = now + HOLD_DURATION / 2;
        let progress_before = combo.progress(mid);
        assert!(progress_before > 0.0);

        combo.handle_event(ComboEvent::Up(ComboKey::Escape), mid);
        assert!((combo.progress(mid) - progress_before).abs() < 1e-3);

        combo.advance(mid + HOLD_DRAIN);
        assert!(combo.progress(mid + HOLD_DRAIN).abs() < 1e-6);

        // Escape still up: re-pressing Return alone must not resume the hold.
        combo.advance(now + HOLD_DURATION * 3);
        assert!(!combo.is_completing());
    }

    #[test]
    fn an_extra_key_down_resets_progress_and_release_lets_it_restart() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        let mid = now + HOLD_DURATION / 2;
        assert!(combo.progress(mid) > 0.0);

        combo.handle_event(ComboEvent::Down(ComboKey::Other), mid);
        combo.advance(mid + HOLD_DRAIN);
        assert!(combo.progress(mid + HOLD_DRAIN).abs() < 1e-6);

        // Releasing the extra key restores "only Esc + Return down": the hold restarts.
        let released_at = mid + HOLD_DRAIN + Duration::from_millis(1);
        combo.handle_event(ComboEvent::Up(ComboKey::Other), released_at);
        let later = released_at + HOLD_DURATION / 2;
        assert!(combo.progress(later) > 0.0);
    }

    #[test]
    fn a_modifier_change_resets_progress_and_blocks_restart_until_a_fresh_press() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        let mid = now + HOLD_DURATION / 2;
        assert!(combo.progress(mid) > 0.0);

        combo.handle_event(ComboEvent::ModifierChange, mid);
        combo.advance(mid + HOLD_DRAIN);
        assert!(combo.progress(mid + HOLD_DRAIN).abs() < 1e-6);

        // Escape and Return are still (physically) down the whole time, and no key ever went up:
        // the combo must stay blocked, unlike the plain extra-key case above.
        let much_later = mid + HOLD_DURATION * 3;
        combo.advance(much_later);
        assert!(
            combo.progress(much_later).abs() < 1e-6,
            "a modifier pulse must not let the hold silently resume while poisoned"
        );
        assert!(!combo.is_completing());

        // Releasing and re-pressing Return clears the poison and lets a fresh hold start.
        combo.handle_event(ComboEvent::Up(ComboKey::Return), much_later);
        let repress_at = much_later + Duration::from_millis(1);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), repress_at);
        let after_repress = repress_at + HOLD_DURATION / 2;
        assert!(combo.progress(after_repress) > 0.0);
    }

    #[test]
    fn fresh_press_after_release_is_a_clean_new_hold() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        let released_at = now + Duration::from_millis(10);
        combo.handle_event(ComboEvent::Up(ComboKey::Escape), released_at);
        combo.handle_event(ComboEvent::Up(ComboKey::Return), released_at);

        // Let the release's drain finish, so the re-press starts from an empty ring.
        let ring_idle_at = released_at + HOLD_DRAIN;
        combo.advance(ring_idle_at);

        let restart = ring_idle_at + Duration::from_millis(1);
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), restart);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), restart);
        let mid = restart + HOLD_DURATION / 2;
        assert!((combo.progress(mid) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn re_press_during_drain_resumes_instead_of_getting_stuck() {
        let now = Instant::now();
        let mut combo = ComboTracker::new();
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), now);
        combo.handle_event(ComboEvent::Down(ComboKey::Return), now);
        let slip = now + HOLD_DURATION / 2;
        combo.handle_event(ComboEvent::Up(ComboKey::Escape), slip);
        let re_press = slip + Duration::from_millis(20);
        combo.handle_event(ComboEvent::Down(ComboKey::Escape), re_press);
        // No further event arrives (autorepeat is filtered): the hold must still complete.
        let done = re_press + HOLD_DURATION;
        combo.advance(done);
        assert!(combo.is_completing());
    }
}
