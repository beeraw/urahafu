//! The [`winit::application::ApplicationHandler`] impl: dispatches winit events to the methods in
//! `startup.rs`, `clean.rs` and `menu.rs`.

use std::time::Instant;

use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use super::coords::cursor_to_session_point;
use super::{App, LaunchMode, RunState, UserEvent, main_thread_marker, system_appearance};
use crate::core::session::{InputEvent, SessionPhase};
use crate::platform::overlay::Overlay;

impl ApplicationHandler<UserEvent> for App {
    /// One-time startup (guarded by `self.started`, since winit may call `resumed` more than
    /// once over an app's lifetime): sets the Accessory activation policy (no Dock icon), then
    /// starts the launch mode decided in [`super::run`].
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.started {
            return;
        }
        self.started = true;

        NSApplication::sharedApplication(main_thread_marker())
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        match self.mode {
            LaunchMode::MenuBar => self.start_menu_bar_mode(event_loop),
            LaunchMode::Direct => self.start_direct_mode(event_loop),
        }
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
        }
    }

    /// Drives timers: a cleaning session's next tick, or the idle-time permission watcher
    /// (DESIGN.md §10) — never a busy loop.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match &self.state {
            RunState::Cleaning(_) => self.tick_cleaning(event_loop),
            RunState::Idle => self.tick_idle(event_loop),
        }
    }
}
