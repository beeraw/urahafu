//! The cleaning session state machine (`docs/ARCHITECTURE.md` "Core types", DESIGN.md §8).
//!
//! [`Session`] is the single source of truth for "what should be on screen and what should the
//! platform layer do about input blocking", driven entirely by explicit `Instant`s passed in by
//! the caller — it never reads a clock itself. The renderer and input blocker are expected to
//! call [`Session::handle_input`] on every event, [`Session::tick`] on every wake-up (scheduled
//! via [`Session::next_wake`]), and [`Session::view`] whenever they need to redraw.
//!
//! ## Hold-to-unlock, not a typed sequence
//!
//! Unlocking no longer matches typed characters against a target word: the user presses and
//! holds an on-screen ✕ button for 2 seconds — or, per DESIGN.md §8, holds Escape and
//! Return together (and no other key) for the same duration, via [`crate::core::combo`].
//! [`InputEvent`] carries no character at all — only pointer geometry (in the coordinate
//! convention documented on the type) and, for keyboard events, the layout-independent
//! [`KeyKind`] a physical virtual key code was reduced to (`Escape`/`Return`/`Space`/`Other`),
//! never a character. [`Session::set_unlock_target`] is how the platform layer tells this state
//! machine where the button currently is (its geometry is a rendering/layout concern, computed by
//! `core::layout` and the app layer — not decided here).

use std::time::{Duration, Instant};

use crate::core::color::{CleaningColor, Rgb, ink_for};
use crate::core::combo::{ComboEvent, ComboKey, ComboTracker};
use crate::core::countdown::{Countdown, CountdownView};
use crate::core::failsafe::{Deadline, FailsafeDelay};
use crate::core::hint::HintFader;
use crate::core::hold_ring::HoldRing;
use crate::core::i18n::Language;
use crate::core::layout::Point;
use crate::core::pixel_test::PixelTest;

/// Overlay fade-out duration once unlocking begins (fail-safe, or right after the completion
/// scale pulse).
const UNLOCK_FADE: Duration = Duration::from_millis(300);
/// Animation-frame cadence used while something is actively animating.
const ANIMATION_FRAME: Duration = Duration::from_millis(16);
/// Radius (in the same points as [`InputEvent`]'s pointer coordinates) around the unlock
/// button's center within which the pointer being present counts as "hovering" it.
const HOVER_RADIUS: f32 = 160.0;
/// Fade-in duration for the hint/button when the hover reveal first triggers.
const HOVER_FADE_IN: Duration = Duration::from_millis(200);
/// Period of the countdown-phase button's gentle pulse (opacity 60% <-> 100%).
const COUNTDOWN_PULSE_PERIOD: Duration = Duration::from_millis(1200);
/// Opacity range of the countdown-phase button pulse.
const COUNTDOWN_PULSE_MIN_OPACITY: f32 = 0.6;
const COUNTDOWN_PULSE_MAX_OPACITY: f32 = 1.0;

/// A layout-independent keyboard key identity — the platform layer reduces a real virtual key
/// code to one of these before it ever reaches `core::session`, and never forwards the character
/// a key would otherwise type (`docs/ARCHITECTURE.md` goal 2). Only Escape, Return and Space are
/// ever distinguished; every other key (including every modifier key, which instead arrives as
/// [`InputEvent::ModifierChange`]) collapses to [`KeyKind::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    /// The Escape key.
    Escape,
    /// The Return key — the platform layer maps the numeric keypad's Enter key to this too, since
    /// it counts as Return for the Esc+Return unlock combo (DESIGN.md §8).
    Return,
    /// The space bar.
    Space,
    /// Every other key.
    Other,
}

/// One input event, already translated from a platform key/pointer event into the shape the
/// session cares about.
///
/// ## Coordinate convention
///
/// Every pointer variant's `x`/`y` is in **main-screen logical points, origin at the top-left**
/// (the same convention `core::layout` and `core::canvas` already use). The platform layer is
/// responsible for converting both of its input sources into this single space before handing an
/// event to [`Session::handle_input`]: the input-blocker event tap's coordinates (global display
/// points via `CGEventGetLocation`, already top-left-origin on the main display) and winit's
/// `CursorMoved` (window-local logical coordinates, needing the window's screen origin added) —
/// see `src/app/`'s coordinate-conversion helper.
///
/// No character is ever carried by a key event, even for keys that would otherwise type a
/// letter: the hold-to-unlock button needs none, so none is decoded, kept, or sent anywhere
/// (`docs/ARCHITECTURE.md` goal 2, now trivially true rather than merely upheld).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputEvent {
    /// A key went down.
    KeyDown {
        /// Which key.
        kind: KeyKind,
        /// Whether this is an OS-generated autorepeat of an already-held key
        /// (`kCGKeyboardEventAutorepeat`), not a fresh press. Repeats still reshow the hint like
        /// any other key (unchanged from before this event carried the distinction) but are
        /// filtered out of the Esc+Return combo's start/reset decisions — see
        /// `combo_event_for` and `crate::core::combo`'s own module docs.
        repeat: bool,
    },
    /// A key was released.
    KeyUp(KeyKind),
    /// A modifier key's flags changed (`kCGEventFlagsChanged`: Shift, Control, Option, Command,
    /// Caps Lock, Fn, or similar). Carries no information about which modifier or whether it went
    /// down or up — the Esc+Return unlock combo is the only thing that cares, and it only needs
    /// to know that *something* happened (see `crate::core::combo`'s module docs).
    ModifierChange,
    /// A pointer button went down at `(x, y)`.
    PointerDown {
        /// See the type's coordinate convention.
        x: f32,
        /// See the type's coordinate convention.
        y: f32,
    },
    /// The pointer moved while a button was held, to `(x, y)`.
    PointerDragged {
        /// See the type's coordinate convention.
        x: f32,
        /// See the type's coordinate convention.
        y: f32,
    },
    /// A pointer button was released at `(x, y)`.
    PointerUp {
        /// See the type's coordinate convention.
        x: f32,
        /// See the type's coordinate convention.
        y: f32,
    },
    /// The pointer moved with no button held, to `(x, y)` (used for the hover reveal).
    PointerMoved {
        /// See the type's coordinate convention.
        x: f32,
        /// See the type's coordinate convention.
        y: f32,
    },
    /// A scroll-wheel/trackpad scroll event.
    Scroll,
}

/// The coarse phase of a session, for callers that only need a high-level state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// The 3-2-1 countdown is running; inputs are not blocked yet.
    Countdown,
    /// Inputs are blocked; waiting for the hold-to-unlock button or the fail-safe deadline.
    Locked,
    /// The hold completed (or fail-safe fired); the overlay is fading out.
    Unlocking,
    /// The session is over.
    Finished(EndReason),
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// The user held the unlock button for the full 2 s.
    Unlocked,
    /// The fail-safe deadline was reached before the button was held long enough.
    Failsafe,
    /// The user cancelled during the countdown (Escape or a pointer-down).
    Cancelled,
}

/// A side effect the caller (platform layer) must perform in response to a state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    /// Install (or confirm) the input-blocking event tap: the countdown just ended.
    BlockInputs,
    /// Remove the input-blocking event tap: unlocking has begun (hold completed or fail-safe
    /// fired). Emitted *when entering* [`SessionPhase::Unlocking`], before the fade, so inputs
    /// are freed before the overlay visually disappears.
    ReleaseInputs,
    /// The session is fully done: close the overlay window(s)/HUD.
    Close,
}

