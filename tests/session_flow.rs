//! Integration tests driving whole [`urahafu::core::session::Session`] flows with manual time
//! arithmetic — no real sleeping, ever.

// This whole file is test code (an integration test binary), so the crate's `unwrap`/`expect`
// ban does not apply here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use urahafu::core::color::CleaningColor;
use urahafu::core::countdown::Countdown;
use urahafu::core::failsafe::FailsafeDelay;
use urahafu::core::i18n::Language;
use urahafu::core::layout::Point;
use urahafu::core::session::{
    EndReason, InputEvent, KeyKind, Session, SessionCommand, SessionConfig, SessionPhase,
};

/// A generous hit target so tests can drive the hold without fussing over exact geometry.
const TARGET_CENTER: Point = Point { x: 720.0, y: 800.0 };
const TARGET_RADIUS: f32 = 22.0;
const TARGET_HIT_RADIUS: f32 = 32.0;
const HOLD_DURATION: Duration = Duration::from_millis(2000);
const COMPLETION_SCALE_PULSE: Duration = Duration::from_millis(150);
const UNLOCK_FADE: Duration = Duration::from_millis(300);

fn config(keyboard_only: bool, failsafe: FailsafeDelay) -> SessionConfig {
    SessionConfig {
        color: CleaningColor::Black,
        failsafe,
        keyboard_only,
        language: Language::ENGLISH,
    }
}

/// Drives the countdown to completion and returns the session plus the instant it became
/// [`SessionPhase::Locked`], with the unlock target already set.
fn lock(session: &mut Session, start: Instant) -> Instant {
    session.set_unlock_target(TARGET_CENTER, TARGET_RADIUS, TARGET_HIT_RADIUS);
    let locked_at = start + Countdown::TOTAL;
    let commands = session.tick(locked_at);
    assert_eq!(commands, vec![SessionCommand::BlockInputs]);
    assert_eq!(session.phase(), SessionPhase::Locked);
    locked_at
}

fn press(session: &mut Session, now: Instant) -> Vec<SessionCommand> {
    session.handle_input(
        InputEvent::PointerDown {
            x: TARGET_CENTER.x,
            y: TARGET_CENTER.y,
        },
        now,
    )
}

fn release(session: &mut Session, now: Instant) -> Vec<SessionCommand> {
    session.handle_input(
        InputEvent::PointerUp {
            x: TARGET_CENTER.x,
            y: TARGET_CENTER.y,
        },
        now,
    )
}

fn press_space(session: &mut Session, now: Instant) -> Vec<SessionCommand> {
    session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Space,
            repeat: false,
        },
        now,
    )
}

/// Holds the unlock button from `locked_at` for the full 2 s, returning the instant the
/// completion scale pulse started.
fn hold_to_completion(session: &mut Session, locked_at: Instant) -> Instant {
    press(session, locked_at);
    let at_2s = locked_at + HOLD_DURATION;
    session.tick(at_2s);
    at_2s
}

#[test]
fn happy_path_full_unlock_flow() {
    let start = Instant::now();
    let mut session = Session::new(config(false, FailsafeDelay::Seconds60), start);

    // Countdown: nothing happens until it elapses.
    assert_eq!(session.phase(), SessionPhase::Countdown);
    assert!(session.tick(start + Duration::from_millis(500)).is_empty());

    let locked_at = lock(&mut session, start);

    let pulse_start = hold_to_completion(&mut session, locked_at);
    assert_eq!(
        session.phase(),
        SessionPhase::Locked,
        "the completion scale pulse plays first"
    );

    let after_pulse = pulse_start + COMPLETION_SCALE_PULSE;
    let commands = session.tick(after_pulse);
    assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
    assert_eq!(session.phase(), SessionPhase::Unlocking);

    // Inputs must already be released at this point (ReleaseInputs was returned when entering
    // Unlocking), well before the overlay visually finishes fading.
    let end = after_pulse + UNLOCK_FADE;
    let commands = session.tick(end);
    assert_eq!(commands, vec![SessionCommand::Close]);
    assert_eq!(session.phase(), SessionPhase::Finished(EndReason::Unlocked));
    assert!(session.next_wake(end).is_none());
}

#[test]
fn cancel_during_countdown_never_blocks_inputs() {
    let start = Instant::now();
    let mut session = Session::new(config(false, FailsafeDelay::Seconds30), start);

    let commands = session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Escape,
            repeat: false,
        },
        start + Duration::from_millis(500),
    );
    assert_eq!(commands, vec![SessionCommand::Close]);
    assert_eq!(
        session.phase(),
        SessionPhase::Finished(EndReason::Cancelled)
    );

    // BlockInputs must never have been issued: ticking well past the countdown's natural end
    // must not retroactively lock anything.
    let commands = session.tick(start + Countdown::TOTAL + Duration::from_secs(5));
    assert!(commands.is_empty());
}

