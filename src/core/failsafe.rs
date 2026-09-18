//! Fail-safe auto-unlock timing (DESIGN.md §1, §13).
//!
//! The fail-safe delay is the "the user never gets stuck" guarantee at the pure-logic level: a
//! [`Deadline`] captured when locking starts and never reset. Whatever else happens, once the
//! deadline passes the session ends. The actual last line of defense — the watchdog thread that
//! removes the input tap independently of this state machine — lives in
//! `src/platform/input_blocker.rs`; this module only computes remaining time and the fraction
//! used by the time-remaining bar.

use std::time::{Duration, Instant};

/// The allowed fail-safe delays (DESIGN.md §6: 30/60/90 s, default 60 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailsafeDelay {
    /// Auto-unlock after 30 seconds.
    Seconds30,
    /// Auto-unlock after 60 seconds (the default).
    #[default]
    Seconds60,
    /// Auto-unlock after 90 seconds.
    Seconds90,
}

impl FailsafeDelay {
    /// The delay as a [`Duration`].
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_secs(self.seconds())
    }

    /// The delay in whole seconds.
    #[must_use]
    pub fn seconds(self) -> u64 {
        match self {
            Self::Seconds30 => 30,
            Self::Seconds60 => 60,
            Self::Seconds90 => 90,
        }
    }

    /// Parses a delay from its seconds value; returns `None` for anything other than 30, 60 or
    /// 90 (callers fall back to [`FailsafeDelay::default`] for settings parsing).
    #[must_use]
    pub fn from_seconds(seconds: u64) -> Option<Self> {
        match seconds {
            30 => Some(Self::Seconds30),
            60 => Some(Self::Seconds60),
            90 => Some(Self::Seconds90),
            _ => None,
        }
    }

    /// Starts a [`Deadline`] `self` after `now`.
    #[must_use]
    pub fn deadline_from(self, now: Instant) -> Deadline {
        Deadline::new(now, self.duration())
    }
}

/// A fixed point in time, computed once when input blocking starts, that never moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    at: Instant,
}

impl Deadline {
    /// Creates a deadline `duration` after `started_at`.
    #[must_use]
    pub fn new(started_at: Instant, duration: Duration) -> Self {
        Self {
            at: started_at + duration,
        }
    }

    /// The instant the deadline falls on.
    #[must_use]
    pub fn at(self) -> Instant {
        self.at
    }

    /// Whether `now` is at or past the deadline.
    #[must_use]
    pub fn is_expired(self, now: Instant) -> bool {
        now >= self.at
    }

    /// Time remaining until the deadline, or `Duration::ZERO` if already expired.
    #[must_use]
    pub fn remaining(self, now: Instant) -> Duration {
        self.at.saturating_duration_since(now)
    }

    /// Whole seconds remaining, rounded up so the displayed countdown never reads "0 s" while
    /// input is still blocked (e.g. 59.2 s remaining reads as 60 s).
    #[must_use]
    pub fn remaining_secs_ceil(self, now: Instant) -> u32 {
        let remaining = self.remaining(now);
        let secs = remaining.as_secs();
        let whole = if remaining.subsec_nanos() > 0 {
            secs + 1
        } else {
            secs
        };
        u32::try_from(whole).unwrap_or(u32::MAX)
    }