/// Static configuration for one cleaning session, fixed for its whole lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// Cleaning screen color.
    pub color: CleaningColor,
    /// Fail-safe auto-unlock delay.
    pub failsafe: FailsafeDelay,
    /// Whether this is a keyboard-only session (HUD instead of a full-screen overlay).
    pub keyboard_only: bool,
    /// Language for any text the platform layer renders from this session's data.
    pub language: Language,
}

/// Snapshot of the countdown/deadline for the fraction-remaining time bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarView {
    /// Fraction of the fail-safe window remaining, in `[0.0, 1.0]`.
    pub fraction: f32,
    /// Bar opacity: higher in the final emphasis window (DESIGN.md §8: 12% normally, 25% in the
    /// last 10 seconds).
    pub opacity: f32,
}

/// Everything the renderer needs to draw the hold-to-unlock ✕ button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnlockButtonView {
    /// Button center, in the same coordinate space as [`InputEvent`]'s pointer variants.
    pub center: Point,
    /// The button's visual radius.
    pub radius: f32,
    /// The button's (circular) hit-target radius.
    pub hit_radius: f32,
    /// Button opacity, `[0.0, 1.0]` (shares the hint's fade curve, except at least fully visible
    /// while a hold is in progress or completing).
    pub opacity: f32,
    /// Hold progress ring fraction, `[0.0, 1.0]` (0 outside of an active/draining hold).
    pub hold_progress: f32,
    /// Button scale multiplier (1.0 normally; scales up to 1.1 during the completion pulse).
    pub scale: f32,
}

/// Everything the renderer needs to draw one frame; contains no drawing logic itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionView {
    /// Screen background color (the cleaning color, or a dead-pixel test step's color).
    pub background: Rgb,
    /// Ink color for text/button/bar, chosen for legibility against `background`.
    pub ink: Rgb,
    /// Whether this is a keyboard-only session (HUD instead of full-screen overlay).
    pub keyboard_only: bool,
    /// Countdown digit view, only present during [`SessionPhase::Countdown`].
    pub countdown: Option<CountdownView>,
    /// Opacity of the hint text (and the second line grouped with it), `[0.0, 1.0]`.
    pub hint_opacity: f32,
    /// Opacity of the "urahafu" wordmark, `[0.0, 1.0]`.
    pub wordmark_opacity: f32,
    /// The hold-to-unlock button, if its geometry has been set via
    /// [`Session::set_unlock_target`]. Present during both [`SessionPhase::Countdown`] (shown
    /// pulsing, not holdable) and [`SessionPhase::Locked`].
    pub unlock_button: Option<UnlockButtonView>,
    /// Whether the countdown screen's hold-to-unlock explanation line ("To unlock, hold ✕ for
    /// 2 seconds") should be shown; only ever `true` during [`SessionPhase::Countdown`].
    pub countdown_unlock_hint: bool,
    /// Time-remaining bar, `None` during the countdown, the dead-pixel test, or once unlocking.
    pub bar: Option<BarView>,
    /// Dead-pixel test step indicator `(step, total)`, e.g. `(2, 5)`; only shown while the test
    /// is running and the hint is visible.
    pub pixel_step: Option<(u8, u8)>,
    /// Seconds remaining until fail-safe auto-unlock, rounded up.
    pub remaining_secs: u32,
    /// Overlay opacity, for the final unlock fade-out; `1.0` at every other time.
    pub overlay_opacity: f32,
}

/// Where the hold-to-unlock button currently is, as told to the session by the platform layer
/// (`Session::set_unlock_target`) — the session never computes this itself, it only reacts to
/// pointer events relative to it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct UnlockTarget {
    center: Point,
    radius: f32,
    hit_radius: f32,
}

/// Internal state, one variant per [`SessionPhase`] but carrying the data needed to compute a
/// [`SessionView`] and drive transitions.
#[derive(Debug, Clone)]
enum State {
    Countdown {
        start: Instant,
    },
    Locked {
        hint: HintFader,
        pixel_test: PixelTest,
        deadline: Deadline,
        /// Whether the wordmark should still ever be shown: true only until the first input
        /// event, matching DESIGN.md §8 ("disappears with [the initial hint]").
        wordmark_eligible: bool,
        /// The unlock button's own pointer-hold interaction state.
        hold: HoldRing,
        /// The Esc+Return keyboard combo's interaction state (DESIGN.md §8); it drives its
        /// own [`HoldRing`] independently of `hold`, and the two are combined (max) in
        /// [`Session::view_locked`] — either one completing unlocks the session.
        combo: ComboTracker,
        /// Whether the pointer is currently within [`HOVER_RADIUS`] of the unlock button.
        hover: bool,
        /// When `hover` last became `true`, for the hover fade-in's own timeline.
        hover_since: Option<Instant>,
    },
    Unlocking {
        start: Instant,
        reason: EndReason,
    },
    Finished {
        reason: EndReason,
    },
}

/// The cleaning session state machine. See the module docs for the calling convention.
#[derive(Debug, Clone)]
pub struct Session {
    config: SessionConfig,
    countdown: Countdown,
    state: State,
    /// The hold-to-unlock button's current geometry, set by the platform layer via
    /// [`Session::set_unlock_target`] and persisted across phase transitions (it does not reset
    /// when the countdown ends, since the button typically does not move at that instant).
    unlock_target: Option<UnlockTarget>,
}

impl Session {
    /// Starts a new session at `now`, in [`SessionPhase::Countdown`].
    #[must_use]
    pub fn new(config: SessionConfig, now: Instant) -> Self {
        Self {
            config,
            countdown: Countdown::new(),
            state: State::Countdown { start: now },
            unlock_target: None,
        }
    }

    /// The session's static configuration.
    #[must_use]
    pub fn config(&self) -> SessionConfig {
        self.config
    }

    /// Tells the session where the hold-to-unlock button currently is: `center` and `hit_radius`
    /// in the same coordinate space as [`InputEvent`]'s pointer variants, `radius` the button's
    /// visual radius (exposed on [`SessionView::unlock_button`] for the renderer, but otherwise
    /// not used by the hold logic itself, which only cares about `hit_radius`).
    ///
    /// The platform layer calls this whenever the overlay opens, or whenever its scale/size
    /// changes (`core::layout`'s `Layout::unlock_button`/`UNLOCK_BUTTON_HIT_RADIUS` for the
    /// full-screen mode, `core::layout::hud_unlock_button`/`HUD_UNLOCK_BUTTON_HIT_RADIUS` for the
    /// keyboard-only HUD).
    pub fn set_unlock_target(&mut self, center: Point, radius: f32, hit_radius: f32) {
        self.unlock_target = Some(UnlockTarget {
            center,
            radius,
            hit_radius,
        });
    }

    /// The fail-safe deadline for the current lock, once the session has entered
    /// [`SessionPhase::Locked`]; `None` before that (during [`SessionPhase::Countdown`]) or after
    /// (once [`SessionPhase::Unlocking`] or [`SessionPhase::Finished`]).
    ///
    /// This is the exact `Instant` [`Session::tick`] compares against to emit
    /// [`SessionCommand::ReleaseInputs`]. The platform layer must arm
    /// `platform::input_blocker::InputBlocker::start`'s watchdog with this same instant — calling
    /// it right after receiving [`SessionCommand::BlockInputs`] — so the two independent
    /// "give up and unlock" mechanisms (this state machine, and the input blocker's own
    /// watchdog thread) never disagree about when the fail-safe fires.
    #[must_use]
    pub fn failsafe_deadline(&self) -> Option<Instant> {
        match &self.state {
            State::Locked { deadline, .. } => Some(deadline.at()),
            State::Countdown { .. } | State::Unlocking { .. } | State::Finished { .. } => None,
        }
    }

