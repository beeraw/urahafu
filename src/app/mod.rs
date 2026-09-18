//! The application layer: winit's [`winit::application::ApplicationHandler`], wiring every other
//! module together (`docs/ARCHITECTURE.md` `src/app.rs`, DESIGN.md §6, §10-§12).
//!
//! This module owns no business logic of its own — every decision (what the countdown does, what
//! text to show, when the fail-safe fires) already lives in [`crate::core`] or is delegated to a
//! `crate::platform` module. What is here is purely the glue: deciding which mode to start in,
//! driving the [`crate::core::session::Session`] state machine from winit's event loop, mapping
//! its [`crate::core::session::SessionCommand`]s onto the input blocker and the overlay windows,
//! and mapping tray menu clicks onto settings changes.
//!
//! Split across files once [`App`]'s own logic would no longer fit in one readable file
//! (`docs/ARCHITECTURE.md` "Keep app.rs readable"):
//! - `startup.rs` — the two launch modes (menu-bar vs. direct) and the first-launch permission
//!   wait.
//! - `clean.rs` — the "Clean" flow: preflight, opening the session/overlay, driving
//!   [`crate::core::session::SessionCommand`]s (installing/releasing the input blocker, closing).
//! - `menu.rs` — tray menu command handling (settings toggles, login item, hide-icon
//!   confirmation, About, Quit).
//! - `handler.rs` — the [`winit::application::ApplicationHandler`] impl itself, dispatching winit
//!   events to the methods above.

mod clean;
mod coords;
mod handler;
mod menu;
mod startup;

use std::path::PathBuf;
use std::time::Duration;

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use tray_icon::menu::MenuEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

use crate::Error;
use crate::core::i18n::Language;
use crate::core::session::{InputEvent, Session};
use crate::core::settings::Settings;
use crate::platform::input_blocker::BlockerGuard;
use crate::platform::overlay::Overlay;
use crate::platform::render::Appearance;
use crate::platform::system;
use crate::platform::text::TextRasterizer;
use crate::platform::tray::Tray;

/// The project's GitHub repository (DESIGN.md §6 item 8, §12 "View on GitHub"), matching
/// `Cargo.toml`'s `repository` field.
const GITHUB_URL: &str = "https://github.com/beeraw/urahafu";

/// How often to poll [`crate::platform::permission::is_trusted`] while in menu-bar mode and
/// Accessibility access is missing (DESIGN.md §10 "Permission monitoring"). No timeout:
/// the call is cheap (a single read, no prompt), and unlike the old one-shot wait after the
/// first-launch alert, this watcher must keep working regardless of how the user got to System
/// Settings — first-launch alert, the permission-required error alert (DESIGN.md §11 case 1), or
/// entirely on their own.
const PERMISSION_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Custom winit user event: how the tray menu (its own event source, `tray-icon`/`muda`) and the
/// input blocker's background thread (its own `mpsc::Receiver`) get their events into the winit
/// event loop, which only [`ApplicationHandler::user_event`](winit::application::ApplicationHandler::user_event)
/// lets external code feed.
#[derive(Debug)]
pub(crate) enum UserEvent {
    /// One decoded input event, forwarded from the input blocker's tap thread by a small relay
    /// thread reading its `mpsc::Receiver` (see `clean.rs`).
    Input(InputEvent),
    /// A tray menu click, forwarded from `tray_icon::menu::MenuEvent::set_event_handler`.
    Menu(MenuEvent),
}

/// Which of the two launch modes (DESIGN.md §6, §10-§11) the app starts in, decided once at
/// startup from [`Settings::show_menu_bar_icon`] and whether Option was held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    /// Show the tray icon and menu; cleaning starts only when the user picks "Clean" from it.
    MenuBar,
    /// No tray icon: a cleaning session starts immediately, and the app exits once it ends (or
    /// after an error alert).
    Direct,
}

/// Decides the launch mode (DESIGN.md §6 "hold Option while opening Urahafu", §10-§11
/// "direct-launch mode"): menu-bar mode whenever the icon is configured to show, or Option is
/// held (which reopens the menu even when the icon is normally hidden); direct mode otherwise.
///
/// Pure and side-effect-free so it is unit-tested without any platform dependency.
#[must_use]
pub(crate) fn launch_mode(show_menu_bar_icon: bool, option_held: bool) -> LaunchMode {
    if show_menu_bar_icon || option_held {
        LaunchMode::MenuBar
    } else {
        LaunchMode::Direct
    }
}

/// One alert [`App::start_menu_bar_mode`] shows at startup, in the order [`startup_alerts`]
/// returns them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupAlert {
    /// DESIGN.md §10 "Welcome alert".
    Welcome,
    /// DESIGN.md §10 "Accessibility permission request".
    FirstLaunchPermission,
}

/// Decides which alerts [`App::start_menu_bar_mode`] shows, and in what order, given whether the
/// welcome alert (DESIGN.md §10) was already shown (`welcome_seen`, [`crate::core::settings::Settings`])
/// and whether Accessibility is currently trusted. Pure and side-effect-free so it is
/// unit-tested without any platform dependency; [`App::start_menu_bar_mode`] is the only caller,
/// and drives the actual `NSAlert`s in this exact order.
#[must_use]
pub(crate) fn startup_alerts(welcome_seen: bool, trusted: bool) -> Vec<StartupAlert> {
    let mut alerts = Vec::new();
    if !welcome_seen {
        alerts.push(StartupAlert::Welcome);
    }
    if !trusted {
        alerts.push(StartupAlert::FirstLaunchPermission);
    }
    alerts
}

