//! The app's main menu bar (DESIGN.md "App main menu"), visible whenever the
//! app has the `Regular` activation policy (i.e. whenever the settings window can be shown —
//! `src/app/mod.rs`'s `App::present_window`/`hide_window`). Built once at startup and never
//! rebuilt or hidden itself (there is no menu-bar-icon-only equivalent of it — DESIGN.md's tray
//! menu, `platform::tray`, is the separate, always-available menu-bar-icon menu).
//!
//! Two menus:
//! - The App menu (its displayed title is replaced by the running process name by `AppKit`
//!   itself, regardless of what string is set here — standard `AppKit` behavior for the first
//!   menu-bar menu): "About Urahafu", "Settings…" (⌘,), "Quit Urahafu" (⌘Q).
//! - "Window": "Close" (⌘W), which calls `NSWindow`'s standard `performClose:` through the
//!   responder chain rather than a specific window this crate would have to track — see
//!   `crate::platform::ffi::menu_item::wire_to_perform_close`'s own doc comment.
//!
//! Every other item routes through the settings window's own [`ActionTarget`]
//! (`SettingsWindow::action_target`), exactly like its own controls (`settings_window`'s
//! `checkbox`/`popup_button` helpers) — new tags (this module's own [`tag`]), decoded into a
//! `src/app/window.rs`'s `WindowCommand` by the same `command_for_action` mapping.
//!
//! No custom drawing: every item is stock `AppKit`, safe code only — the two `unsafe` calls this
//! needs (`setTarget:`/`setAction:`) live in `crate::platform::ffi::menu_item`
//! (`docs/ARCHITECTURE.md` goal 4).

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::NSString;

use crate::core::i18n::{Language, Text};
use crate::platform::ffi::action_target::ActionTarget;
use crate::platform::ffi::menu_item;
use crate::platform::settings_window::tag as window_tag;

/// Tags for this module's own items — routed through the same [`ActionTarget`] and
/// `src/app/window.rs`'s `WindowCommand` mapping as the settings window's controls, so these must
/// not collide with `crate::platform::settings_window::tag`'s values (currently 1-9).
pub mod tag {
    /// The App menu's "Settings…" item: shows the settings window, like the tray's own item.
    pub const OPEN_SETTINGS: isize = 10;
    /// The App menu's "Quit Urahafu" item.
    pub const QUIT: isize = 11;
}

/// Builds the app's main menu bar and installs it. `target` is the settings window's own
/// [`ActionTarget`] — every action-routed item here reuses that single instance, the same one
/// wired to every settings-window control.
pub fn install(mtm: MainThreadMarker, language: Language, target: &ActionTarget) {
    let menu_bar = NSMenu::new(mtm);

    let about = action_item(
        mtm,
        Text::MenuAbout.get(language),
        "",
        window_tag::ABOUT,
        target,
    );
    let separator_1 = NSMenuItem::separatorItem(mtm);
    let settings = action_item(
        mtm,
        Text::MenuSettings.get(language),
        ",",
        tag::OPEN_SETTINGS,
        target,
    );
    let separator_2 = NSMenuItem::separatorItem(mtm);
    let quit = action_item(mtm, Text::MenuQuit.get(language), "q", tag::QUIT, target);
    add_menu(
        mtm,
        &menu_bar,
        "Urahafu",
        &[&about, &separator_1, &settings, &separator_2, &quit],
    );

    let close = NSMenuItem::new(mtm);
    close.setTitle(&NSString::from_str(Text::MenuClose.get(language)));
    close.setKeyEquivalent(&NSString::from_str("w"));
    menu_item::wire_to_perform_close(&close);
    add_menu(mtm, &menu_bar, Text::MenuWindow.get(language), &[&close]);

    NSApplication::sharedApplication(mtm).setMainMenu(Some(&menu_bar));
}

/// One `NSMenuItem` wired to `target`'s `performAction:`
/// (`crate::platform::ffi::menu_item::wire_to_action_target`), exactly like a settings-window
/// control — `tag_value` is how the callback (`src/app/window.rs`'s `command_for_action`) tells
/// items apart.
fn action_item(
    mtm: MainThreadMarker,
    title: &str,
    key_equivalent: &str,
    tag_value: isize,
    target: &ActionTarget,
) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    item.setKeyEquivalent(&NSString::from_str(key_equivalent));
    item.setTag(tag_value);
    menu_item::wire_to_action_target(&item, target);
    item
}

/// Appends one top-level menu titled `title`, containing `items` in order, to `menu_bar` — the
/// standard `AppKit` pattern of a top-level `NSMenuItem` with no action of its own, holding a
/// submenu.
fn add_menu(mtm: MainThreadMarker, menu_bar: &NSMenu, title: &str, items: &[&NSMenuItem]) {
    let submenu = NSMenu::new(mtm);
    submenu.setTitle(&NSString::from_str(title));
    for item in items {
        submenu.addItem(item);
    }
    let top_level = NSMenuItem::new(mtm);
    top_level.setTitle(&NSString::from_str(title));
    top_level.setSubmenu(Some(&submenu));
    menu_bar.addItem(&top_level);
}