    /// The current coarse phase.
    #[must_use]
    pub fn phase(&self) -> SessionPhase {
        match &self.state {
            State::Countdown { .. } => SessionPhase::Countdown,
            State::Locked { .. } => SessionPhase::Locked,
            State::Unlocking { .. } => SessionPhase::Unlocking,
            State::Finished { reason } => SessionPhase::Finished(*reason),
        }
    }

    /// Handles one input event at `now`, returning any [`SessionCommand`]s the caller must act
    /// on.
    pub fn handle_input(&mut self, event: InputEvent, now: Instant) -> Vec<SessionCommand> {
        match &mut self.state {
            State::Countdown { .. } => {
                if matches!(
                    event,
                    InputEvent::KeyDown {
                        kind: KeyKind::Escape,
                        ..
                    } | InputEvent::PointerDown { .. }
                ) {
                    self.state = State::Finished {
                        reason: EndReason::Cancelled,
                    };
                    return vec![SessionCommand::Close];
                }
                Vec::new()
            }
            State::Locked {
                hint,
                pixel_test,
                wordmark_eligible,
                hold,
                combo,
                hover,
                hover_since,
                ..
            } => {
                // Every keyboard event (including the ones handled specially below) feeds the
                // Esc+Return combo tracker first (DESIGN.md §8) — it must see Space and
                // every other key too, since they all count as "other key" for it.
                if let Some(combo_event) = Self::combo_event_for(event) {
                    combo.handle_event(combo_event, now);
                }

                // Space advances the dead-pixel test and, per DESIGN.md §8, does *not* re-show
                // the hint by itself. In keyboard-only mode there is no pixel test at all
                // (docs/ARCHITECTURE.md): Space there falls through to the generic reshow below.
                if matches!(
                    event,
                    InputEvent::KeyDown {
                        kind: KeyKind::Space,
                        ..
                    }
                ) && !self.config.keyboard_only
                {
                    pixel_test.advance();
                    return Vec::new();
                }

                match event {
                    InputEvent::PointerMoved { x, y } => {
                        if let Some(target) = self.unlock_target {
                            let now_hover =
                                distance(target.center, Point::new(x, y)) <= HOVER_RADIUS;
                            if now_hover && !*hover {
                                *hover = true;
                                *hover_since = Some(now);
                                *wordmark_eligible = false;
                            } else if !now_hover && *hover {
                                *hover = false;
                                *hover_since = None;
                                hint.hold_from(now);
                            }
                        }
                    }
                    InputEvent::PointerDown { x, y } => {
                        let inside = self
                            .unlock_target
                            .is_some_and(|t| distance(t.center, Point::new(x, y)) <= t.hit_radius);
                        if inside {
                            hold.start(now);
                        } else {
                            hint.on_input(now);
                            *wordmark_eligible = false;
                        }
                    }
                    InputEvent::PointerUp { .. } => {
                        hold.release(now);
                    }
                    InputEvent::PointerDragged { x, y } => {
                        if matches!(hold, HoldRing::Holding { .. }) {
                            let outside = self.unlock_target.is_none_or(|t| {
                                distance(t.center, Point::new(x, y)) > t.hit_radius
                            });
                            if outside {
                                hold.release(now);
                            }
                        }
                    }
                    InputEvent::KeyDown { .. }
                    | InputEvent::ModifierChange
                    | InputEvent::Scroll => {
                        hint.on_input(now);
                        *wordmark_eligible = false;
                    }
                    // Feeding the combo tracker (above) is the only thing a key-up does; it never
                    // reshows the hint on its own (only a fresh key-down/scroll/blocked click do).
                    InputEvent::KeyUp(_) => {}
                }
                Vec::new()
            }
            State::Unlocking { .. } | State::Finished { .. } => Vec::new(),
        }
    }

    /// Maps a general [`InputEvent`] to the narrower [`ComboEvent`] the Esc+Return combo tracker
    /// understands, or `None` for events the combo does not care about at all (pointer/scroll
    /// input, and an autorepeat `KeyDown` — see [`crate::core::combo`]'s module docs for why a
    /// repeat carries nothing new for it).
    fn combo_event_for(event: InputEvent) -> Option<ComboEvent> {
        match event {
            InputEvent::KeyDown {
                kind: KeyKind::Escape,
                repeat: false,
            } => Some(ComboEvent::Down(ComboKey::Escape)),
            InputEvent::KeyDown {
                kind: KeyKind::Return,
                repeat: false,
            } => Some(ComboEvent::Down(ComboKey::Return)),
            InputEvent::KeyDown {
                kind: KeyKind::Space | KeyKind::Other,
                repeat: false,
            } => Some(ComboEvent::Down(ComboKey::Other)),
            InputEvent::KeyUp(KeyKind::Escape) => Some(ComboEvent::Up(ComboKey::Escape)),
            InputEvent::KeyUp(KeyKind::Return) => Some(ComboEvent::Up(ComboKey::Return)),
            InputEvent::KeyUp(KeyKind::Space | KeyKind::Other) => {
                Some(ComboEvent::Up(ComboKey::Other))
            }
            InputEvent::ModifierChange => Some(ComboEvent::ModifierChange),
            // An autorepeat carries nothing new for the combo (the key was already counted as
            // held by its first, non-repeat down — see `crate::core::combo`'s module docs), and
            // pointer/scroll input is outside the combo's concern entirely (DESIGN.md §8: only
            // *keys* matter to it).
            InputEvent::KeyDown { repeat: true, .. }
            | InputEvent::PointerDown { .. }
            | InputEvent::PointerDragged { .. }
            | InputEvent::PointerUp { .. }
            | InputEvent::PointerMoved { .. }
            | InputEvent::Scroll => None,
        }
    }

    /// Advances timers at `now`, returning any [`SessionCommand`]s the caller must act on:
    /// countdown end → [`SessionCommand::BlockInputs`], hold completion or fail-safe deadline →
    /// [`SessionCommand::ReleaseInputs`], unlock fade end → [`SessionCommand::Close`].
    pub fn tick(&mut self, now: Instant) -> Vec<SessionCommand> {
        match &mut self.state {
            State::Countdown { start } => {
                let elapsed = now.saturating_duration_since(*start);
                if self.countdown.is_finished(elapsed) {
                    let deadline = self.config.failsafe.deadline_from(now);
                    self.state = State::Locked {
                        hint: HintFader::new(now),
                        pixel_test: PixelTest::new(),
                        deadline,
                        wordmark_eligible: true,
                        hold: HoldRing::Idle,
                        combo: ComboTracker::new(),
                        hover: false,
                        hover_since: None,
                    };
                    return vec![SessionCommand::BlockInputs];
                }
                Vec::new()
            }
            State::Locked {
                deadline,
                hold,
                combo,
                ..
            } => {
                // Either the pointer hold or the keyboard combo completing unlocks the session;
                // while either is mid-completion-pulse, the fail-safe deadline is deliberately
                // not checked this tick (mirroring the pointer-only behavior before the combo
                // existed), so the reason reported is never "Failsafe" for a hold that had
                // already finished.
                if hold.is_completing() || combo.is_completing() {
                    if hold.completion_elapsed(now) || combo.completion_elapsed(now) {
                        self.state = State::Unlocking {
                            start: now,
                            reason: EndReason::Unlocked,
                        };
                        return vec![SessionCommand::ReleaseInputs];
                    }
                    return Vec::new();
                }
                if deadline.is_expired(now) {
                    self.state = State::Unlocking {
                        start: now,
                        reason: EndReason::Failsafe,
                    };
                    return vec![SessionCommand::ReleaseInputs];
                }
                hold.advance(now);
                combo.advance(now);
                Vec::new()
            }
            State::Unlocking { start, reason } => {
                if now >= *start + UNLOCK_FADE {
                    self.state = State::Finished { reason: *reason };
                    return vec![SessionCommand::Close];
                }
                Vec::new()
            }
            State::Finished { .. } => Vec::new(),
        }
    }

