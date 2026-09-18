//! One-time startup (DESIGN.md "Settings window"): builds the settings window and the
//! tray, shows the window unless this is a login launch with the icon visible, and runs the
//! permission watcher whenever idle and Accessibility access is missing.

use std::time::Instant;

use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use winit::event_loop::{ActiveEventLoop, ControlFlow};

use super::{App, PERMISSION_POLL_INTERVAL, main_thread_marker, permission_just_granted};
use crate::platform::ffi::action_target::ActionId;
use crate::platform::settings_window::SettingsWindow;
use crate::platform::tray::Tray;
use crate::platform::{app_menu, login_item, permission};

impl App {
    /// Builds the window and (if configured) the tray, then shows the window unless this is a
    /// login launch with the icon visible (DESIGN.md "Settings window"): a normal launch
    /// always shows the window; a `--login` launch shows it only if the icon is hidden (never run
    /// invisibly). The permission watcher itself needs no explicit start: [`App::tick_idle`]
    /// polls on its own whenever `self.state` is [`super::RunState::Idle`] and Accessibility is
    /// still untrusted.
    pub(super) fn start(&mut self, event_loop: &ActiveEventLoop) {
        let mtm = main_thread_marker();
        let trusted = permission::is_trusted();
        self.accessibility_granted = trusted;

        let proxy = self.proxy.clone();
        let window = SettingsWindow::new(mtm, self.language, move |action: ActionId| {
            let event = match action {
                ActionId::Reopen => super::UserEvent::Reopen,
                other => match super::window::command_for_action(other) {
                    Some(command) => super::UserEvent::Window(command),
                    None => return,
                },
            };
            let _ = proxy.send_event(event);
        });
        let login_enabled = login_item::is_enabled(&self.home);
        window.update(&self.settings, login_enabled, trusted, self.language);

        // Built once, alongside the window whose ActionTarget it reuses (DESIGN.md "App main menu");
        // shown whenever the app is Regular (App::present_window/hide_window), like every other
        // app's main menu — it is not itself shown/hidden separately.
        app_menu::install(mtm, self.language, window.action_target());

        self.window = Some(window);

        if self.settings.show_menu_bar_icon {
            match Tray::new(&self.settings, self.language, trusted) {
                Ok(tray) => self.tray = Some(tray),
                Err(err) => {
                    // The window still lets the app be used, so this is no longer fatal on its
                    // own — log it and keep going without a tray icon.
                    eprintln!("urahafu: failed to create the tray icon: {err}");
                }
            }
        }

        let show_window = if self.is_login_launch {
            !self.settings.show_menu_bar_icon
        } else {
            true
        };
        if show_window {
            self.present_window(mtm);
        } else {
            // Login launch with the icon shown: menu bar only, no Dock icon.
            NSApplication::sharedApplication(mtm)
                .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }

        let _ = event_loop;
    }

    /// One [`super::RunState::Idle`] tick (DESIGN.md "Settings window"): while Accessibility is
    /// untrusted, polls [`permission::is_trusted`] every [`PERMISSION_POLL_INTERVAL`]
    /// (`ControlFlow::WaitUntil`, never a busy loop, no timeout — the call is cheap). Runs
    /// regardless of how the app was launched or how the user got to System Settings (the
    /// window's banner button, the permission-required error alert, or entirely on their own).
    /// The moment trust flips from missing to granted, refreshes the window/tray (banner
    /// disappears, Clean enabled) — no alert anymore.
    pub(super) fn tick_idle(&mut self, event_loop: &ActiveEventLoop) {
        let trusted = permission::is_trusted();
        if permission_just_granted(self.accessibility_granted, trusted) {
            self.accessibility_granted = trusted;
            self.refresh_window_and_tray();
        } else {
            self.accessibility_granted = trusted;
        }

        if trusted {
            event_loop.set_control_flow(ControlFlow::Wait);
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + PERMISSION_POLL_INTERVAL,
            ));
        }
    }
}
