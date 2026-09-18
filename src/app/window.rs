//! Settings window command handling (DESIGN.md "Settings window"): decoding
//! [`ActionId`]s into [`WindowCommand`]s, and applying them (settings changes, showing/hiding the
//! window and tray, About, Clean).

use winit::event_loop::ActiveEventLoop;

use super::{App, GITHUB_URL, RunState, main_thread_marker};
use crate::core::color::CleaningColor;
use crate::platform::alert::{self, AlertSpec};
use crate::platform::app_menu::tag as menu_tag;
use crate::platform::ffi::action_target::ActionId;
use crate::platform::settings_window::{SettingsWindow, tag};
use crate::platform::tray::Tray;
use crate::platform::{login_item, permission, system};

/// One decoded settings-window (or app main menu) action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowCommand {
    /// The permission banner's button: opens System Settings on the Accessibility pane.
    GrantAccess,
    /// A color swatch was clicked; carries the color it picked directly (there are now two
    /// independent swatch buttons rather than one control with a selection index, so the tag
    /// itself — decoded in [`command_for_action`] — is the only place that knows which one fired).
    SetColor(CleaningColor),
    /// The auto-unlock popup changed.
    SetFailsafe,
    /// The "Keyboard-Only Mode" checkbox changed.
    ToggleKeyboardOnly,
    /// The "Show Icon in Menu Bar" checkbox changed.
    ToggleShowIcon,
    /// The "Open at Login" checkbox changed.
    ToggleLogin,
    /// The "Clean Screen"/"Clean Keyboard" button.
    Clean,
    /// The "About Urahafu" button/menu item.
    About,
    /// The app main menu's "Settings…" item (DESIGN.md "App main menu"): shows the settings window,
    /// exactly like the tray's own "Settings…" item.
    ShowSettings,
    /// The app main menu's "Quit Urahafu" item (⌘Q, DESIGN.md "App main menu"): quits the app, dropping
    /// any active blocker first, exactly like the tray's "Quit Urahafu" item.
    Quit,
    /// The window was asked to close (red button, Cmd-W, or Esc).
    Close,
}

/// Decodes an [`ActionId`] into a [`WindowCommand`]. `None` for [`ActionId::Reopen`] (handled
/// separately as `UserEvent::Reopen`, not a window command) or an unrecognized tag. Pure and
/// side-effect-free so it is unit-tested without `AppKit`.
#[must_use]
pub(crate) fn command_for_action(action: ActionId) -> Option<WindowCommand> {
    match action {
        ActionId::Close => Some(WindowCommand::Close),
        ActionId::Reopen => None,
        ActionId::Tag(value) => match value {
            tag::GRANT_ACCESS => Some(WindowCommand::GrantAccess),
            tag::COLOR_BLACK => Some(WindowCommand::SetColor(CleaningColor::Black)),
            tag::COLOR_WHITE => Some(WindowCommand::SetColor(CleaningColor::White)),
            tag::FAILSAFE => Some(WindowCommand::SetFailsafe),
            tag::KEYBOARD_ONLY => Some(WindowCommand::ToggleKeyboardOnly),
            tag::SHOW_ICON => Some(WindowCommand::ToggleShowIcon),
            tag::OPEN_AT_LOGIN => Some(WindowCommand::ToggleLogin),
            tag::ABOUT => Some(WindowCommand::About),
            tag::CLEAN => Some(WindowCommand::Clean),
            tag::ESCAPE_CLOSE => Some(WindowCommand::Close),
            menu_tag::OPEN_SETTINGS => Some(WindowCommand::ShowSettings),
            menu_tag::QUIT => Some(WindowCommand::Quit),
            _ => None,
        },
    }
}

/// What closing the window (red button, Cmd-W, Esc) does: hide it (the app keeps running in the
/// menu bar) if the tray icon is currently shown, or quit the app entirely if not — there would
/// be no way left to reach it. Pure and side-effect-free so it is unit-tested without `AppKit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseAction {
    /// Hide the window; the app keeps running in the menu bar.
    Hide,
    /// Quit the app.
    Quit,
}

#[must_use]
pub(crate) fn close_action(tray_visible: bool) -> CloseAction {
    if tray_visible {
        CloseAction::Hide
    } else {
        CloseAction::Quit
    }
}

impl App {
    /// Dispatches one decoded [`WindowCommand`].
    pub(super) fn handle_window_command(
        &mut self,
        event_loop: &ActiveEventLoop,
        command: WindowCommand,
    ) {
        match command {
            WindowCommand::GrantAccess => {
                if let Err(err) = permission::open_accessibility_settings() {
                    eprintln!("urahafu: failed to open System Settings: {err}");
                }
            }
            WindowCommand::SetColor(color) => {
                self.settings.color = color;
                self.persist_settings();
            }
            WindowCommand::SetFailsafe => {
                let Some(failsafe) = self.window.as_ref().map(SettingsWindow::selected_failsafe)
                else {
                    return;
                };
                self.settings.failsafe = failsafe;
                self.persist_settings();
            }
            WindowCommand::ToggleKeyboardOnly => {
                let Some(keyboard_only) = self
                    .window
                    .as_ref()
                    .map(SettingsWindow::keyboard_only_checked)
                else {
                    return;
                };
                self.settings.keyboard_only = keyboard_only;
                self.persist_settings();
            }
            WindowCommand::ToggleShowIcon => {
                let Some(show) = self.window.as_ref().map(SettingsWindow::show_icon_checked) else {
                    return;
                };
                self.set_show_icon(show);
            }
            WindowCommand::ToggleLogin => self.toggle_login(),
            WindowCommand::Clean => {
                // Ignore while a session is already running (mirrors the tray's "Clean").
                if matches!(self.state, RunState::Idle) {
                    self.start_clean_from_window(event_loop);
                }
            }
            WindowCommand::About => self.show_about(),
            WindowCommand::ShowSettings => self.present_window(main_thread_marker()),
            WindowCommand::Quit => self.quit(event_loop),
            WindowCommand::Close => self.close_window(event_loop),
        }
    }