    /// Computes everything the renderer needs to draw the current frame at `now`.
    #[must_use]
    pub fn view(&self, now: Instant) -> SessionView {
        match &self.state {
            State::Countdown { start } => self.view_countdown(*start, now),
            State::Locked {
                hint,
                pixel_test,
                deadline,
                wordmark_eligible,
                hold,
                combo,
                hover,
                hover_since,
            } => self.view_locked(
                hint,
                *pixel_test,
                *deadline,
                *wordmark_eligible,
                hold,
                combo,
                *hover,
                *hover_since,
                now,
            ),
            State::Unlocking { start, .. } => self.view_unlocking(*start, now),
            State::Finished { .. } => self.view_finished(),
        }
    }

    fn view_countdown(&self, start: Instant, now: Instant) -> SessionView {
        let elapsed = now.saturating_duration_since(start);
        let background = self.config.color.rgb();
        let unlock_button = self.unlock_target.map(|target| UnlockButtonView {
            center: target.center,
            radius: target.radius,
            hit_radius: target.hit_radius,
            opacity: countdown_pulse_opacity(elapsed),
            hold_progress: 0.0,
            scale: 1.0,
        });
        SessionView {
            background,
            ink: ink_for(background),
            keyboard_only: self.config.keyboard_only,
            countdown: self.countdown.view(elapsed),
            hint_opacity: 0.0,
            wordmark_opacity: 0.0,
            unlock_button,
            countdown_unlock_hint: true,
            bar: None,
            pixel_step: None,
            #[allow(
                clippy::cast_possible_truncation,
                reason = "fail-safe delay is 30/60/90 seconds"
            )]
            remaining_secs: self.config.failsafe.seconds() as u32,
            overlay_opacity: 1.0,
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "a private helper mirroring State::Locked's own fields; splitting it further would just move the same data through an extra struct"
    )]
    fn view_locked(
        &self,
        hint: &HintFader,
        pixel_test: PixelTest,
        deadline: Deadline,
        wordmark_eligible: bool,
        hold: &HoldRing,
        combo: &ComboTracker,
        hover: bool,
        hover_since: Option<Instant>,
        now: Instant,
    ) -> SessionView {
        let step = pixel_test.step();
        let background = step.color().unwrap_or_else(|| self.config.color.rgb());

        let hint_opacity = if hover {
            let since = hover_since.unwrap_or(now);
            hover_opacity(since, now).max(hint.opacity(now))
        } else {
            hint.opacity(now)
        };

        let wordmark_opacity = if wordmark_eligible && !step.is_running() {
            hint_opacity
        } else {
            0.0
        };
        let bar = if step.is_running() {
            None
        } else {
            Some(BarView {
                fraction: deadline.fraction_remaining(now, self.config.failsafe.duration()),
                opacity: if deadline.is_in_final_emphasis(now) {
                    0.25
                } else {
                    0.12
                },
            })
        };
        let pixel_step = if step.is_running() && hint_opacity > 0.0 {
            step.step_indicator()
        } else {
            None
        };

        // The unlock button shows identical visual feedback whichever gesture is driving it
        // (DESIGN.md §8: "the same progress ring ... fills"), and when both are active at
        // once its progress/scale is the max of the two (DESIGN.md §8: "progress = the max").
        let combo_ring = combo.ring();
        let active = matches!(hold, HoldRing::Holding { .. } | HoldRing::Completing { .. })
            || matches!(
                combo_ring,
                HoldRing::Holding { .. } | HoldRing::Completing { .. }
            );
        let button_opacity = if active { 1.0 } else { hint_opacity };
        let scale = hold.scale(now).max(combo_ring.scale(now));
        let hold_progress = hold.progress(now).max(combo_ring.progress(now));
        let unlock_button = self.unlock_target.map(|target| UnlockButtonView {
            center: target.center,
            radius: target.radius,
            hit_radius: target.hit_radius,
            opacity: button_opacity,
            hold_progress,
            scale,
        });

        SessionView {
            background,
            ink: ink_for(background),
            keyboard_only: self.config.keyboard_only,
            countdown: None,
            hint_opacity,
            wordmark_opacity,
            unlock_button,
            countdown_unlock_hint: false,
            bar,
            pixel_step,
            remaining_secs: deadline.remaining_secs_ceil(now),
            overlay_opacity: 1.0,
        }
    }

    fn view_unlocking(&self, start: Instant, now: Instant) -> SessionView {
        let background = self.config.color.rgb();
        let elapsed = now.saturating_duration_since(start);
        let t = (elapsed.as_secs_f32() / UNLOCK_FADE.as_secs_f32()).clamp(0.0, 1.0);
        let overlay_opacity = 1.0 - t;
        SessionView {
            background,
            ink: ink_for(background),
            keyboard_only: self.config.keyboard_only,
            countdown: None,
            hint_opacity: 0.0,
            wordmark_opacity: 0.0,
            unlock_button: None,
            countdown_unlock_hint: false,
            bar: None,
            pixel_step: None,
            remaining_secs: 0,
            overlay_opacity,
        }
    }

    fn view_finished(&self) -> SessionView {
        let background = self.config.color.rgb();
        SessionView {
            background,
            ink: ink_for(background),
            keyboard_only: self.config.keyboard_only,
            countdown: None,
            hint_opacity: 0.0,
            wordmark_opacity: 0.0,
            unlock_button: None,
            countdown_unlock_hint: false,
            bar: None,
            pixel_step: None,
            remaining_secs: 0,
            overlay_opacity: 0.0,
        }
    }

    /// The next instant the caller should wake up and call [`Session::tick`] (and typically
    /// redraw): roughly every animation frame while something is actively animating, otherwise
    /// the next timer boundary (fail-safe second tick, deadline, or unlock fade end). `None`
    /// once the session is [`SessionPhase::Finished`].
    #[must_use]
    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        match &self.state {
            State::Countdown { start } => {
                let elapsed = now.saturating_duration_since(*start);
                if self.countdown.is_finished(elapsed) {
                    Some(now)
                } else {
                    // The countdown-phase button pulse animates continuously too.
                    Some((now + ANIMATION_FRAME).min(*start + Countdown::TOTAL))
                }
            }
            State::Locked {
                hint,
                deadline,
                hold,
                combo,
                hover,
                hover_since,
                ..
            } => {
                let mut candidates = vec![deadline.at(), next_second_boundary(*deadline, now)];
                if let Some(hint_wake) = hint.next_wake(now) {
                    candidates.push(hint_wake);
                }
                let hover_animating = *hover
                    && hover_since
                        .is_some_and(|since| now.saturating_duration_since(since) < HOVER_FADE_IN);
                let animating = hold.is_active() || combo.ring().is_active() || hover_animating;
                if animating {
                    candidates.push((now + ANIMATION_FRAME).min(deadline.at()));
                }
                candidates.into_iter().min()
            }
            State::Unlocking { start, .. } => {
                let end = *start + UNLOCK_FADE;
                Some(if now >= end {
                    now
                } else {
                    (now + ANIMATION_FRAME).min(end)
                })
            }
            State::Finished { .. } => None,
        }
    }
}

