//! The application layer: winit's [`winit::application::ApplicationHandler`], wiring every other
//! module together (`docs/ARCHITECTURE.md` `src/app.rs`, DESIGN.md "Settings window").
//!
//! This module owns no business logic of its own — every decision (what the countdown does, what
//! text to show, when the fail-safe fires) already lives in [`crate::core`] or is delegated to a
//! `crate::platform` module. What is here is purely the glue: showing/hiding the settings window
//! and the tray depending on how the app was launched, driving the
//! [`crate::core::session::Session`] state machine from winit's event loop, mapping its
//! [`crate::core::session::SessionCommand`]s onto the input blocker and the overlay windows, and
//! mapping tray/window commands onto settings changes.
//!
//! Split across files once [`App`]'s own logic would no longer fit in one readable file
//! (`docs/ARCHITECTURE.md` "Keep app.rs readable"):
//! - `args.rs` — pure command-line argument parsing (`--login`).
//! - `startup.rs` — one-time startup: showing the window/tray depending on how the app was
//!   launched, and the permission watcher.
//! - `clean.rs` — the "Clean" flow: preflight, opening the session/overlay, driving
//!   [`crate::core::session::SessionCommand`]s (installing/releasing the input blocker, closing),
//!   and returning to the window or the tray once a session ends.
//! - `menu.rs` — tray menu command handling (Grant Access, Clean, Settings, Quit).
//! - `window.rs` — settings window command handling (settings toggles, login item, About,
//!   closing).
//! - `handler.rs` — the [`winit::application::ApplicationHandler`] impl itself, dispatching winit
//!   events to the methods above.

mod args;
mod clean;
mod coords;
mod handler;
mod menu;
mod startup;
mod window;

use std::path::PathBuf;
use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
use tray_icon::menu::MenuEvent;
use winit::event_loop::{ControlFlow, EventLoop, EventLoopProxy};

use crate::Error;
use crate::core::i18n::Language;
use crate::core::session::{InputEvent, Session};
use crate::core::settings::Settings;
use crate::platform::input_blocker::BlockerGuard;
use crate::platform::overlay::Overlay;
use crate::platform::render::Appearance;
use crate::platform::settings_window::SettingsWindow;
use crate::platform::system;
use crate::platform::text::TextRasterizer;
use crate::platform::tray::Tray;
use window::WindowCommand;

/// The project's GitHub repository, matching `Cargo.toml`'s `repository` field.
const GITHUB_URL: &str = "https://github.com/beeraw/urahafu";

/// How often to poll [`crate::platform::permission::is_trusted`] while idle and Accessibility
/// access is missing (DESIGN.md "Settings window"). No timeout: the call is cheap (a single
/// read, no prompt), and this watcher must keep working regardless of how the user got to System
/// Settings — the window's banner button, the permission-required error alert, or entirely on
/// their own.
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Custom winit user event: how the tray menu (its own event source, `tray-icon`/`muda`), the
/// settings window (its own event source, `NSControl`/`NSWindowDelegate` target-action via
/// [`crate::platform::ffi::action_target`]) and the input blocker's background thread (its own
/// `mpsc::Receiver`) get their events into the winit event loop, which only
/// [`ApplicationHandler::user_event`](winit::application::ApplicationHandler::user_event) lets
/// external code feed.
#[derive(Debug)]
pub(crate) enum UserEvent {
    /// One decoded input event, forwarded from the input blocker's tap thread by a small relay
    /// thread reading its `mpsc::Receiver` (see `clean.rs`).
    Input(InputEvent),
    /// A tray menu click, forwarded from `tray_icon::menu::MenuEvent::set_event_handler`.
    Menu(MenuEvent),
    /// A settings window control action.
    Window(WindowCommand),
    /// The Dock/Finder "reopen" Apple Event (the user opened the app again while it was already
    /// running).
    Reopen,
}

/// Whether [`App::tick_idle`]'s latest poll of [`crate::platform::permission::is_trusted`] just
/// flipped from missing to granted (DESIGN.md "Settings window") — the only transition that
/// refreshes the window/tray outside of a direct user action. Pure and side-effect-free so it is
/// unit-tested without any platform dependency.
#[must_use]
pub(crate) fn permission_just_granted(was_trusted: bool, is_trusted: bool) -> bool {
    !was_trusted && is_trusted
}

/// Where a running cleaning session was started from — decides where control returns once it
/// ends (DESIGN.md "Settings window").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanOrigin {
    /// Started from the settings window's "Clean" button: the window shows again once the
    /// session ends.
    Window,
    /// Started from the tray menu: back to idle, as before.
    Tray,
}

/// State of the app outside an active cleaning session.
enum RunState {
    /// Nothing running: waiting for a command, or between sessions. Also where the permission
    /// watcher runs — see [`App::tick_idle`].
    Idle,
    /// A cleaning session is in progress. Boxed: this variant is far larger than the others
    /// (an `Overlay` owning real windows/surfaces), and `RunState` is moved around by value.
    Cleaning(Box<CleaningSession>),
}

