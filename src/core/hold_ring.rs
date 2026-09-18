//! Shared "press-and-hold for a fixed duration" progress-ring state machine (DESIGN.md §8),
//! reused by both ways a session can be unlocked: the on-screen ✕ button's pointer hold
//! (`core::session`) and the Esc+Return keyboard combo (`core::combo`, DESIGN.md §8). Both
//! must show the exact same visual feedback (a ring that fills over 2.0 s, drains over 200 ms on
//! an early release, and a 150 ms completion scale pulse before the caller unlocks), so that
//! timing/animation logic lives here once instead of being duplicated between the two callers.

use std::time::{Duration, Instant};

use crate::core::easing::ease_out_cubic;

/// How long a hold must be sustained to complete (DESIGN.md §8/§13).
pub const HOLD_DURATION: Duration = Duration::from_millis(2000);
/// How long the progress ring takes to drain back to 0 after an early release.
pub const HOLD_DRAIN: Duration = Duration::from_millis(200);
/// How long the completion scale pulse (1.0 -> 1.1) lasts before the caller should treat the hold
/// as done and act on it (e.g. transition to unlocking).
pub const COMPLETION_SCALE_PULSE: Duration = Duration::from_millis(150);
/// The amount the button scales up by, at the peak of [`COMPLETION_SCALE_PULSE`].
pub const COMPLETION_SCALE_DELTA: f32 = 0.1;

/// One press-and-hold gesture's state, driven entirely by explicit `Instant`s (no internal timer).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum HoldRing {
    /// Not held, ring at 0.
    #[default]
    Idle,
    /// Held continuously since `start`; progress increases linearly toward [`HOLD_DURATION`].
    Holding {
        /// When the hold began.
        start: Instant,
    },
    /// Released (or otherwise broken) early: the ring drains from `from` to 0 over
    /// [`HOLD_DRAIN`], starting at `start`.
    Draining {
        /// When the drain began.
        start: Instant,
        /// The progress fraction the ring was at when the drain began.
        from: f32,
    },
    /// The hold reached [`HOLD_DURATION`] at `start`: the caller's control scale-pulses for
    /// [`COMPLETION_SCALE_PULSE`] before [`HoldRing::completion_elapsed`] tells the caller to act.
    Completing {
        /// When the completion pulse began.
        start: Instant,
    },
}

impl HoldRing {
    /// Starts the hold at `now`. From [`HoldRing::Idle`] the ring fills from 0; a re-press while
    /// [`HoldRing::Draining`] resumes filling from the ring's current (partly drained) progress, so
    /// a finger that slips for a moment doesn't leave the control held with nothing happening
    /// (no further event would arrive to restart it once the drain finished: the pointer may stay
    /// still, and key autorepeat is filtered out). Short accidental taps still never unlock: each
    /// drain between taps loses more than a tap gains. A no-op while already
    /// [`HoldRing::Holding`] or [`HoldRing::Completing`].
    pub fn start(&mut self, now: Instant) {
        match *self {
            Self::Idle => *self = Self::Holding { start: now },
            Self::Draining { .. } => {
                let already_filled = HOLD_DURATION.mul_f32(self.progress(now));
                let start = now.checked_sub(already_filled).unwrap_or(now);
                *self = Self::Holding { start };
            }
            Self::Holding { .. } | Self::Completing { .. } => {}
        }
    }

    /// Releases (or otherwise breaks) the hold at `now`: if currently [`HoldRing::Holding`],
    /// starts draining the ring back to 0 from its current progress over [`HOLD_DRAIN`]; a no-op
    /// in every other state — in particular, once [`HoldRing::Completing`], the gesture can no
    /// longer be cancelled (mirrors the unlock button's own completion pulse).
    pub fn release(&mut self, now: Instant) {
        if let Self::Holding { start } = *self {
            let progress = progress_at(start, now);
            *self = Self::Draining {
                start: now,
                from: progress,
            };
        }
    }

    /// Advances time-based transitions: [`HoldRing::Holding`] -> [`HoldRing::Completing`] once
    /// [`HOLD_DURATION`] has elapsed, [`HoldRing::Draining`] -> [`HoldRing::Idle`] once
    /// [`HOLD_DRAIN`] has elapsed. Does not itself signal completion — call
    /// [`HoldRing::completion_elapsed`] for that, typically right after calling this.
    pub fn advance(&mut self, now: Instant) {
        match *self {
            Self::Holding { start } if now.saturating_duration_since(start) >= HOLD_DURATION => {
                *self = Self::Completing { start: now };
            }
            Self::Draining { start, .. } if now.saturating_duration_since(start) >= HOLD_DRAIN => {
                *self = Self::Idle;
            }
            _ => {}
        }
    }

    /// Whether the ring is anywhere but [`HoldRing::Idle`] (used to decide whether the caller
    /// needs to keep animating/waking up, and whether the unlock button should render at full
    /// opacity).
    #[must_use]
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// Whether the completion scale pulse is currently playing (has reached [`HOLD_DURATION`] but
    /// not yet [`COMPLETION_SCALE_PULSE`] past that).
    #[must_use]
    pub fn is_completing(&self) -> bool {
        matches!(self, Self::Completing { .. })
    }

    /// Whether the completion scale pulse has fully played out at `now` — the instant the caller
    /// should act on the completed hold (e.g. transition to unlocking). `false` in every other
    /// state, including [`HoldRing::Completing`] before the pulse has finished.
    #[must_use]
    pub fn completion_elapsed(&self, now: Instant) -> bool {
        matches!(*self, Self::Completing { start } if now.saturating_duration_since(start) >= COMPLETION_SCALE_PULSE)
    }

