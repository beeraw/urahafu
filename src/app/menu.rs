//! Tray menu command handling (DESIGN.md §6: the reduced menu).

use tray_icon::menu::MenuEvent;
use winit::event_loop::ActiveEventLoop;

use super::{App, RunState, main_thread_marker};
use crate::platform::permission;
use crate::platform::tray::{Tray, TrayCommand};

impl App {
    /// Decodes and dispatches one tray [`MenuEvent`]; unrelated events (from another tray icon,
    /// in principle) are ignored.
    pub(super) fn handle_menu_event(&mut self, event_loop: &ActiveEventLoop, event: &MenuEvent) {
        let Some(command) = Tray::command_for(event) else {
            return;
        };
        match command {
            TrayCommand::GrantAccess => {
                if let Err(err) = permission::open_accessibility_settings() {
                    eprintln!("urahafu: failed to open System Settings: {err}");
                }
            }
            TrayCommand::Clean => {
                // Ignore "Clean" while a session is already running.
                if matches!(self.state, RunState::Idle) {
                    self.start_clean_from_tray(event_loop);
                }
            }
            TrayCommand::Settings => self.present_window(main_thread_marker()),
            TrayCommand::Quit => self.quit(event_loop),
        }
    }

    pub(super) fn quit(&mut self, event_loop: &ActiveEventLoop) {
        if let RunState::Cleaning(cleaning) = &mut self.state {
            // Drop the blocker guard before anything else so a quit mid-session can never leave
            // input blocked.
            cleaning.blocker = None;
            cleaning.overlay.close();
        }
        self.state = RunState::Idle;
        self.tray = None;
        self.window = None;
        event_loop.exit();
    }
}
