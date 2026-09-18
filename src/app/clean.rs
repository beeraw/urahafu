//! The "Clean" flow (DESIGN.md §7-§9, §11): preflight checks, opening the
//! [`crate::core::session::Session`]/[`Overlay`], and driving
//! [`crate::core::session::SessionCommand`]s as the session advances.

use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use winit::event_loop::{ActiveEventLoop, ControlFlow};

use super::{App, CleaningSession, LaunchMode, RunState, UserEvent, main_thread_marker};
use crate::core::layout::{self, Layout};
use crate::core::session::{Session, SessionCommand, SessionConfig};
use crate::platform::alert::{self, AlertSpec};
use crate::platform::ffi::secure_input;
use crate::platform::input_blocker::{BlockError, InputBlocker};
use crate::platform::overlay::{Overlay, OverlayMode};
use crate::platform::permission;
use crate::platform::render;
use crate::platform::text::TextRasterizer;

/// The Safety-net preflight (DESIGN.md §11): the same reasons an event tap can fail to start,
/// checked up front so a doomed cleaning session never gets as far as showing the countdown.
/// Reuses [`BlockError`] rather than inventing a parallel enum, since [`InputBlocker::start`]
/// checks the exact same two conditions itself before ever attempting to create the tap.
fn preflight() -> Result<(), BlockError> {
    if !permission::is_trusted() {
        return Err(BlockError::PermissionDenied);
    }
    if secure_input::is_secure_input_enabled() {
        return Err(BlockError::SecureInputActive);
    }
    Ok(())
}

impl App {
    /// Starts a cleaning attempt: preflight, then (on success) opens the session/overlay. Called
    /// both for the tray's "Clean" command and to retry after a "Try Again" alert.
    pub(super) fn try_start_clean(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(err) = preflight() {
            self.handle_block_error(event_loop, err);
            return;
        }
        self.open_session_and_overlay(event_loop);
    }

    fn open_session_and_overlay(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let config = SessionConfig {
            color: self.settings.color,
            failsafe: self.settings.failsafe,
            keyboard_only: self.settings.keyboard_only,
            language: self.language,
        };
        let session = Session::new(config, now);
        let mode = if self.settings.keyboard_only {
            OverlayMode::KeyboardOnly
        } else {
            OverlayMode::FullScreen
        };

        match Overlay::open(event_loop, mode) {
            Ok(overlay) => {
                self.state = RunState::Cleaning(Box::new(CleaningSession {
                    session,
                    overlay,
                    blocker: None,
                    rasterizer: TextRasterizer::new(),
                }));
                self.update_unlock_target(now);
                self.tick_cleaning(event_loop);
            }
            Err(err) => {
                eprintln!("urahafu: failed to open the cleaning overlay: {err}");
                self.finish_clean_attempt(event_loop);
            }
        }
    }

    /// Tells the session where the hold-to-unlock button currently is (`Session::set_unlock_target`):
    /// from `core::layout::Layout::unlock_button` in full-screen mode, or from the HUD pill's own
    /// (content-dependent) layout in keyboard-only mode. Called right after opening the
    /// overlay and on every tick, so the button stays reachable even as the HUD pill's width
    /// changes with its (language-dependent) content.
    fn update_unlock_target(&mut self, now: std::time::Instant) {
        let RunState::Cleaning(cleaning) = &mut self.state else {
            return;
        };
        let screen = cleaning.overlay.main_screen_size();
        let session_layout = Layout::new(screen);
        match cleaning.overlay.mode() {
            OverlayMode::FullScreen => {
                cleaning.session.set_unlock_target(
                    session_layout.unlock_button(),
                    layout::UNLOCK_BUTTON_RADIUS,
                    layout::UNLOCK_BUTTON_HIT_RADIUS,
                );
            }
            OverlayMode::KeyboardOnly => {
                let view = cleaning.session.view(now);
                let content_width = render::hud_content_width_points(
                    &mut cleaning.rasterizer,
                    &view,
                    self.language,
                );
                let pill = session_layout.hud_pill(content_width.max(160.0));
                cleaning.session.set_unlock_target(
                    layout::hud_unlock_button(pill, self.language.is_rtl()),
                    layout::HUD_UNLOCK_BUTTON_RADIUS,
                    layout::HUD_UNLOCK_BUTTON_HIT_RADIUS,
                );
            }
        }
    }

