//! Hint (and wordmark / secondary line) fade timing (DESIGN.md §8, §13).
//!
//! The hint (and the hold-to-unlock button, which shares this fade curve) is shown for 6 seconds
//! on entry, then fades out over 1.2 s. Any blocked input re-shows it: fade-in 200 ms, hold 2.5 s,
//! fade-out 1.2 s, and the hold is "throttled" — each new input restarts the whole cycle from
//! `now` rather than stacking timers.
//! `HintFader` models this as a small state machine driven by explicit instants, so it needs no
//! background timer of its own.

use std::time::{Duration, Instant};

use crate::core::easing::ease_in_out_cubic;

/// Duration the hint stays fully visible right after the session starts (DESIGN.md §8: 6 s, up
/// from the typed-sequence design's 4 s).
const INITIAL_HOLD: Duration = Duration::from_secs(6);
/// Fade-out duration after the initial hold.
const INITIAL_FADE_OUT: Duration = Duration::from_millis(1200);
/// Fade-in duration when input re-shows the hint.
const RESHOW_FADE_IN: Duration = Duration::from_millis(200);
/// Hold duration when input re-shows the hint.
const RESHOW_HOLD: Duration = Duration::from_millis(2500);
/// Fade-out duration when the re-shown hint disappears again.
const RESHOW_FADE_OUT: Duration = Duration::from_millis(1200);

/// Which segment of the fade timeline the hint is currently in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Fading in from a re-show.
    FadeIn { start: Instant, duration: Duration },
    /// Fully visible for `hold`, then fading out over `fade_out`.
    Hold {
        start: Instant,
        hold: Duration,
        fade_out: Duration,
    },
    /// Fully hidden, waiting for input.
    Hidden,
}

/// Tracks the hint's opacity over time, including re-shows triggered by input.
#[derive(Debug, Clone, Copy)]
pub struct HintFader {
    stage: Stage,
}

impl HintFader {
    /// Creates a fader that starts showing the hint at `now`, fully visible immediately (the
    /// overlay itself fades in as the countdown ends; the hint text appears at full opacity and
    /// holds for 4 s before its own fade-out).
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            stage: Stage::Hold {
                start: now,
                hold: INITIAL_HOLD,
                fade_out: INITIAL_FADE_OUT,
            },
        }
    }

    /// Notifies the fader that a blocked input happened at `now`, re-showing the hint: fade-in
    /// 200 ms, hold 2.5 s, fade-out 1.2 s. Calling this again while already shown restarts the
    /// cycle from `now` (throttled: it does not stack).
    pub fn on_input(&mut self, now: Instant) {
        self.stage = Stage::FadeIn {
            start: now,
            duration: RESHOW_FADE_IN,
        };
    }

    /// Jumps straight to the re-show's fully-visible hold (2.5 s), skipping its 200 ms fade-in —
    /// used when the hint/button is already fully visible for a different reason (the hover
    /// reveal, `core::session`) and only the "hold 2.5 s then fade 1.2 s" tail needs to start.
    pub fn hold_from(&mut self, now: Instant) {
        self.stage = Stage::Hold {
            start: now,
            hold: RESHOW_HOLD,
            fade_out: RESHOW_FADE_OUT,
        };
    }

    /// The current opacity in `[0.0, 1.0]` at `now`.
    #[must_use]
    pub fn opacity(&self, now: Instant) -> f32 {
        match self.stage {
            Stage::FadeIn { start, duration } => {
                let t = elapsed_fraction(start, duration, now);
                ease_in_out_cubic(t)
            }
            Stage::Hold {
                start,
                hold,
                fade_out,
            } => {
                let elapsed = now.saturating_duration_since(start);
                if elapsed < hold {
                    1.0
                } else {
                    let fade_elapsed = elapsed.saturating_sub(hold);
                    let fade_t =
                        elapsed_fraction(start + hold, fade_out, start + hold + fade_elapsed);
                    1.0 - ease_in_out_cubic(fade_t)
                }
            }
            Stage::Hidden => 0.0,
        }
    }

    /// Advances the internal stage machine based on `now`; call this (or rely on
    /// [`HintFader::is_hidden`]) regularly so stage transitions (fade-in → hold+fade-out →
    /// hidden) happen. Idempotent.
    pub fn tick(&mut self, now: Instant) {
        self.stage = match self.stage {
            Stage::FadeIn { start, duration } if now >= start + duration => {
                // Anchor the hold at the exact instant the fade-in completed, not at `now`: a
                // late `tick` call must not push the hold/fade-out window further into the
                // future than it should be.
                Stage::Hold {
                    start: start + duration,
                    hold: RESHOW_HOLD,
                    fade_out: RESHOW_FADE_OUT,
                }
            }
            Stage::Hold {
                start,
                hold,
                fade_out,
            } if now >= start + hold + fade_out => Stage::Hidden,
            other => other,
        };
    }

    /// The next instant at which `opacity` changes discontinuously (a stage transition), if any
    /// animation is still in flight. `None` once the hint is fully hidden and idle.
    #[must_use]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        match self.stage {
            Stage::FadeIn { start, duration } => Some(transition_wake(start + duration, now)),
            Stage::Hold {
                start,
                hold,
                fade_out,
            } => Some(transition_wake(start + hold + fade_out, now)),
            Stage::Hidden => None,
        }
    }

    /// Whether the hint is fully hidden (opacity permanently 0 until the next input).
    #[must_use]
    pub fn is_hidden(&self, now: Instant) -> bool {
        matches!(self.stage, Stage::Hidden) || self.opacity(now) <= 0.0
    }
}

