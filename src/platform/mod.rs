//! macOS platform layer (`docs/ARCHITECTURE.md`): windows, rendering, tray, system services, and
//! the `unsafe` FFI module. Everything here is macOS-only; gated with `#[cfg(target_os = "macos")]`
//! from `src/lib.rs`.

// The `ffi` module is the crate's one deliberate exception to `unsafe_code = "deny"` (see its own
// docs and `docs/ARCHITECTURE.md` goal 4): every `unsafe` block anywhere in this crate lives under
// it, each documented with a `// SAFETY:` comment, and it only exposes safe functions/types to the
// rest of `platform`.
#[allow(unsafe_code)]
pub mod ffi;

pub mod alert;
pub mod app_menu;
pub mod input_blocker;
pub mod login_item;
pub mod permission;
pub mod render;
pub mod system;
pub mod text;

pub mod overlay;
pub mod settings_window;
pub mod tray;