    /// Saves `self.settings` (logging, never propagating, a failure — a disk error here must not
    /// crash the app or block the window) and refreshes the window/tray from the new state.
    pub(super) fn persist_settings(&mut self) {
        if let Err(err) = self.settings.save(&self.settings_path) {
            eprintln!("urahafu: failed to save settings: {err}");
        }
        self.refresh_window_and_tray();
    }

    /// Refreshes the window and (if shown) the tray from `self.settings`/`self.accessibility_granted`.
    pub(super) fn refresh_window_and_tray(&mut self) {
        let login_enabled = login_item::is_enabled(&self.home);
        if let Some(window) = &self.window {
            window.update(
                &self.settings,
                login_enabled,
                self.accessibility_granted,
                self.language,
            );
        }
        if let Some(tray) = &mut self.tray {
            tray.update(&self.settings, self.accessibility_granted);
        }
    }

    /// "Show Icon in Menu Bar" applies live: creates or drops the tray immediately, no
    /// confirmation, no quit. Turning it off also disables the login item, since it no longer
    /// makes sense without an icon to normally reach it from.
    fn set_show_icon(&mut self, show: bool) {
        self.settings.show_menu_bar_icon = show;
        if show {
            if self.tray.is_none() {
                match Tray::new(&self.settings, self.language, self.accessibility_granted) {
                    Ok(tray) => self.tray = Some(tray),
                    Err(err) => eprintln!("urahafu: failed to create the tray icon: {err}"),
                }
            }
        } else {
            self.tray = None;
            if let Err(err) = login_item::disable(&self.home) {
                eprintln!("urahafu: failed to disable the login item: {err}");
            }
        }
        self.persist_settings();
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
        self.refresh_window_and_tray();
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

    /// Closing the window (red button, Cmd-W, Esc — DESIGN.md "Settings window"): hide it if
    /// the tray icon is shown, or quit the app entirely otherwise.
    fn close_window(&mut self, event_loop: &ActiveEventLoop) {
        match close_action(self.tray.is_some()) {
            CloseAction::Hide => self.hide_window(),
            CloseAction::Quit => self.quit(event_loop),
        }
    }
}

pub(super) fn open_repository() {
    if let Err(err) = system::open_url(GITHUB_URL) {
        eprintln!("urahafu: failed to open {GITHUB_URL}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_maps_to_the_matching_command() {
        assert_eq!(
            command_for_action(ActionId::Tag(tag::GRANT_ACCESS)),
            Some(WindowCommand::GrantAccess)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::COLOR_BLACK)),
            Some(WindowCommand::SetColor(CleaningColor::Black))
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::COLOR_WHITE)),
            Some(WindowCommand::SetColor(CleaningColor::White))
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::FAILSAFE)),
            Some(WindowCommand::SetFailsafe)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::KEYBOARD_ONLY)),
            Some(WindowCommand::ToggleKeyboardOnly)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::SHOW_ICON)),
            Some(WindowCommand::ToggleShowIcon)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::OPEN_AT_LOGIN)),
            Some(WindowCommand::ToggleLogin)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::ABOUT)),
            Some(WindowCommand::About)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::CLEAN)),
            Some(WindowCommand::Clean)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(tag::ESCAPE_CLOSE)),
            Some(WindowCommand::Close)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(menu_tag::OPEN_SETTINGS)),
            Some(WindowCommand::ShowSettings)
        );
        assert_eq!(
            command_for_action(ActionId::Tag(menu_tag::QUIT)),
            Some(WindowCommand::Quit)
        );
    }

    /// The app main menu's own tags (`crate::platform::app_menu::tag`) must not collide with the
    /// settings window's (`crate::platform::settings_window::tag`) — both feed the same
    /// [`command_for_action`] mapping through the same shared `ActionTarget`.
    #[test]
    fn menu_tags_do_not_collide_with_settings_window_tags() {
        let settings_window_tags = [
            tag::GRANT_ACCESS,
            tag::COLOR_BLACK,
            tag::FAILSAFE,
            tag::KEYBOARD_ONLY,
            tag::SHOW_ICON,
            tag::OPEN_AT_LOGIN,
            tag::ABOUT,
            tag::CLEAN,
            tag::ESCAPE_CLOSE,
            tag::COLOR_WHITE,
        ];
        assert!(!settings_window_tags.contains(&menu_tag::OPEN_SETTINGS));
        assert!(!settings_window_tags.contains(&menu_tag::QUIT));
        assert_ne!(menu_tag::OPEN_SETTINGS, menu_tag::QUIT);
    }

    #[test]
    fn unknown_tag_is_none() {
        assert_eq!(command_for_action(ActionId::Tag(-1)), None);
    }

    #[test]
    fn close_maps_to_close() {
        assert_eq!(
            command_for_action(ActionId::Close),
            Some(WindowCommand::Close)
        );
    }

    #[test]
    fn reopen_is_not_a_window_command() {
        assert_eq!(command_for_action(ActionId::Reopen), None);
    }

    #[test]
    fn close_action_hides_when_tray_is_visible() {
        assert_eq!(close_action(true), CloseAction::Hide);
    }

    #[test]
    fn close_action_quits_when_tray_is_hidden() {
        assert_eq!(close_action(false), CloseAction::Quit);
    }
}