/// The next wake instant given a stage's end instant: immediately if already past it (so the
/// caller can `tick` and transition), otherwise a ~16 ms animation frame ahead (capped at `end`).
fn transition_wake(end: Instant, now: Instant) -> Instant {
    if now >= end {
        now
    } else {
        end.min(now + Duration::from_millis(16))
    }
}

/// Fraction of `duration` elapsed since `start`, as of `now`, clamped to `[0.0, 1.0]`.
fn elapsed_fraction(start: Instant, duration: Duration, now: Instant) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    let elapsed = now.saturating_duration_since(start);
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_show_is_fully_visible() {
        let now = Instant::now();
        let fader = HintFader::new(now);
        assert!((fader.opacity(now) - 1.0).abs() < 1e-6);
        assert!((fader.opacity(now + Duration::from_secs(3)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn initial_hint_fades_out_after_four_seconds() {
        let now = Instant::now();
        let fader = HintFader::new(now);
        let mid_fade = now + INITIAL_HOLD + INITIAL_FADE_OUT / 2;
        let opacity = fader.opacity(mid_fade);
        assert!(opacity > 0.0 && opacity < 1.0);
        let after = now + INITIAL_HOLD + INITIAL_FADE_OUT + Duration::from_millis(1);
        assert!(fader.opacity(after) <= 1e-3);
    }

    #[test]
    fn reshow_fades_in_holds_then_fades_out() {
        let now = Instant::now();
        let mut fader = HintFader::new(now);
        // Let it go fully hidden first.
        fader.tick(now + Duration::from_secs(10));
        assert!(fader.is_hidden(now + Duration::from_secs(10)));

        let input_at = now + Duration::from_secs(20);
        fader.on_input(input_at);
        assert!((fader.opacity(input_at) - 0.0).abs() < 1e-3);

        let mid_fade_in = input_at + RESHOW_FADE_IN / 2;
        let opacity = fader.opacity(mid_fade_in);
        assert!(opacity > 0.0 && opacity < 1.0);

        let fully_in = input_at + RESHOW_FADE_IN;
        assert!((fader.opacity(fully_in) - 1.0).abs() < 1e-3);

        let still_holding = input_at + RESHOW_FADE_IN + Duration::from_secs(1);
        fader.tick(still_holding);
        assert!((fader.opacity(still_holding) - 1.0).abs() < 1e-6);

        let after_all =
            input_at + RESHOW_FADE_IN + RESHOW_HOLD + RESHOW_FADE_OUT + Duration::from_millis(1);
        fader.tick(after_all);
        assert!(fader.is_hidden(after_all));
    }

    #[test]
    fn repeated_input_restarts_the_cycle_instead_of_stacking() {
        let now = Instant::now();
        let mut fader = HintFader::new(now);
        fader.tick(now + Duration::from_secs(10));

        let first_input = now + Duration::from_secs(20);
        fader.on_input(first_input);

        // A second input shortly after should restart the fade-in from scratch.
        let second_input = first_input + Duration::from_millis(50);
        fader.on_input(second_input);
        assert!((fader.opacity(second_input) - 0.0).abs() < 1e-3);

        let fully_in_again = second_input + RESHOW_FADE_IN;
        assert!((fader.opacity(fully_in_again) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn next_wake_is_none_once_hidden() {
        let now = Instant::now();
        let mut fader = HintFader::new(now);
        let far = now + Duration::from_secs(100);
        fader.tick(far);
        assert!(fader.next_wake(far).is_none());
    }

    #[test]
    fn next_wake_is_some_while_animating() {
        let now = Instant::now();
        let fader = HintFader::new(now);
        assert!(fader.next_wake(now).is_some());
    }
}
