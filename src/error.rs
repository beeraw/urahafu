//! Crate-wide error type.
//!
//! This stays small on purpose: most of `src/core/` cannot fail (parsing falls back to defaults
//! rather than erroring — see [`crate::core::settings`]), and most of `src/platform/` is only
//! fallible from deep inside the running app, where a failure is handled locally (an alert, a
//! `stderr` log) rather than propagated all the way back to [`crate::run`] — see `src/app/`. What
//! ends up here is only what can actually fail before or during
//! `winit::event_loop::EventLoop::run_app`, i.e. before there is any application state left to
//! recover into.

use crate::core::settings::SettingsError;

/// Top-level error type for the crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Reading or writing the settings file failed.
    #[error("settings error: {0}")]
    Settings(#[from] SettingsError),
    /// A small system query needed at startup (currently: locating the home directory) failed.
    #[cfg(target_os = "macos")]
    #[error("system error: {0}")]
    System(#[from] crate::platform::system::SystemError),
    /// Building the menu bar tray icon/menu failed at startup, in menu-bar mode. There is no
    /// sensible way to run menu-bar mode without it, so this is fatal rather than logged and
    /// skipped (unlike most other platform errors, which are handled locally in `src/app/`).
    #[cfg(target_os = "macos")]
    #[error("tray error: {0}")]
    Tray(#[from] crate::platform::tray::TrayError),
    /// Building the winit event loop, or running it, failed.
    #[cfg(target_os = "macos")]
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
}
