//! Injectable time source.
//!
//! Every timing decision in [`crate::core`] takes an explicit [`Instant`] rather than reading
//! the system clock itself, so the whole state machine can be driven deterministically in tests.
//! The [`Clock`] trait exists only to let callers obtain that `Instant`: platform code uses
//! [`SystemClock`], tests use [`ManualClock`].

use std::time::{Duration, Instant};

/// A source of [`Instant`] values.
///
/// Implementations must return monotonically non-decreasing values, matching the guarantee of
/// [`Instant::now`].
pub trait Clock {
    /// Returns the current instant.
    fn now(&self) -> Instant;
}

/// A [`Clock`] backed by the real system clock (`Instant::now`).
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A [`Clock`] whose value is set explicitly by tests, never by wall-clock time.
///
/// `ManualClock` starts at an arbitrary but fixed instant obtained once from the real clock (so
/// it is a valid `Instant`), and only moves forward when [`ManualClock::advance`] is called.
#[derive(Debug, Clone)]
pub struct ManualClock {
    now: Instant,
}

impl ManualClock {
    /// Creates a new manual clock starting "now".
    #[must_use]
    pub fn new() -> Self {
        Self {
            now: Instant::now(),
        }
    }

    /// Moves the clock forward by `duration`.
    pub fn advance(&mut self, duration: Duration) {
        self.now += duration;
    }

    /// Sets the clock to `instant` directly.
    pub fn set(&mut self, instant: Instant) {
        self.now = instant;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_advances() {
        let clock = SystemClock;
        let a = clock.now();
        let b = clock.now();
        assert!(b >= a);
    }

    #[test]
    fn manual_clock_only_moves_when_advanced() {
        let mut clock = ManualClock::new();
        let a = clock.now();
        let b = clock.now();
        assert_eq!(a, b);
        clock.advance(Duration::from_secs(1));
        let c = clock.now();
        assert_eq!(c, a + Duration::from_secs(1));
    }

    #[test]
    fn manual_clock_can_be_set_directly() {
        let mut clock = ManualClock::new();
        let target = clock.now() + Duration::from_secs(42);
        clock.set(target);
        assert_eq!(clock.now(), target);
    }
}