/// Everything owned by one in-progress cleaning session.
struct CleaningSession {
    session: Session,
    overlay: Overlay,
    /// `Some` only while inputs are actually blocked (between `SessionCommand::BlockInputs` and
    /// `SessionCommand::ReleaseInputs`); dropping it stops blocking input immediately
    /// (`crate::platform::input_blocker`).
    blocker: Option<BlockerGuard>,
    rasterizer: TextRasterizer,
    /// Where this session was started from (DESIGN.md "Settings window").
    origin: CleanOrigin,
}

/// The winit [`winit::application::ApplicationHandler`] implementation; see the module docs for
/// how its logic is split across files.
struct App {
    home: PathBuf,
    settings_path: PathBuf,
    settings: Settings,
    language: Language,
    /// Whether this run was launched with `--login` (from the `LaunchAgent`) — decided once at
    /// startup by [`args::is_login_launch`].
    is_login_launch: bool,
    proxy: EventLoopProxy<UserEvent>,
    /// Set once [`winit::application::ApplicationHandler::resumed`] has done its one-time
    /// startup work, so a later `resumed` call (winit may call it more than once over an app's
    /// lifetime) does not repeat it.
    started: bool,
    tray: Option<Tray>,
    /// The settings window, built once on the first `resumed` call and reused for the app's
    /// whole lifetime; `None` only before that first call.
    window: Option<SettingsWindow>,
    state: RunState,
    /// Last-known Accessibility trust state, used both to build the tray/window's permission UI
    /// and, by [`App::tick_idle`], to detect the missing → granted transition. Kept in sync by
    /// startup and by every [`App::tick_idle`] poll.
    accessibility_granted: bool,
}

impl App {
    fn new(
        home: PathBuf,
        settings_path: PathBuf,
        settings: Settings,
        language: Language,
        is_login_launch: bool,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Self {
        Self {
            home,
            settings_path,
            settings,
            language,
            is_login_launch,
            proxy,
            started: false,
            tray: None,
            window: None,
            state: RunState::Idle,
            accessibility_granted: false,
        }
    }

    /// Shows and activates the settings window, switching the activation policy to `Regular` so
    /// it gets a Dock icon and Cmd-Tab entry while visible (DESIGN.md "Settings window").
    fn present_window(&mut self, mtm: MainThreadMarker) {
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Regular);
        if let Some(window) = &self.window {
            window.show(mtm);
        }
    }

    /// Hides the settings window and switches back to the `Accessory` policy (no Dock icon) —
    /// the app keeps running in the menu bar.
    fn hide_window(&mut self) {
        if let Some(window) = &self.window {
            window.hide();
        }
        NSApplication::sharedApplication(main_thread_marker())
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

/// Obtains proof of running on the main thread.
///
/// # Panics
///
/// Panics if called off the main thread. Every call site in this module runs from inside a
/// [`winit::application::ApplicationHandler`] callback, which winit only ever invokes on the main
/// thread on macOS — so this can only panic if that platform guarantee is broken.
#[allow(
    clippy::expect_used,
    reason = "winit's ApplicationHandler callbacks are documented to only ever run on the main \
              thread on macOS; there is no useful fallback if that guarantee were ever broken"
)]
fn main_thread_marker() -> MainThreadMarker {
    MainThreadMarker::new().expect("ApplicationHandler callbacks run on the main thread")
}

/// The keyboard-only HUD's current system appearance (DESIGN.md §9), read from
/// `NSApplication.effectiveAppearance`. Compared by name rather than against the `NSAppearanceName*`
/// constants: those are `extern "C"` statics, and reading one requires `unsafe`, which this module
/// (outside `crate::platform::ffi`) may not use; matching the (safe, `Display`-able) appearance
/// name string avoids that without adding a new `ffi` submodule for a single string comparison.
fn system_appearance() -> Appearance {
    let app = NSApplication::sharedApplication(main_thread_marker());
    let name = app.effectiveAppearance().name().to_string();
    if name.contains("Dark") {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// Runs the application: builds the winit event loop and drives [`App`] until it exits.
///
/// # Errors
///
/// Returns [`Error`] if the home directory cannot be located, or if building or running the
/// winit event loop fails. Nothing inside the running app is fatal anymore (a failure to build
/// the tray or open a URL, for instance, is logged and handled locally instead): the settings
/// window is always still there to fall back on.
pub(crate) fn run() -> Result<(), Error> {
    let home = system::home_dir()?;
    let settings_path = Settings::default_path(&home);
    let settings = Settings::load(&settings_path).unwrap_or_else(|err| {
        eprintln!("urahafu: failed to load settings, using defaults: {err}");
        Settings::default()
    });

    let preferred: Vec<String> = system::preferred_languages();
    let preferred_refs: Vec<&str> = preferred.iter().map(String::as_str).collect();
    let language = Language::from_preferred(&preferred_refs);

    let args: Vec<String> = std::env::args().collect();
    let is_login_launch = args::is_login_launch(args.iter().map(String::as_str));

    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));

    let mut app = App::new(
        home,
        settings_path,
        settings,
        language,
        is_login_launch,
        proxy,
    );
    event_loop.run_app(&mut app)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_just_granted_detects_the_missing_to_granted_flip() {
        assert!(permission_just_granted(false, true));
    }

    #[test]
    fn permission_just_granted_ignores_every_other_transition() {
        assert!(!permission_just_granted(true, true));
        assert!(!permission_just_granted(false, false));
        assert!(!permission_just_granted(true, false));
    }
}
