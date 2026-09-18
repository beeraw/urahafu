//! Dead-pixel test cycle (DESIGN.md §8): Space advances Off → Red → Green → Blue → White →
//! Black → Off.
//!
//! `PixelTest` is a tiny, explicit state machine (no timing involved — it only advances on
//! Space) kept separate from [`crate::core::session::Session`] so its five-step cycle and its
//! effect on the step indicator ("2/5") stay independently testable.

use crate::core::color::Rgb;

/// One step of the dead-pixel test cycle, including the "off" (not running) state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PixelTestStep {
    /// Not running: the normal cleaning screen is shown.
    #[default]
    Off,
    /// Step 1/5.
    Red,
    /// Step 2/5.
    Green,
    /// Step 3/5.
    Blue,
    /// Step 4/5.
    White,
    /// Step 5/5.
    Black,
}

impl PixelTestStep {
    /// The solid color shown for this step, or `None` when [`PixelTestStep::Off`] (the caller
    /// should fall back to the session's chosen cleaning color).
    #[must_use]
    pub fn color(self) -> Option<Rgb> {
        match self {
            Self::Off => None,
            Self::Red => Some(Rgb::new(255, 0, 0)),
            Self::Green => Some(Rgb::new(0, 255, 0)),
            Self::Blue => Some(Rgb::new(0, 0, 255)),
            Self::White => Some(Rgb::new(255, 255, 255)),
            Self::Black => Some(Rgb::new(0, 0, 0)),
        }
    }

    /// The `(step, total)` pair shown as "n/5" (DESIGN.md `overlay.pixel_test.step`), or `None`
    /// when off.
    #[must_use]
    pub fn step_indicator(self) -> Option<(u8, u8)> {
        let step = match self {
            Self::Off => return None,
            Self::Red => 1,
            Self::Green => 2,
            Self::Blue => 3,
            Self::White => 4,
            Self::Black => 5,
        };
        Some((step, 5))
    }

    /// The next step in the cycle: Off → Red → Green → Blue → White → Black → Off.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Red,
            Self::Red => Self::Green,
            Self::Green => Self::Blue,
            Self::Blue => Self::White,
            Self::White => Self::Black,
            Self::Black => Self::Off,
        }
    }

    /// Whether the test is currently running (anything but [`PixelTestStep::Off`]).
    #[must_use]
    pub fn is_running(self) -> bool {
        self != Self::Off
    }
}

/// Drives the dead-pixel test cycle via [`PixelTest::advance`] (called on Space).
#[derive(Debug, Default, Clone, Copy)]
pub struct PixelTest {
    step: PixelTestStep,
}

impl PixelTest {
    /// Creates a new, inactive pixel test.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current step.
    #[must_use]
    pub fn step(self) -> PixelTestStep {
        self.step
    }

    /// Advances to the next step (called when Space is pressed).
    pub fn advance(&mut self) {
        self.step = self.step.next();
    }

    /// Resets the test to [`PixelTestStep::Off`] (e.g. when a non-Space key ends the test).
    pub fn reset(&mut self) {
        self.step = PixelTestStep::Off;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_through_all_steps_and_back_to_off() {
        let mut test = PixelTest::new();
        assert_eq!(test.step(), PixelTestStep::Off);

        let expected = [
            PixelTestStep::Red,
            PixelTestStep::Green,
            PixelTestStep::Blue,
            PixelTestStep::White,
            PixelTestStep::Black,
            PixelTestStep::Off,
        ];
        for step in expected {
            test.advance();
            assert_eq!(test.step(), step);
        }
    }

    #[test]
    fn colors_match_design() {
        assert_eq!(PixelTestStep::Red.color(), Some(Rgb::new(255, 0, 0)));
        assert_eq!(PixelTestStep::Green.color(), Some(Rgb::new(0, 255, 0)));
        assert_eq!(PixelTestStep::Blue.color(), Some(Rgb::new(0, 0, 255)));
        assert_eq!(PixelTestStep::White.color(), Some(Rgb::new(255, 255, 255)));
        assert_eq!(PixelTestStep::Black.color(), Some(Rgb::new(0, 0, 0)));
        assert_eq!(PixelTestStep::Off.color(), None);
    }

    #[test]
    fn step_indicator_matches_design() {
        assert_eq!(PixelTestStep::Red.step_indicator(), Some((1, 5)));
        assert_eq!(PixelTestStep::Green.step_indicator(), Some((2, 5)));
        assert_eq!(PixelTestStep::Blue.step_indicator(), Some((3, 5)));
        assert_eq!(PixelTestStep::White.step_indicator(), Some((4, 5)));
        assert_eq!(PixelTestStep::Black.step_indicator(), Some((5, 5)));
        assert_eq!(PixelTestStep::Off.step_indicator(), None);
    }

    #[test]
    fn is_running_is_false_only_when_off() {
        assert!(!PixelTestStep::Off.is_running());
        assert!(PixelTestStep::Red.is_running());
        assert!(PixelTestStep::Black.is_running());
    }

    #[test]
    fn reset_returns_to_off() {
        let mut test = PixelTest::new();
        test.advance();
        test.advance();
        assert_ne!(test.step(), PixelTestStep::Off);
        test.reset();
        assert_eq!(test.step(), PixelTestStep::Off);
    }
}
