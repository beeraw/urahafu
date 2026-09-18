//! Pure logic core: no macOS dependency, no I/O beyond the settings file helpers in
//! [`settings`]. Everything here is compiled and unit-tested on any platform, with time supplied
//! explicitly via [`clock::Clock`] rather than read from the system.

pub mod canvas;
pub mod clock;
pub mod color;
pub mod combo;
pub mod countdown;
pub mod easing;
pub mod failsafe;
pub mod hint;
pub mod hold_ring;
pub mod i18n;
pub mod layout;
pub mod login_item;
pub mod pixel_test;
pub mod session;
pub mod settings;
