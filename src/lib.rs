//! Urahafu — a macOS screen-cleaning utility.
//!
//! This crate is split into a pure logic core, [`core`], which has no macOS dependency and is
//! unit-tested on any platform, and (once the platform layer lands) a macOS-specific
//! `platform` module plus an `app` module wiring the two together. See
//! `docs/ARCHITECTURE.md` for the full module contract and `design/DESIGN.md` for every
//! user-visible detail (timings, sizes, colors, strings).

#[cfg(target_os = "macos")]
mod app;
pub mod core;
pub mod error;
#[cfg(target_os = "macos")]
pub mod platform;

pub use error::Error;

/// Runs the application.
///
/// On macOS this builds and drives the winit `ApplicationHandler` in the crate's (private) `app`
/// module (see `docs/ARCHITECTURE.md` and `design/DESIGN.md` for the full behavior). On any other
/// target — this crate's [`core`] is deliberately platform-independent, see its module docs —
/// there is no application to run, so this is a no-op.
///
/// # Errors
///
/// Returns an [`Error`] if the home directory cannot be located, if building or running the
/// winit event loop fails, or if something unrecoverable happens while it runs.
pub fn run() -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    {
        app::run()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}
