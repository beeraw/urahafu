//! The 3-2-1 countdown shown before input blocking starts (DESIGN.md §7).
//!
//! The countdown lasts three seconds, one per digit. Each digit fades and scales in over
//! 150 ms (ease-out, scale 0.92 → 1.0), holds at full opacity, then fades out over the last
//! 200 ms of its second. [`Countdown`] is a pure function of elapsed time: it holds no mutable
//! state itself, so [`crate::core::session::Session`] can simply ask it for a view at any
//! instant.

use std::time::Duration;

use crate::core::easing::{ease_out_cubic, lerp, linear};

/// First digit shown, counting down to 1.
const START_DIGIT: u8 = 3;
/// Duration each digit is shown for, including its fade-in and fade-out.
const DIGIT_DURATION: Duration = Duration::from_millis(1000);
/// Fade-in + scale-in duration at the start of each digit.
const FADE_IN: Duration = Duration::from_millis(150);
/// Fade-out duration at the end of each digit.
const FADE_OUT: Duration = Duration::from_millis(200);
/// Initial scale a digit animates in from.
const START_SCALE: f32 = 0.92;

/// Snapshot of the countdown digit's appearance at a given instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CountdownView {
    /// The digit currently shown (3, 2 or 1).
    pub digit: u8,
    /// Opacity in `[0.0, 1.0]`.
    pub opacity: f32,
    /// Scale factor, animating from 0.92 to 1.0 on entry.
    pub scale: f32,
}

/// Pure timing calculator for the countdown animation.
///
/// `Countdown` has no mutable state: it is a set of constants plus a function from "elapsed
/// time since the countdown started" to a [`CountdownView`].
#[derive(Debug, Default, Clone, Copy)]
pub struct Countdown;

impl Countdown {
    /// Total duration of the countdown (3 seconds: one second per starting digit).
    pub const TOTAL: Duration = Duration::from_millis(3000);

    /// Creates a new countdown timing calculator.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Returns the digit view for `elapsed` time since the countdown started, or `None` once
    /// the countdown is finished (`elapsed >= Countdown::TOTAL`).
    #[must_use]
    pub fn view(&self, elapsed: Duration) -> Option<CountdownView> {
        if elapsed >= Self::TOTAL {
            return None;
        }

        // `elapsed < TOTAL` (3000ms) so this division yields 0, 1 or 2: safe to narrow to u32.
        let index_u128 = elapsed.as_millis() / DIGIT_DURATION.as_millis();
        #[allow(
            clippy::cast_possible_truncation,
            reason = "index_u128 is in 0..3 by construction (elapsed < TOTAL), fits easily in u32"
        )]
        let index = index_u128 as u32;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "index is in 0..3 by construction, fits easily in u8"
        )]
        let digit = START_DIGIT - index as u8;

        let local = elapsed.saturating_sub(DIGIT_DURATION * index);
        let fade_start = DIGIT_DURATION.saturating_sub(FADE_OUT);

        let (opacity, scale) = if local < FADE_IN {
            let t = local.as_secs_f32() / FADE_IN.as_secs_f32();
            let eased = ease_out_cubic(t);
            (eased, lerp(START_SCALE, 1.0, eased))
        } else if local >= fade_start {
            let t = local.saturating_sub(fade_start).as_secs_f32() / FADE_OUT.as_secs_f32();
            (1.0 - linear(t), 1.0)
        } else {
            (1.0, 1.0)
        };

        Some(CountdownView {
            digit,
            opacity,
            scale,
        })
    }

    /// Whether the countdown has finished by `elapsed`.
    #[must_use]
    pub fn is_finished(&self, elapsed: Duration) -> bool {
        elapsed >= Self::TOTAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_three_with_zero_opacity() {
        let view = Countdown::new().view(Duration::ZERO).unwrap();
        assert_eq!(view.digit, 3);
        assert!((view.opacity - 0.0).abs() < 1e-6);
        assert!((view.scale - START_SCALE).abs() < 1e-6);
    }

    #[test]
    fn reaches_full_opacity_after_fade_in() {
        let view = Countdown::new().view(FADE_IN).unwrap();
        assert!((view.opacity - 1.0).abs() < 1e-3);
        assert!((view.scale - 1.0).abs() < 1e-3);
    }

    #[test]
    fn holds_at_full_opacity_mid_second() {
        let view = Countdown::new().view(Duration::from_millis(500)).unwrap();
        assert_eq!(view.digit, 3);
        assert!((view.opacity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fades_out_at_end_of_second() {
        let just_before_next = Duration::from_millis(999);
        let view = Countdown::new().view(just_before_next).unwrap();
        assert_eq!(view.digit, 3);
        assert!(view.opacity < 0.1);
    }

    #[test]
    fn second_digit_starts_at_one_second() {
        let view = Countdown::new().view(Duration::from_millis(1000)).unwrap();
        assert_eq!(view.digit, 2);
        assert!((view.opacity - 0.0).abs() < 1e-6);
    }

    #[test]
    fn third_digit_is_one() {
        let view = Countdown::new().view(Duration::from_millis(2500)).unwrap();
        assert_eq!(view.digit, 1);
    }

    #[test]
    fn finished_exactly_at_three_seconds() {
        let countdown = Countdown::new();
        assert!(!countdown.is_finished(Duration::from_millis(2999)));
        assert!(countdown.is_finished(Duration::from_secs(3)));
        assert!(countdown.view(Duration::from_secs(3)).is_none());
    }

    #[test]
    fn finished_well_past_the_end() {
        let countdown = Countdown::new();
        assert!(countdown.is_finished(Duration::from_secs(10)));
        assert!(countdown.view(Duration::from_secs(10)).is_none());
    }
}