/// Whether [`App::tick_idle`]'s latest poll of [`crate::platform::permission::is_trusted`] just
/// flipped from missing to granted (DESIGN.md §10 "Permission monitoring") — the only
/// transition that shows the "ready" confirmation. Pure and side-effect-free so it is
/// unit-tested without any platform dependency.
#[must_use]
pub(crate) fn permission_just_granted(was_trusted: bool, is_trusted: bool) -> bool {
    !was_trusted && is_trusted
}

/// State of the app outside an active cleaning session.
enum RunState {
    /// Nothing running: menu-bar mode waiting for a command, or between sessions. Also where the
    /// permission watcher runs (DESIGN.md §10 "Permission monitoring") — see
    /// [`App::tick_idle`].
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
}

/// The winit [`winit::application::ApplicationHandler`] implementation; see the module docs for
/// how its logic is split across files.
struct App {
    home: PathBuf,
    settings_path: PathBuf,
    settings: Settings,
    language: Language,
    mode: LaunchMode,
    proxy: EventLoopProxy<UserEvent>,
    /// Set once [`winit::application::ApplicationHandler::resumed`] has done its one-time
    /// startup work, so a later `resumed` call (winit may call it more than once over an app's
    /// lifetime) does not repeat it.
    started: bool,
    tray: Option<Tray>,
    state: RunState,
    /// Last-known Accessibility trust state, used both to build the tray's item 0 (DESIGN.md §6)
    /// and, by [`App::tick_idle`], to detect the missing → granted transition that triggers the
    /// "ready" confirmation (DESIGN.md §10). Kept in sync by [`App::start_menu_bar_mode`] and by
    /// every [`App::tick_idle`] poll; meaningless (and unused) in [`LaunchMode::Direct`].
    accessibility_granted: bool,
    /// Set when something unrecoverable happens (currently: the tray fails to build at startup)
    /// and [`winit::event_loop::ActiveEventLoop::exit`] was requested because of it; [`run`]
    /// checks this after the event loop returns and turns it into the `Err` main.rs reports.
    fatal: Option<Error>,
}

impl App {
    fn new(
        home: PathBuf,
        settings_path: PathBuf,
        settings: Settings,
        language: Language,
        mode: LaunchMode,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Self {
        Self {
            home,
            settings_path,
            settings,
            language,
            mode,
            proxy,
            started: false,
            tray: None,
            state: RunState::Idle,
            accessibility_granted: false,
            fatal: None,
        }
    }

    /// Marks the app for exit because of `err`, to be reported by [`run`] once the event loop
    /// returns (see [`App::fatal`]'s docs for why this can't just return a `Result` directly).
    fn fail(&mut self, event_loop: &ActiveEventLoop, err: impl Into<Error>) {
        self.fatal = Some(err.into());
        event_loop.exit();
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
/// Returns [`Error`] if the home directory cannot be located, if building or running the winit
/// event loop fails, or if something unrecoverable happened while it was running (see
/// [`App::fatal`]).
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

    let option_held = system::option_key_held();
    let mode = launch_mode(settings.show_menu_bar_icon, option_held);

    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);

    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let _ = menu_proxy.send_event(UserEvent::Menu(event));
    }));

    let mut app = App::new(home, settings_path, settings, language, mode, proxy);
    event_loop.run_app(&mut app)?;

    if let Some(err) = app.fatal.take() {
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_mode_when_icon_hidden_and_option_not_held() {
        assert_eq!(launch_mode(false, false), LaunchMode::Direct);
    }

    #[test]
    fn menu_bar_mode_when_icon_shown() {
        assert_eq!(launch_mode(true, false), LaunchMode::MenuBar);
    }

    #[test]
    fn menu_bar_mode_when_option_held_even_with_icon_hidden() {
        assert_eq!(launch_mode(false, true), LaunchMode::MenuBar);
    }

    #[test]
    fn menu_bar_mode_when_both_icon_shown_and_option_held() {
        assert_eq!(launch_mode(true, true), LaunchMode::MenuBar);
    }

    #[test]
    fn startup_alerts_shows_both_on_a_fresh_install_without_permission() {
        assert_eq!(
            startup_alerts(false, false),
            vec![StartupAlert::Welcome, StartupAlert::FirstLaunchPermission]
        );
    }

    #[test]
    fn startup_alerts_shows_only_welcome_on_a_fresh_install_already_granted() {
        // The bug this feature fixes: permission already trusted at first launch must still
        // explain the app (DESIGN.md §10), just without chaining into the permission alert.
        assert_eq!(startup_alerts(false, true), vec![StartupAlert::Welcome]);
    }

    #[test]
    fn startup_alerts_shows_only_permission_once_welcome_was_seen() {
        assert_eq!(
            startup_alerts(true, false),
            vec![StartupAlert::FirstLaunchPermission]
        );
    }

    #[test]
    fn startup_alerts_shows_nothing_once_welcome_seen_and_permission_granted() {
        assert_eq!(startup_alerts(true, true), Vec::new());
    }

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
