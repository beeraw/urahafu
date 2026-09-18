//! Every `unsafe` call in this crate lives under this module (`docs/ARCHITECTURE.md` goal 4).
//! Each submodule exposes only safe functions/types to the rest of `platform`; every `unsafe`
//! block carries a `// SAFETY:` comment explaining why it's sound.

pub mod accessibility;
pub mod action_target;
pub mod coretext;
pub mod event_tap;
pub mod locale;
pub mod menu_item;
pub mod secure_input;
pub mod window;