/// Euclidean distance between two points in the same coordinate space.
fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Fraction of `duration` elapsed since `start`, as of `now`, clamped to `[0.0, 1.0]`.
fn fraction(start: Instant, duration: Duration, now: Instant) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    let elapsed = now.saturating_duration_since(start);
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

/// The hover-reveal fade-in opacity, `[0.0, 1.0]`, `HOVER_FADE_IN` after `since` (when hovering
/// started); stays at `1.0` for as long as the caller keeps calling this with `hover` still
/// `true`, since it never itself decides when hovering ends.
fn hover_opacity(since: Instant, now: Instant) -> f32 {
    fraction(since, HOVER_FADE_IN, now)
}

/// The countdown-phase button's gentle pulse opacity (60%..100%, `COUNTDOWN_PULSE_PERIOD`
/// period), starting at the minimum at `elapsed == 0`.
fn countdown_pulse_opacity(elapsed: Duration) -> f32 {
    let period_secs = COUNTDOWN_PULSE_PERIOD.as_secs_f32();
    let t = (elapsed.as_secs_f32() % period_secs) / period_secs;
    let mid = f32::midpoint(COUNTDOWN_PULSE_MIN_OPACITY, COUNTDOWN_PULSE_MAX_OPACITY);
    let amplitude = (COUNTDOWN_PULSE_MAX_OPACITY - COUNTDOWN_PULSE_MIN_OPACITY) / 2.0;
    mid - amplitude * (2.0 * std::f32::consts::PI * t).cos()
}

