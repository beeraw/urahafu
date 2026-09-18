//! The two launch modes, the first-launch welcome/permission alerts, and the permission watcher
//! (DESIGN.md §6, §10-§11).

use std::time::Instant;

use winit::event_loop::{ActiveEventLoop, ControlFlow};

use super::{
    App, LaunchMode, PERMISSION_POLL_INTERVAL, StartupAlert, main_thread_marker,
    permission_just_granted, startup_alerts,
};
use crate::platform::alert::{self, AlertSpec};
use crate::platform::tray::Tray;
use crate::platform::{login_item, permission};

impl App {
    /// Menu-bar mode startup (DESIGN.md §6, §10): builds the tray (showing item 0 iff
    /// Accessibility isn't trusted yet), then shows whichever startup alerts
    /// [`super::startup_alerts`] says are due, in order. The permission watcher itself needs no
    /// explicit start: [`App::tick_idle`] polls on its own whenever `self.mode` is
    /// [`LaunchMode::MenuBar`], `self.state` is [`RunState::Idle`] and Accessibility is still
    /// untrusted, regardless of how that state was reached.
    pub(super) fn start_menu_bar_mode(&mut self, event_loop: &ActiveEventLoop) {
        let login_enabled = login_item::is_enabled(&self.home);
        let trusted = permission::is_trusted();
        self.accessibility_granted = trusted;

        match Tray::new(&self.settings, self.language, login_enabled, trusted) {
            Ok(tray) => self.tray = Some(tray),
            Err(err) => {
                self.fail(event_loop, err);
                return;
            }
        }

        for alert in startup_alerts(self.settings.welcome_seen, trusted) {
            match alert {
                StartupAlert::Welcome => self.show_welcome(trusted),
                StartupAlert::FirstLaunchPermission => self.show_first_launch_permission_alert(),
            }
        }
    }

    /// Shows the welcome alert (DESIGN.md §10) and marks `welcome_seen`, saving the settings
    /// right away (logging, never propagating, a failure — same rule as every other settings
    /// write in this app: a disk error here must not crash the app or block startup).
    fn show_welcome(&mut self, trusted: bool) {
        let _ = alert::show(
            &AlertSpec::welcome(self.language, trusted),
            main_thread_marker(),
        );
        self.settings.welcome_seen = true;
        if let Err(err) = self.settings.save(&self.settings_path) {
            eprintln!("urahafu: failed to save settings: {err}");
        }
    }

    /// Shows the first-launch permission alert (DESIGN.md §10, "Accessibility permission
    /// request") and opens System Settings if its default button was picked.
    fn show_first_launch_permission_alert(&mut self) {
        let choice = alert::show(
            &AlertSpec::first_launch(self.language),
            main_thread_marker(),
        );
        // `AlertSpec::first_launch`'s buttons are `[Open System Settings, Later]`.
        if choice == 0 {
            if let Err(err) = permission::open_accessibility_settings() {
                eprintln!("urahafu: failed to open System Settings: {err}");
            }
        }
    }

    /// Direct-launch mode startup (DESIGN.md §11): without Accessibility access, cleaning can
    /// never start, so this shows the error alert and exits without ever opening the overlay.
    pub(super) fn start_direct_mode(&mut self, event_loop: &ActiveEventLoop) {
        if !permission::is_trusted() {
            let choice = alert::show(
                &AlertSpec::permission_required(self.language),
                main_thread_marker(),
            );
            // `AlertSpec::permission_required`'s buttons are `[Open System Settings, Cancel]`.
            if choice == 0 {
                if let Err(err) = permission::open_accessibility_settings() {
                    eprintln!("urahafu: failed to open System Settings: {err}");
                }
            }
            event_loop.exit();
            return;
        }
        self.try_start_clean(event_loop);
    }

    /// One [`RunState::Idle`] tick (DESIGN.md §10 "Permission monitoring"): while in
    /// menu-bar mode and Accessibility is untrusted, polls [`permission::is_trusted`] every
    /// [`PERMISSION_POLL_INTERVAL`] (`ControlFlow::WaitUntil`, never a busy loop, no timeout —
    /// the call is cheap). Runs independently of how the app got here (first-launch alert
    /// Open/Later, the permission-required error alert, or a manual grant in System Settings).
    /// The moment trust flips from missing to granted, shows the "ready" confirmation once and
    /// refreshes the tray (item 0 disappears) — this only ever runs from [`RunState::Idle`], so
    /// it naturally never fires during a cleaning session and is deferred until one ends.
    pub(super) fn tick_idle(&mut self, event_loop: &ActiveEventLoop) {
        if self.mode != LaunchMode::MenuBar {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        let trusted = permission::is_trusted();
        if permission_just_granted(self.accessibility_granted, trusted) {
            self.accessibility_granted = trusted;
            let _ = alert::show(
                &AlertSpec::first_launch_ready(self.language),
                main_thread_marker(),
            );
            self.refresh_tray();
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
