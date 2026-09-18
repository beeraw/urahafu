//! The [`winit::application::ApplicationHandler`] impl: dispatches winit events to the methods in
//! `startup.rs`, `clean.rs`, `menu.rs` and `window.rs`.

use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use super::coords::cursor_to_session_point;
use super::{App, RunState, UserEvent, main_thread_marker, system_appearance};
use crate::core::session::{InputEvent, SessionPhase};
use crate::platform::overlay::Overlay;

impl ApplicationHandler<UserEvent> for App {
    /// One-time startup (guarded by `self.started`, since winit may call `resumed` more than
    /// once over an app's lifetime): builds the window/tray and shows the window unless this is
    /// a `--login` launch with the icon visible (DESIGN.md "Settings window"). The app launches as
    /// a regular app (no `LSUIElement`), so macOS brings it to the front like any other app it
    /// launches; `App::start` only switches to the `Accessory` policy when no window is shown.
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;

        force_appearance_for_testing();

        self.start(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let RunState::Cleaning(cleaning) = &mut self.state else {
            return;
        };
        if !cleaning.overlay.owns(window_id) {
            return;
        }

        if matches!(event, WindowEvent::RedrawRequested) {
            let now = Instant::now();
            let view = cleaning.session.view(now);
            let appearance = system_appearance();
            if let Err(err) =
                cleaning
                    .overlay
                    .redraw(&view, self.language, appearance, &mut cleaning.rasterizer)
            {
                eprintln!("urahafu: failed to redraw the cleaning overlay: {err}");
            }
            return;
        }

        // `mouseMoved` is never blocked by the input-tap event (`platform::input_blocker`'s
        // module docs), so the hover reveal is fed from winit's own `CursorMoved` instead,
        // converted into the session's coordinate space. This matters in every phase the overlay
        // is open in, not just the countdown, so it is handled ahead of the countdown-only path
        // below.
        if let WindowEvent::CursorMoved { position, .. } = &event {
            if let Some((origin, scale)) = cleaning.overlay.window_origin_and_scale(window_id) {
                let (x, y) = cursor_to_session_point(origin, (position.x, position.y), scale);
                let now = Instant::now();
                let commands = cleaning
                    .session
                    .handle_input(InputEvent::PointerMoved { x, y }, now);
                self.drive_session(event_loop, commands);
            }
            return;
        }

        // Only during the countdown: the input blocker isn't installed yet, so ordinary winit
        // window events are the only way to observe Escape/click to cancel
        // (`crate::platform::overlay::Overlay::translate_input`'s own docs).
        if cleaning.session.phase() != SessionPhase::Countdown {
            return;
        }
        let Some(input_event) = Overlay::translate_input(&event) else {
            return;
        };
        let now = Instant::now();
        let commands = cleaning.session.handle_input(input_event, now);
        self.drive_session(event_loop, commands);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Input(input_event) => {
                let RunState::Cleaning(cleaning) = &mut self.state else {
                    return;
                };
                let now = Instant::now();
                let commands = cleaning.session.handle_input(input_event, now);
                self.drive_session(event_loop, commands);
            }
            UserEvent::Menu(menu_event) => self.handle_menu_event(event_loop, &menu_event),
            UserEvent::Window(command) => self.handle_window_command(event_loop, command),
            UserEvent::Reopen => {
                // Ignored while a cleaning session runs (DESIGN.md "Settings window").
                if matches!(self.state, RunState::Idle) {
                    self.present_window(main_thread_marker());
                }
            }
        }
    }

    /// Drives timers: a cleaning session's next tick, or the idle-time permission watcher —
    /// never a busy loop.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match &self.state {
            RunState::Cleaning(_) => self.tick_cleaning(event_loop),
            RunState::Idle => self.tick_idle(event_loop),
        }
    }
}

/// Development builds only: `URAHAFU_APPEARANCE=dark` (or `light`) forces the app's appearance, so
/// the settings window can be checked in both modes without changing the system setting
/// (`docs/MANUAL_TESTING.md`). Compiled out of release builds.
fn force_appearance_for_testing() {
    #[cfg(debug_assertions)]
    {
        let name = match std::env::var("URAHAFU_APPEARANCE").as_deref() {
            Ok("dark") => "NSAppearanceNameDarkAqua",
            Ok("light") => "NSAppearanceNameAqua",
            _ => return,
        };
        let appearance = objc2_app_kit::NSAppearance::appearanceNamed(
            &objc2_foundation::NSString::from_str(name),
        );
        objc2_app_kit::NSApplication::sharedApplication(main_thread_marker())
            .setAppearance(appearance.as_deref());
    }
}