/// The next instant at which `deadline`'s rounded-up remaining-seconds count changes.
fn next_second_boundary(deadline: Deadline, now: Instant) -> Instant {
    let secs_ceil = u64::from(deadline.remaining_secs_ceil(now));
    if secs_ceil == 0 {
        return deadline.at();
    }
    deadline
        .at()
        .checked_sub(Duration::from_secs(secs_ceil - 1))
        .unwrap_or_else(|| deadline.at())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hold_ring::{COMPLETION_SCALE_PULSE, HOLD_DRAIN, HOLD_DURATION};

    /// A generous hit radius so tests can target the button without fussing over exact geometry.
    const TARGET_RADIUS: f32 = 22.0;
    const TARGET_HIT_RADIUS: f32 = 32.0;
    const TARGET_CENTER: Point = Point { x: 720.0, y: 800.0 };

    fn config() -> SessionConfig {
        SessionConfig {
            color: CleaningColor::Black,
            failsafe: FailsafeDelay::Seconds60,
            keyboard_only: false,
            language: Language::ENGLISH,
        }
    }

    fn locked_session(now: Instant) -> (Session, Instant) {
        let mut session = Session::new(config(), now);
        session.set_unlock_target(TARGET_CENTER, TARGET_RADIUS, TARGET_HIT_RADIUS);
        let locked_at = now + Countdown::TOTAL;
        session.tick(locked_at);
        (session, locked_at)
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

    /// A fresh (non-repeat) key-down event, for tests that don't care about autorepeat.
    fn key_down(kind: KeyKind) -> InputEvent {
        InputEvent::KeyDown {
            kind,
            repeat: false,
        }
    }

    #[test]
    fn starts_in_countdown() {
        let now = Instant::now();
        let session = Session::new(config(), now);
        assert_eq!(session.phase(), SessionPhase::Countdown);
        assert!(session.view(now).countdown.is_some());
        assert!(session.view(now).countdown_unlock_hint);
    }

    #[test]
    fn escape_during_countdown_cancels() {
        let now = Instant::now();
        let mut session = Session::new(config(), now);
        let commands = session.handle_input(key_down(KeyKind::Escape), now);
        assert_eq!(commands, vec![SessionCommand::Close]);
        assert_eq!(
            session.phase(),
            SessionPhase::Finished(EndReason::Cancelled)
        );
    }

    #[test]
    fn pointer_down_during_countdown_cancels() {
        let now = Instant::now();
        let mut session = Session::new(config(), now);
        let commands = session.handle_input(InputEvent::PointerDown { x: 0.0, y: 0.0 }, now);
        assert_eq!(commands, vec![SessionCommand::Close]);
        assert_eq!(
            session.phase(),
            SessionPhase::Finished(EndReason::Cancelled)
        );
    }

    #[test]
    fn other_input_during_countdown_is_ignored() {
        let now = Instant::now();
        let mut session = Session::new(config(), now);
        let commands = session.handle_input(key_down(KeyKind::Other), now);
        assert!(commands.is_empty());
        assert_eq!(session.phase(), SessionPhase::Countdown);
        let commands = session.handle_input(InputEvent::Scroll, now);
        assert!(commands.is_empty());
        let commands = session.handle_input(InputEvent::PointerMoved { x: 0.0, y: 0.0 }, now);
        assert!(commands.is_empty());
        assert_eq!(session.phase(), SessionPhase::Countdown);
    }

    #[test]
    fn hold_before_locked_is_impossible() {
        // A pointer-down at the (not-yet-relevant) target position during Countdown always
        // cancels the session outright; there is no way to start a hold before Locked.
        let now = Instant::now();
        let mut session = Session::new(config(), now);
        session.set_unlock_target(TARGET_CENTER, TARGET_RADIUS, TARGET_HIT_RADIUS);
        let commands = press(&mut session, now);
        assert_eq!(commands, vec![SessionCommand::Close]);
        assert_eq!(
            session.phase(),
            SessionPhase::Finished(EndReason::Cancelled)
        );
    }

    #[test]
    fn tick_transitions_countdown_to_locked_and_blocks_inputs() {
        let now = Instant::now();
        let mut session = Session::new(config(), now);
        let before_end = (now + Countdown::TOTAL)
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert!(session.tick(before_end).is_empty());
        assert_eq!(session.phase(), SessionPhase::Countdown);

        let at_end = now + Countdown::TOTAL;
        let commands = session.tick(at_end);
        assert_eq!(commands, vec![SessionCommand::BlockInputs]);
        assert_eq!(session.phase(), SessionPhase::Locked);
    }

    #[test]
    fn full_hold_completes_at_exactly_two_seconds() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        press(&mut session, locked_at);

        let just_before = (locked_at + HOLD_DURATION)
            .checked_sub(Duration::from_millis(1))
            .unwrap();
        assert!(session.tick(just_before).is_empty());
        assert_eq!(session.phase(), SessionPhase::Locked);
        assert!(
            session
                .view(just_before)
                .unlock_button
                .unwrap()
                .hold_progress
                < 1.0
        );

        let at_2s = locked_at + HOLD_DURATION;
        assert!(
            session.tick(at_2s).is_empty(),
            "completion pulse starts here, not the transition"
        );
        assert_eq!(session.phase(), SessionPhase::Locked);
        assert!((session.view(at_2s).unlock_button.unwrap().hold_progress - 1.0).abs() < 1e-6);

        let after_pulse = at_2s + COMPLETION_SCALE_PULSE;
        let commands = session.tick(after_pulse);
        assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
        assert_eq!(session.phase(), SessionPhase::Unlocking);

        let close_at = after_pulse + UNLOCK_FADE;
        let commands = session.tick(close_at);
        assert_eq!(commands, vec![SessionCommand::Close]);
        assert_eq!(session.phase(), SessionPhase::Finished(EndReason::Unlocked));
    }

    #[test]
    fn early_release_cancels_and_drains() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        press(&mut session, locked_at);

        let mid_hold = locked_at + HOLD_DURATION / 2;
        let progress_at_release = session.view(mid_hold).unlock_button.unwrap().hold_progress;
        assert!(progress_at_release > 0.0 && progress_at_release < 1.0);

        release(&mut session, mid_hold);
        assert_eq!(session.phase(), SessionPhase::Locked);
        assert!(
            (session.view(mid_hold).unlock_button.unwrap().hold_progress - progress_at_release)
                .abs()
                < 1e-3
        );

        let mid_drain = mid_hold + HOLD_DRAIN / 2;
        let draining_progress = session.view(mid_drain).unlock_button.unwrap().hold_progress;
        assert!(draining_progress > 0.0 && draining_progress < progress_at_release);

        let after_drain = mid_hold + HOLD_DRAIN;
        session.tick(after_drain);
        assert!(
            (session
                .view(after_drain)
                .unlock_button
                .unwrap()
                .hold_progress)
                .abs()
                < 1e-6
        );
        assert_eq!(session.phase(), SessionPhase::Locked);

        // Far past 2s of wall-clock time since the original press: still locked, never unlocked.
        let far_later = locked_at + HOLD_DURATION * 3;
        assert_ne!(session.phase(), SessionPhase::Unlocking);
        let _ = far_later;
    }

    #[test]
    fn leaving_target_cancels_the_hold() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        press(&mut session, locked_at);

        let mid_hold = locked_at + HOLD_DURATION / 2;
        // Drag far outside the hit radius.
        session.handle_input(
            InputEvent::PointerDragged {
                x: TARGET_CENTER.x + TARGET_HIT_RADIUS * 5.0,
                y: TARGET_CENTER.y,
            },
            mid_hold,
        );
        let progress = session.view(mid_hold).unlock_button.unwrap().hold_progress;
        assert!(
            progress > 0.0,
            "drain starts from the progress at the moment of leaving"
        );

        let after_drain = mid_hold + HOLD_DRAIN;
        session.tick(after_drain);
        assert!(
            (session
                .view(after_drain)
                .unlock_button
                .unwrap()
                .hold_progress)
                .abs()
                < 1e-6
        );

        let far_later = locked_at + HOLD_DURATION * 3;
        session.tick(far_later);
        assert_eq!(session.phase(), SessionPhase::Locked);
    }

    #[test]
    fn dragging_while_staying_inside_the_target_does_not_cancel() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        press(&mut session, locked_at);

        let mid_hold = locked_at + HOLD_DURATION / 2;
        session.handle_input(
            InputEvent::PointerDragged {
                x: TARGET_CENTER.x + 5.0,
                y: TARGET_CENTER.y,
            },
            mid_hold,
        );
        let progress = session.view(mid_hold).unlock_button.unwrap().hold_progress;
        assert!(
            progress > 0.4,
            "still holding: progress keeps advancing, not draining"
        );
    }

    #[test]
    fn drag_in_from_outside_does_not_start_a_hold() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        // No PointerDown ever happened: a drag alone must never start a hold.
        session.handle_input(
            InputEvent::PointerDragged {
                x: TARGET_CENTER.x,
                y: TARGET_CENTER.y,
            },
            locked_at,
        );
        let later = locked_at + HOLD_DURATION / 2;
        assert!((session.view(later).unlock_button.unwrap().hold_progress).abs() < 1e-6);
        assert_eq!(session.phase(), SessionPhase::Locked);
    }

    #[test]
    fn accidental_short_taps_never_unlock() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        for i in 0..20 {
            let t = locked_at + Duration::from_millis(i * 50);
            press(&mut session, t);
            release(&mut session, t + Duration::from_millis(10));
        }
        let later = locked_at + Duration::from_secs(2);
        session.tick(later);
        assert_eq!(session.phase(), SessionPhase::Locked);
    }

    #[test]
    fn hover_reveal_radius_boundaries() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        let hidden_at = locked_at + Duration::from_secs(10);
        assert!((session.view(hidden_at).hint_opacity).abs() < 1e-3);

        // Just inside the hover radius.
        session.handle_input(
            InputEvent::PointerMoved {
                x: TARGET_CENTER.x + HOVER_RADIUS - 1.0,
                y: TARGET_CENTER.y,
            },
            hidden_at,
        );
        let fully_faded_in = hidden_at + HOVER_FADE_IN;
        assert!((session.view(fully_faded_in).hint_opacity - 1.0).abs() < 1e-3);

        // Stays visible arbitrarily long while still hovering.
        let much_later = fully_faded_in + Duration::from_secs(10);
        assert!((session.view(much_later).hint_opacity - 1.0).abs() < 1e-3);

        // Move outside the hover radius: starts the normal reshow hold+fade timeline from there.
        session.handle_input(
            InputEvent::PointerMoved {
                x: TARGET_CENTER.x + HOVER_RADIUS + 50.0,
                y: TARGET_CENTER.y,
            },
            much_later,
        );
        assert!((session.view(much_later).hint_opacity - 1.0).abs() < 1e-3);
        let long_after_leaving = much_later + Duration::from_secs(10);
        assert!((session.view(long_after_leaving).hint_opacity).abs() < 1e-3);
    }

    #[test]
    fn failsafe_still_wins_even_mid_hold() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        press(&mut session, locked_at);
        let mid_hold = locked_at + Duration::from_millis(500);
        let commands = session.tick(mid_hold);
        assert!(commands.is_empty());

        let deadline = locked_at + Duration::from_secs(60);
        let commands = session.tick(deadline);
        assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
        assert_eq!(session.phase(), SessionPhase::Unlocking);
    }

    #[test]
    fn failsafe_deadline_triggers_unlocking_with_release_inputs() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        let before_deadline = locked_at + Duration::from_secs(59);
        assert!(session.tick(before_deadline).is_empty());

        let at_deadline = locked_at + Duration::from_secs(60);
        let commands = session.tick(at_deadline);
        assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
        assert_eq!(session.phase(), SessionPhase::Unlocking);
    }

    #[test]
    fn any_input_reshows_hint_while_locked() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        let hidden_at = locked_at + Duration::from_secs(10);
        assert!((session.view(hidden_at).hint_opacity).abs() < 1e-3);

        session.handle_input(key_down(KeyKind::Other), hidden_at);
        // Right at the input instant the re-show fade-in has just started (opacity 0); it ramps
        // up from there.
        assert!((session.view(hidden_at).hint_opacity).abs() < 1e-3);
        let mid_fade_in = hidden_at + Duration::from_millis(100);
        assert!(session.view(mid_fade_in).hint_opacity > 0.0);
    }

    #[test]
    fn pointer_down_outside_target_reshows_hint_but_does_not_start_a_hold() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        let hidden_at = locked_at + Duration::from_secs(10);
        session.handle_input(
            InputEvent::PointerDown {
                x: TARGET_CENTER.x + 500.0,
                y: TARGET_CENTER.y,
            },
            hidden_at,
        );
        let mid_fade_in = hidden_at + Duration::from_millis(100);
        assert!(session.view(mid_fade_in).hint_opacity > 0.0);
        assert!(
            (session
                .view(mid_fade_in)
                .unlock_button
                .unwrap()
                .hold_progress)
                .abs()
                < 1e-6
        );
        assert_eq!(session.phase(), SessionPhase::Locked);
    }

    #[test]
    fn scroll_reshows_hint_while_locked() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        let hidden_at = locked_at + Duration::from_secs(10);
        session.handle_input(InputEvent::Scroll, hidden_at);
        let mid_fade_in = hidden_at + Duration::from_millis(100);
        assert!(session.view(mid_fade_in).hint_opacity > 0.0);
    }

    #[test]
    fn space_advances_pixel_test_without_reshowing_hint() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        let hidden_at = locked_at + Duration::from_secs(10);
        assert!((session.view(hidden_at).hint_opacity).abs() < 1e-3);

        session.handle_input(key_down(KeyKind::Space), hidden_at);
        assert!(
            (session.view(hidden_at).hint_opacity).abs() < 1e-3,
            "space alone must not reshow the hint"
        );
        assert_eq!(session.view(hidden_at).background, Rgb::new(255, 0, 0));
    }

    #[test]
    fn space_in_keyboard_only_mode_does_not_run_pixel_test() {
        let now = Instant::now();
        let mut session = Session::new(
            SessionConfig {
                keyboard_only: true,
                ..config()
            },
            now,
        );
        let locked_at = now + Countdown::TOTAL;
        session.tick(locked_at);
        let hidden_at = locked_at + Duration::from_secs(10);
        assert!((session.view(hidden_at).hint_opacity).abs() < 1e-3);

        session.handle_input(key_down(KeyKind::Space), hidden_at);
        assert_eq!(
            session.view(hidden_at).background,
            Rgb::BLACK,
            "no pixel test in keyboard-only mode"
        );
        let mid_fade_in = hidden_at + Duration::from_millis(100);
        assert!(
            session.view(mid_fade_in).hint_opacity > 0.0,
            "space is just another key there, and reshows the hint"
        );
    }

    #[test]
    fn pixel_test_hides_bar_and_wordmark() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Space), locked_at);
        let view = session.view(locked_at);
        assert!(view.bar.is_none());
        assert!((view.wordmark_opacity).abs() < 1e-6);
    }

    #[test]
    fn pixel_step_only_shown_when_hint_visible() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        let hidden_at = locked_at + Duration::from_secs(10);
        assert!((session.view(hidden_at).hint_opacity).abs() < 1e-3);

        session.handle_input(key_down(KeyKind::Space), hidden_at);
        // Hint not visible (space doesn't reshow it): no step indicator yet.
        assert_eq!(session.view(hidden_at).pixel_step, None);

        // A real key reshows the hint while the test keeps running.
        session.handle_input(key_down(KeyKind::Other), hidden_at);
        let mid_fade_in = hidden_at + Duration::from_millis(100);
        assert_eq!(session.view(mid_fade_in).pixel_step, Some((1, 5)));
    }

    #[test]
    fn wordmark_disappears_after_first_input_even_if_hint_reshows() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        assert!(session.view(locked_at).wordmark_opacity > 0.0);

        session.handle_input(key_down(KeyKind::Other), locked_at);
        let later = locked_at + Duration::from_millis(500);
        assert!((session.view(later).wordmark_opacity).abs() < 1e-6);
        assert!(
            session.view(later).hint_opacity > 0.0,
            "hint itself does reshow"
        );
    }

    #[test]
    fn bar_opacity_increases_in_final_ten_seconds() {
        let now = Instant::now();
        let (session, locked_at) = locked_session(now);
        let mid = locked_at + Duration::from_secs(30);
        let bar = session.view(mid).bar.expect("bar present");
        assert!((bar.opacity - 0.12).abs() < 1e-6);

        let near_end = locked_at + Duration::from_secs(55);
        let bar = session.view(near_end).bar.expect("bar present");
        assert!((bar.opacity - 0.25).abs() < 1e-6);
    }

    #[test]
    fn failsafe_deadline_is_only_some_while_locked() {
        let now = Instant::now();
        let session = Session::new(config(), now);
        assert_eq!(session.failsafe_deadline(), None);

        let (mut session, locked_at) = locked_session(now);
        let deadline = session
            .failsafe_deadline()
            .expect("deadline present while locked");
        assert_eq!(deadline, locked_at + FailsafeDelay::Seconds60.duration());

        press(&mut session, locked_at);
        session.tick(locked_at + HOLD_DURATION);
        session.tick(locked_at + HOLD_DURATION + COMPLETION_SCALE_PULSE);
        assert_eq!(
            session.failsafe_deadline(),
            None,
            "no deadline once unlocking"
        );
    }

    #[test]
    fn next_wake_is_none_once_finished() {
        let now = Instant::now();
        let mut session = Session::new(config(), now);
        session.handle_input(key_down(KeyKind::Escape), now);
        assert!(session.next_wake(now).is_none());
    }

    #[test]
    fn next_wake_during_countdown_is_animation_paced() {
        let now = Instant::now();
        let session = Session::new(config(), now);
        let wake = session.next_wake(now).expect("should wake soon");
        assert!(wake <= now + Duration::from_millis(16));
    }

    #[test]
    fn next_wake_while_locked_is_bounded_by_deadline() {
        let now = Instant::now();
        let (session, locked_at) = locked_session(now);
        let far_future = locked_at + Duration::from_secs(1000);
        let wake = session
            .next_wake(locked_at)
            .expect("should wake before the deadline");
        assert!(wake <= locked_at + FailsafeDelay::Seconds60.duration());
        let _ = far_future;
    }

    #[test]
    fn keyboard_only_flag_is_passed_through_to_view() {
        let now = Instant::now();
        let session = Session::new(
            SessionConfig {
                keyboard_only: true,
                ..config()
            },
            now,
        );
        assert!(session.view(now).keyboard_only);
    }

    #[test]
    fn repeated_hold_after_unlocking_is_a_no_op() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        press(&mut session, locked_at);
        session.tick(locked_at + HOLD_DURATION);
        session.tick(locked_at + HOLD_DURATION + COMPLETION_SCALE_PULSE);
        assert_eq!(session.phase(), SessionPhase::Unlocking);
        let commands = press(
            &mut session,
            locked_at + HOLD_DURATION + COMPLETION_SCALE_PULSE,
        );
        assert!(commands.is_empty());
        assert_eq!(session.phase(), SessionPhase::Unlocking);
    }

    #[test]
    fn pixel_test_unaffected_by_hold_state() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        for _ in 0..5 {
            session.handle_input(key_down(KeyKind::Space), locked_at);
        }
        assert_eq!(session.view(locked_at).background, Rgb::new(0, 0, 0));
        press(&mut session, locked_at);
        assert_eq!(session.view(locked_at).background, Rgb::new(0, 0, 0));
    }

    // -- Esc+Return keyboard combo (DESIGN.md §8) -------------------------------------------

    fn key_up(session: &mut Session, kind: KeyKind, now: Instant) -> Vec<SessionCommand> {
        session.handle_input(InputEvent::KeyUp(kind), now)
    }

    #[test]
    fn combo_hold_unlocks_like_the_button() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        session.handle_input(key_down(KeyKind::Return), locked_at);

        let at_2s = locked_at + HOLD_DURATION;
        assert!(session.tick(at_2s).is_empty(), "completion pulse first");
        assert_eq!(session.phase(), SessionPhase::Locked);

        let after_pulse = at_2s + COMPLETION_SCALE_PULSE;
        let commands = session.tick(after_pulse);
        assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
        assert_eq!(session.phase(), SessionPhase::Unlocking);
    }

    #[test]
    fn combo_start_order_of_the_two_keys_does_not_matter() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Return), locked_at);
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        let mid = locked_at + HOLD_DURATION / 2;
        assert!(session.view(mid).unlock_button.unwrap().hold_progress > 0.0);
    }

    #[test]
    fn combo_release_before_completion_never_unlocks() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        session.handle_input(key_down(KeyKind::Return), locked_at);

        let mid_hold = locked_at + HOLD_DURATION / 2;
        key_up(&mut session, KeyKind::Escape, mid_hold);

        let past_original_duration = locked_at + HOLD_DURATION + Duration::from_millis(1);
        session.tick(past_original_duration);
        assert_eq!(session.phase(), SessionPhase::Locked);
    }

    #[test]
    fn combo_extra_key_down_resets_progress() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        session.handle_input(key_down(KeyKind::Return), locked_at);
        let mid = locked_at + HOLD_DURATION / 2;
        assert!(session.view(mid).unlock_button.unwrap().hold_progress > 0.0);

        // A third key going down (including Space, per DESIGN.md §8) resets it.
        session.handle_input(key_down(KeyKind::Space), mid);
        session.tick(mid + Duration::from_millis(250));
        assert!(
            session
                .view(mid + Duration::from_millis(250))
                .unlock_button
                .unwrap()
                .hold_progress
                .abs()
                < 1e-6
        );
        assert_eq!(session.phase(), SessionPhase::Locked);
    }

    #[test]
    fn combo_extra_key_release_lets_the_hold_restart() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        session.handle_input(key_down(KeyKind::Return), locked_at);
        let mid = locked_at + HOLD_DURATION / 2;
        session.handle_input(key_down(KeyKind::Other), mid);
        let drained = mid + Duration::from_millis(250);
        session.tick(drained);

        key_up(&mut session, KeyKind::Other, drained);
        let later = drained + HOLD_DURATION / 2;
        assert!(session.view(later).unlock_button.unwrap().hold_progress > 0.0);
    }

    #[test]
    fn combo_modifier_change_resets_and_blocks_restart_until_fresh_press() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        session.handle_input(key_down(KeyKind::Return), locked_at);
        let mid = locked_at + HOLD_DURATION / 2;
        assert!(session.view(mid).unlock_button.unwrap().hold_progress > 0.0);

        session.handle_input(InputEvent::ModifierChange, mid);
        let much_later = mid + HOLD_DURATION * 3;
        session.tick(much_later);
        assert!(
            session
                .view(much_later)
                .unlock_button
                .unwrap()
                .hold_progress
                .abs()
                < 1e-6,
            "Esc and Return never went up, but a modifier pulse must still block the combo from \
             silently resuming"
        );
        assert_eq!(session.phase(), SessionPhase::Locked);

        // Releasing and re-pressing Return clears the block.
        key_up(&mut session, KeyKind::Return, much_later);
        let repress_at = much_later + Duration::from_millis(1);
        session.handle_input(key_down(KeyKind::Return), repress_at);
        let after_repress = repress_at + HOLD_DURATION / 2;
        assert!(
            session
                .view(after_repress)
                .unlock_button
                .unwrap()
                .hold_progress
                > 0.0
        );
    }

    #[test]
    fn combo_autorepeat_of_the_second_key_is_ignored() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        // An autorepeat of Return arrives before any fresh (non-repeat) Return down — this must
        // not be mistaken for the key actually being held.
        session.handle_input(
            InputEvent::KeyDown {
                kind: KeyKind::Return,
                repeat: true,
            },
            locked_at,
        );
        let later = locked_at + Duration::from_millis(500);
        assert!(
            session
                .view(later)
                .unlock_button
                .unwrap()
                .hold_progress
                .abs()
                < 1e-6,
            "a repeat must never count as the key becoming held"
        );

        // A real (non-repeat) Return press does start the hold, and further repeats of it don't
        // disturb progress already in flight.
        session.handle_input(key_down(KeyKind::Return), later);
        let mid = later + HOLD_DURATION / 2;
        session.handle_input(
            InputEvent::KeyDown {
                kind: KeyKind::Return,
                repeat: true,
            },
            mid,
        );
        assert!(session.view(mid).unlock_button.unwrap().hold_progress > 0.0);
    }

    #[test]
    fn combo_failsafe_still_fires_independent_of_a_partial_hold() {
        let now = Instant::now();
        let mut session = Session::new(config(), now);
        session.set_unlock_target(TARGET_CENTER, TARGET_RADIUS, TARGET_HIT_RADIUS);
        let locked_at = now + Countdown::TOTAL;
        session.tick(locked_at);

        session.handle_input(key_down(KeyKind::Escape), locked_at);
        session.handle_input(key_down(KeyKind::Return), locked_at);
        let mid_hold = locked_at + Duration::from_millis(500);
        assert!(session.tick(mid_hold).is_empty());

        let deadline = locked_at + FailsafeDelay::Seconds60.duration();
        let commands = session.tick(deadline);
        assert_eq!(commands, vec![SessionCommand::ReleaseInputs]);
        assert_eq!(session.phase(), SessionPhase::Unlocking);
    }

    #[test]
    fn combo_and_pointer_hold_progress_combine_as_the_max() {
        let now = Instant::now();
        let (mut session, locked_at) = locked_session(now);

        // Pointer hold alone, to exactly 25%.
        press(&mut session, locked_at);
        let quarter_point = locked_at + HOLD_DURATION / 4;
        let pointer_only_progress = session
            .view(quarter_point)
            .unlock_button
            .unwrap()
            .hold_progress;
        assert!((pointer_only_progress - 0.25).abs() < 1e-2);

        // The combo starts at the same instant the pointer hold began, so at `quarter_point` it
        // is *also* at 25% — engaging it must not, on its own, change the reported progress yet.
        session.handle_input(key_down(KeyKind::Escape), locked_at);
        session.handle_input(key_down(KeyKind::Return), locked_at);
        let combined_at_quarter = session
            .view(quarter_point)
            .unlock_button
            .unwrap()
            .hold_progress;
        assert!((combined_at_quarter - 0.25).abs() < 1e-2);

        // Release the pointer hold (it starts draining) while the combo keeps going: the
        // reported progress must track the combo's own, higher progress, not fall with the
        // draining pointer ring.
        release(&mut session, quarter_point);
        let three_quarter_point = locked_at + (HOLD_DURATION * 3) / 4;
        let progress_after_release = session
            .view(three_quarter_point)
            .unlock_button
            .unwrap()
            .hold_progress;
        assert!(
            (progress_after_release - 0.75).abs() < 1e-2,
            "expected the still-active combo's ~75% progress to win the max over the draining \
             pointer ring, got {progress_after_release}"
        );
    }
}