    /// Advances the session's timers, applies any resulting [`SessionCommand`]s, then (if still
    /// cleaning) requests a redraw and reschedules the next wake-up. Called both right after
    /// opening a session and from `about_to_wait` on every scheduled wake.
    pub(super) fn tick_cleaning(&mut self, event_loop: &ActiveEventLoop) {
        let RunState::Cleaning(cleaning) = &mut self.state else {
            return;
        };
        let now = Instant::now();
        let commands = cleaning.session.tick(now);
        self.drive_session(event_loop, commands);

        self.update_unlock_target(now);
        let RunState::Cleaning(cleaning) = &mut self.state else {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        };
        cleaning.overlay.request_redraw();
        match cleaning.session.next_wake(now) {
            Some(wake) => event_loop.set_control_flow(ControlFlow::WaitUntil(wake)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    /// Applies every [`SessionCommand`] produced by a `tick`/`handle_input` call, in order.
    pub(super) fn drive_session(
        &mut self,
        event_loop: &ActiveEventLoop,
        commands: Vec<SessionCommand>,
    ) {
        for command in commands {
            match command {
                SessionCommand::BlockInputs => self.handle_block_inputs(event_loop),
                SessionCommand::ReleaseInputs => self.handle_release_inputs(),
                SessionCommand::Close => self.handle_close(event_loop),
            }
        }
    }

    /// The countdown just ended: installs the input blocker, arming its watchdog with the exact
    /// same deadline [`crate::core::session::Session`] uses for its own fail-safe
    /// (`Session::failsafe_deadline`) so the two independent "give up and unlock" mechanisms
    /// never disagree.
    fn handle_block_inputs(&mut self, event_loop: &ActiveEventLoop) {
        let RunState::Cleaning(cleaning) = &mut self.state else {
            return;
        };
        let Some(deadline) = cleaning.session.failsafe_deadline() else {
            return;
        };

        let (tx, rx) = mpsc::channel();
        let proxy = self.proxy.clone();
        let spawned = thread::Builder::new()
            .name("urahafu-input-forwarder".to_owned())
            .spawn(move || {
                // Ends on its own once the sender side closes, i.e. as soon as `BlockerGuard`
                // (owned below) is dropped and its tap thread joins.
                while let Ok(event) = rx.recv() {
                    if proxy.send_event(UserEvent::Input(event)).is_err() {
                        break;
                    }
                }
            });
        if let Err(err) = spawned {
            eprintln!("urahafu: failed to start the input-forwarding thread: {err}");
        }

        match InputBlocker::start(deadline, tx) {
            Ok(guard) => cleaning.blocker = Some(guard),
            Err(err) => {
                cleaning.overlay.close();
                self.state = RunState::Idle;
                self.handle_block_error(event_loop, err);
            }
        }
    }

    /// Unlocking has begun: drop the blocker guard immediately so inputs are freed before the
    /// overlay's fade-out even starts (DESIGN.md §8: "the event tap is removed **before** the
    /// fade-out").
    fn handle_release_inputs(&mut self) {
        if let RunState::Cleaning(cleaning) = &mut self.state {
            cleaning.blocker = None;
        }
    }

    /// The session is fully done (unlocked, fail-safe fired, or cancelled during the countdown):
    /// close the overlay and go back to idle; in direct mode, exit the app entirely.
    fn handle_close(&mut self, event_loop: &ActiveEventLoop) {
        if let RunState::Cleaning(cleaning) = &mut self.state {
            cleaning.blocker = None;
            cleaning.overlay.close();
        }
        self.state = RunState::Idle;
        if self.mode == LaunchMode::Direct {
            event_loop.exit();
        }
    }

    /// Shows the alert matching `err` (DESIGN.md §11) and either retries the whole clean attempt
    /// ("Try Again", for the two retryable failures) or ends it.
    fn handle_block_error(&mut self, event_loop: &ActiveEventLoop, err: BlockError) {
        let mtm = main_thread_marker();
        match err {
            BlockError::PermissionDenied => {
                let choice = alert::show(&AlertSpec::permission_required(self.language), mtm);
                if choice == 0 {
                    if let Err(open_err) = permission::open_accessibility_settings() {
                        eprintln!("urahafu: failed to open System Settings: {open_err}");
                    }
                }
                self.finish_clean_attempt(event_loop);
            }
            BlockError::SecureInputActive => {
                let choice = alert::show(&AlertSpec::secure_input(self.language), mtm);
                if choice == 0 {
                    self.try_start_clean(event_loop);
                } else {
                    self.finish_clean_attempt(event_loop);
                }
            }
            BlockError::TapCreationFailed => {
                let choice = alert::show(&AlertSpec::tap_failed(self.language), mtm);
                if choice == 0 {
                    self.try_start_clean(event_loop);
                } else {
                    self.finish_clean_attempt(event_loop);
                }
            }
        }
    }

    /// Ends a failed clean attempt (no overlay was ever shown, or it was already closed by the
    /// caller): back to idle in menu-bar mode, or exit in direct mode (DESIGN.md §11: "In
    /// direct-launch mode, once the error alert is dismissed, the app quits").
    fn finish_clean_attempt(&mut self, event_loop: &ActiveEventLoop) {
        self.state = RunState::Idle;
        if self.mode == LaunchMode::Direct {
            event_loop.exit();
        }
    }
}
