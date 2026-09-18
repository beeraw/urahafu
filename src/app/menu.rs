//! Tray menu command handling (DESIGN.md §6).

use tray_icon::menu::MenuEvent;
use winit::event_loop::ActiveEventLoop;

use super::{App, GITHUB_URL, RunState, main_thread_marker};
use crate::platform::alert::{self, AlertSpec};
use crate::platform::tray::{Tray, TrayCommand};
use crate::platform::{login_item, permission, system};

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
                // DESIGN.md/`docs/ARCHITECTURE.md`: ignore "Clean" while a session is already
                // running.
                if matches!(self.state, RunState::Idle) {
                    self.try_start_clean(event_loop);
                }
            }
            TrayCommand::ToggleKeyboardOnly => {
                self.settings.keyboard_only = !self.settings.keyboard_only;
                self.persist_settings();
            }
            TrayCommand::SetColor(color) => {
                self.settings.color = color;
                self.persist_settings();
            }
            TrayCommand::SetFailsafe(delay) => {
                self.settings.failsafe = delay;
                self.persist_settings();
            }
            TrayCommand::ToggleShowIcon => self.toggle_show_icon(event_loop),
            TrayCommand::ToggleLogin => self.toggle_login(),
            TrayCommand::About => self.show_about(),
            TrayCommand::OpenRepository => open_repository(),
            TrayCommand::Quit => self.quit(event_loop),
        }
    }

    /// Saves `self.settings` (logging, never propagating, a failure — `docs/ARCHITECTURE.md`
    /// "the user never gets stuck" extends to settings I/O too: a disk error here must not crash
    /// the app or block the menu) and refreshes the tray's check marks/labels from the new state.
    fn persist_settings(&mut self) {
        if let Err(err) = self.settings.save(&self.settings_path) {
            eprintln!("urahafu: failed to save settings: {err}");
        }
        self.refresh_tray();
    }

    /// Refreshes every check mark/label and item 0's presence (DESIGN.md §6) from the current
    /// `self.settings`/`self.accessibility_granted`. Also called by `startup.rs`'s permission
    /// watcher once it observes access being granted (DESIGN.md §10).
    pub(super) fn refresh_tray(&mut self) {
        let login_enabled = login_item::is_enabled(&self.home);
        if let Some(tray) = &mut self.tray {
            tray.update(&self.settings, login_enabled, self.accessibility_granted);
        }
    }

    fn toggle_login(&mut self) {
        let enabled = login_item::is_enabled(&self.home);
        let result = if enabled {
            login_item::disable(&self.home)
        } else {
            match std::env::current_exe() {
                Ok(exe) => login_item::enable(&self.home, &exe),
                Err(err) => {
                    eprintln!("urahafu: failed to locate the running executable: {err}");
                    return;
                }
            }
        };
        if let Err(err) = result {
            eprintln!("urahafu: failed to update the login item: {err}");
        }
        self.refresh_tray();
    }

    /// Unchecking "Show Icon in Menu Bar" asks for confirmation first (DESIGN.md §6): it changes
    /// how the app launches from then on. Checking it back on needs no confirmation.
    fn toggle_show_icon(&mut self, event_loop: &ActiveEventLoop) {
        if !self.settings.show_menu_bar_icon {
            self.settings.show_menu_bar_icon = true;
            self.persist_settings();
            return;
        }

        let choice = alert::show(
            &AlertSpec::hide_icon_confirm(self.language),
            main_thread_marker(),
        );
        if choice != 0 {
            return; // Cancel: leave the setting untouched.
        }

        self.settings.show_menu_bar_icon = false;
        if let Err(err) = self.settings.save(&self.settings_path) {
            eprintln!("urahafu: failed to save settings: {err}");
        }
        if let Err(err) = login_item::disable(&self.home) {
            eprintln!("urahafu: failed to disable the login item: {err}");
        }
        // DESIGN.md §6: the next launch is direct mode; nothing left to do with this run but
        // quit.
        self.quit(event_loop);
    }

    fn show_about(&mut self) {
        let choice = alert::show(
            &AlertSpec::about(self.language, env!("CARGO_PKG_VERSION")),
            main_thread_marker(),
        );
        // `AlertSpec::about`'s buttons are `[OK, View on GitHub]`.
        if choice == 1 {
            open_repository();
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
        event_loop.exit();
    }
}

fn open_repository() {
    if let Err(err) = system::open_url(GITHUB_URL) {
        eprintln!("urahafu: failed to open {GITHUB_URL}: {err}");
    }
}