    /// Fraction of the fail-safe window remaining, in `[0.0, 1.0]`, given the total `duration`
    /// of the window. Used to size the time-remaining bar (DESIGN.md §8: shrinks linearly).
    #[must_use]
    pub fn fraction_remaining(self, now: Instant, duration: Duration) -> f32 {
        if duration.is_zero() {
            return 0.0;
        }
        let remaining = self.remaining(now).as_secs_f32();
        (remaining / duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Whether `now` is within the final emphasis window (last 10 seconds, DESIGN.md §8/§13).
    #[must_use]
    pub fn is_in_final_emphasis(self, now: Instant) -> bool {
        self.remaining(now) <= Duration::from_secs(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_and_seconds_match_design() {
        assert_eq!(FailsafeDelay::Seconds30.seconds(), 30);
        assert_eq!(FailsafeDelay::Seconds60.seconds(), 60);
        assert_eq!(FailsafeDelay::Seconds90.seconds(), 90);
        assert_eq!(FailsafeDelay::Seconds60.duration(), Duration::from_secs(60));
    }

    #[test]
    fn default_is_60_seconds() {
        assert_eq!(FailsafeDelay::default(), FailsafeDelay::Seconds60);
    }

    #[test]
    fn from_seconds_accepts_only_valid_values() {
        assert_eq!(
            FailsafeDelay::from_seconds(30),
            Some(FailsafeDelay::Seconds30)
        );
        assert_eq!(
            FailsafeDelay::from_seconds(60),
            Some(FailsafeDelay::Seconds60)
        );
        assert_eq!(
            FailsafeDelay::from_seconds(90),
            Some(FailsafeDelay::Seconds90)
        );
        assert_eq!(FailsafeDelay::from_seconds(0), None);
        assert_eq!(FailsafeDelay::from_seconds(45), None);
        assert_eq!(FailsafeDelay::from_seconds(120), None);
    }

    #[test]
    fn deadline_exact_boundary_30() {
        let now = Instant::now();
        let deadline = FailsafeDelay::Seconds30.deadline_from(now);
        assert!(!deadline.is_expired(now + Duration::from_secs(29)));
        assert!(deadline.is_expired(now + Duration::from_secs(30)));
    }

    #[test]
    fn deadline_exact_boundary_60() {
        let now = Instant::now();
        let deadline = FailsafeDelay::Seconds60.deadline_from(now);
        assert!(!deadline.is_expired(now + Duration::from_secs(59)));
        assert!(deadline.is_expired(now + Duration::from_secs(60)));
    }

    #[test]
    fn deadline_exact_boundary_90() {
        let now = Instant::now();
        let deadline = FailsafeDelay::Seconds90.deadline_from(now);
        assert!(!deadline.is_expired(now + Duration::from_secs(89)));
        assert!(deadline.is_expired(now + Duration::from_secs(90)));
    }

    #[test]
    fn remaining_never_negative() {
        let now = Instant::now();
        let deadline = FailsafeDelay::Seconds30.deadline_from(now);
        let after = now + Duration::from_secs(60);
        assert_eq!(deadline.remaining(after), Duration::ZERO);
    }

    #[test]
    fn remaining_secs_ceil_rounds_up() {
        let now = Instant::now();
        let deadline = Deadline::new(now, Duration::from_millis(59_200));
        assert_eq!(deadline.remaining_secs_ceil(now), 60);
    }

    #[test]
    fn remaining_secs_ceil_exact_second() {
        let now = Instant::now();
        let deadline = Deadline::new(now, Duration::from_secs(42));
        assert_eq!(deadline.remaining_secs_ceil(now), 42);
    }

    #[test]
    fn fraction_remaining_decreases_linearly() {
        let now = Instant::now();
        let duration = Duration::from_secs(60);
        let deadline = Deadline::new(now, duration);
        assert!((deadline.fraction_remaining(now, duration) - 1.0).abs() < 1e-6);
        assert!(
            (deadline.fraction_remaining(now + Duration::from_secs(30), duration) - 0.5).abs()
                < 1e-3
        );
        assert!(
            (deadline.fraction_remaining(now + Duration::from_secs(60), duration) - 0.0).abs()
                < 1e-6
        );
        assert!(
            (deadline.fraction_remaining(now + Duration::from_secs(90), duration) - 0.0).abs()
                < 1e-6
        );
    }

    #[test]
    fn final_emphasis_window_is_last_ten_seconds() {
        let now = Instant::now();
        let deadline = Deadline::new(now, Duration::from_secs(60));
        assert!(!deadline.is_in_final_emphasis(now + Duration::from_secs(49)));
        assert!(deadline.is_in_final_emphasis(now + Duration::from_secs(50)));
        assert!(deadline.is_in_final_emphasis(now + Duration::from_secs(60)));
    }
}