    /// The ring's fill fraction, `[0.0, 1.0]`, at `now`.
    #[must_use]
    pub fn progress(&self, now: Instant) -> f32 {
        match *self {
            Self::Idle => 0.0,
            Self::Holding { start } => progress_at(start, now),
            Self::Draining { start, from } => from * (1.0 - fraction(start, HOLD_DRAIN, now)),
            Self::Completing { .. } => 1.0,
        }
    }

    /// The control's scale multiplier at `now` (1.0 normally; scales up to
    /// `1.0 + COMPLETION_SCALE_DELTA` during the completion pulse, ease-out).
    #[must_use]
    pub fn scale(&self, now: Instant) -> f32 {
        if let Self::Completing { start } = *self {
            let t = fraction(start, COMPLETION_SCALE_PULSE, now);
            1.0 + COMPLETION_SCALE_DELTA * ease_out_cubic(t)
        } else {
            1.0
        }
    }
}

/// Fraction of `duration` elapsed since `start`, as of `now`, clamped to `[0.0, 1.0]`.
fn fraction(start: Instant, duration: Duration, now: Instant) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    let elapsed = now.saturating_duration_since(start);
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

fn progress_at(start: Instant, now: Instant) -> f32 {
    fraction(start, HOLD_DURATION, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_ring_has_zero_progress() {
        let now = Instant::now();
        let ring = HoldRing::Idle;
        assert!(ring.progress(now).abs() < 1e-9);
        assert!(!ring.is_active());
    }

    #[test]
    fn holding_progresses_linearly_to_completion() {
        let start = Instant::now();
        let mut ring = HoldRing::Idle;
        ring.start(start);
        let mid = start + HOLD_DURATION / 2;
        assert!((ring.progress(mid) - 0.5).abs() < 1e-3);

        ring.advance(start + HOLD_DURATION);
        assert!(ring.is_completing());
        assert!(!ring.completion_elapsed(start + HOLD_DURATION));

        let after_pulse = start + HOLD_DURATION + COMPLETION_SCALE_PULSE;
        assert!(ring.completion_elapsed(after_pulse));
    }

    #[test]
    fn re_press_while_idle_does_nothing_special() {
        let start = Instant::now();
        let mut ring = HoldRing::Idle;
        ring.start(start);
        ring.start(start + Duration::from_millis(1));
        // Still counts from the first `start`, not restarted.
        assert!(matches!(ring, HoldRing::Holding { start: s } if s == start));
    }

    #[test]
    fn release_before_completion_drains() {
        let start = Instant::now();
        let mut ring = HoldRing::Idle;
        ring.start(start);
        let mid = start + HOLD_DURATION / 2;
        ring.release(mid);
        let progress_at_release = ring.progress(mid);
        assert!(progress_at_release > 0.0 && progress_at_release < 1.0);

        let after_drain = mid + HOLD_DRAIN;
        ring.advance(after_drain);
        assert_eq!(ring, HoldRing::Idle);
        assert!(ring.progress(after_drain).abs() < 1e-6);
    }

    #[test]
    fn re_press_during_drain_resumes_from_the_current_progress() {
        let start = Instant::now();
        let mut ring = HoldRing::Idle;
        ring.start(start);
        let mid = start + HOLD_DURATION / 2;
        ring.release(mid);
        // Halfway through the drain, the ring is back down to a quarter.
        let re_press = mid + HOLD_DRAIN / 2;
        ring.start(re_press);
        assert!(matches!(ring, HoldRing::Holding { .. }));
        assert!((ring.progress(re_press) - 0.25).abs() < 1e-2);
        // ...and keeps filling from there, completing once the remaining 75% has elapsed.
        let done = re_press + HOLD_DURATION.mul_f32(0.75) + Duration::from_millis(5);
        ring.advance(done);
        assert!(ring.is_completing());
    }

    #[test]
    fn quick_repeated_taps_never_complete() {
        // 150 ms taps separated by 50 ms gaps, for far longer than a full hold.
        let mut now = Instant::now();
        let mut ring = HoldRing::Idle;
        for _ in 0..100 {
            ring.start(now);
            now += Duration::from_millis(150);
            ring.advance(now);
            ring.release(now);
            now += Duration::from_millis(50);
            ring.advance(now);
            assert!(!ring.is_completing(), "taps must never add up to an unlock");
        }
    }

    #[test]
    fn release_once_completing_does_nothing() {
        let start = Instant::now();
        let mut ring = HoldRing::Idle;
        ring.start(start);
        ring.advance(start + HOLD_DURATION);
        assert!(ring.is_completing());
        ring.release(start + HOLD_DURATION + Duration::from_millis(1));
        assert!(ring.is_completing(), "the completion pulse is atomic");
    }

    #[test]
    fn scale_peaks_during_the_completion_pulse_only() {
        let start = Instant::now();
        let mut ring = HoldRing::Idle;
        assert!((ring.scale(start) - 1.0).abs() < 1e-6);
        ring.start(start);
        assert!((ring.scale(start + HOLD_DURATION / 2) - 1.0).abs() < 1e-6);
        ring.advance(start + HOLD_DURATION);
        let mid_pulse = start + HOLD_DURATION + COMPLETION_SCALE_PULSE / 2;
        assert!(ring.scale(mid_pulse) > 1.0);
    }
}