#[test]
fn failsafe_unlocks_even_with_no_hold_at_all() {
    let start = Instant::now();
    let mut session = Session::new(config(false, FailsafeDelay::Seconds30), start);
    let locked_at = lock(&mut session, start);

    // No input at all; just let the deadline arrive.
    let deadline = locked_at + Duration::from_secs(30);
    let commands = session.tick(deadline);
    assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
    assert_eq!(session.phase(), SessionPhase::Unlocking);

    let close_at = deadline + UNLOCK_FADE + Duration::from_millis(50);
    let commands = session.tick(close_at);
    assert_eq!(commands, vec![SessionCommand::Close]);
    assert_eq!(session.phase(), SessionPhase::Finished(EndReason::Failsafe));
}

#[test]
fn dead_pixel_test_cycles_through_a_locked_session_without_unlocking() {
    let start = Instant::now();
    let mut session = Session::new(config(false, FailsafeDelay::Seconds90), start);
    let locked_at = lock(&mut session, start);

    for _ in 0..5 {
        press_space(&mut session, locked_at);
    }
    // Five presses: Red, Green, Blue, White, Black.
    assert_eq!(
        session.view(locked_at).background,
        urahafu::core::color::Rgb::new(0, 0, 0)
    );
    assert_eq!(session.phase(), SessionPhase::Locked);

    // A sixth press cycles back to the normal cleaning color.
    press_space(&mut session, locked_at);
    assert_eq!(
        session.view(locked_at).background,
        CleaningColor::Black.rgb()
    );
    assert_eq!(session.phase(), SessionPhase::Locked);
}

#[test]
fn keyboard_only_session_locks_and_unlocks_like_a_normal_one() {
    let start = Instant::now();
    let mut session = Session::new(config(true, FailsafeDelay::Seconds60), start);
    assert!(session.view(start).keyboard_only);

    let locked_at = lock(&mut session, start);
    assert!(session.view(locked_at).keyboard_only);

    let pulse_start = hold_to_completion(&mut session, locked_at);
    session.tick(pulse_start + COMPLETION_SCALE_PULSE);
    assert_eq!(session.phase(), SessionPhase::Unlocking);
}

#[test]
fn early_release_never_unlocks_even_after_the_original_two_seconds_pass() {
    let start = Instant::now();
    let mut session = Session::new(config(false, FailsafeDelay::Seconds90), start);
    let locked_at = lock(&mut session, start);

    press(&mut session, locked_at);
    let mid_hold = locked_at + HOLD_DURATION / 2;
    release(&mut session, mid_hold);

    // Wall-clock time keeps passing, but the hold was cancelled: this must never transition to
    // Unlocking on its own past the original 2 s mark.
    let past_original_two_seconds = locked_at + HOLD_DURATION + Duration::from_millis(1);
    session.tick(past_original_two_seconds);
    assert_eq!(session.phase(), SessionPhase::Locked);
}

#[test]
fn next_wake_schedule_never_lags_behind_a_pending_transition() {
    let start = Instant::now();
    let mut session = Session::new(config(false, FailsafeDelay::Seconds30), start);
    session.set_unlock_target(TARGET_CENTER, TARGET_RADIUS, TARGET_HIT_RADIUS);

    // Drive the whole session purely off `next_wake`, never overshooting into a state where a
    // transition was already due but unreported.
    let mut now = start;
    let mut iterations = 0;
    let mut pressed = false;
    while let Some(wake) = session.next_wake(now) {
        assert!(wake >= now, "next_wake must never be in the past");
        now = wake;
        session.tick(now);
        if session.phase() == SessionPhase::Locked && !pressed {
            // Start (and hold) the button on the first Locked observation so the test
            // terminates instead of waiting the full fail-safe delay.
            press(&mut session, now);
            pressed = true;
        }
        iterations += 1;
        assert!(iterations < 10_000, "next_wake loop did not converge");
    }
    assert!(matches!(session.phase(), SessionPhase::Finished(_)));
}

/// End-to-end: holding Escape and Return together, and no other key, for the same 2 s duration as
/// the button unlocks a session exactly like the button does (DESIGN.md §8) — never touching
/// the pointer at all.
#[test]
fn escape_and_return_combo_unlocks_the_session_like_the_button() {
    let start = Instant::now();
    let mut session = Session::new(config(false, FailsafeDelay::Seconds60), start);
    let locked_at = lock(&mut session, start);

    session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Escape,
            repeat: false,
        },
        locked_at,
    );
    session.handle_input(
        InputEvent::KeyDown {
            kind: KeyKind::Return,
            repeat: false,
        },
        locked_at,
    );

    let at_2s = locked_at + HOLD_DURATION;
    assert!(
        session.tick(at_2s).is_empty(),
        "the completion scale pulse plays first"
    );
    assert_eq!(session.phase(), SessionPhase::Locked);

    let after_pulse = at_2s + COMPLETION_SCALE_PULSE;
    let commands = session.tick(after_pulse);
    assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
    assert_eq!(session.phase(), SessionPhase::Unlocking);

    let end = after_pulse + UNLOCK_FADE;
    let commands = session.tick(end);
    assert_eq!(commands, vec![SessionCommand::Close]);
    assert_eq!(session.phase(), SessionPhase::Finished(EndReason::Unlocked));
}
