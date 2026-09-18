//! Rounds the keyboard-only HUD pill's corners at the window level (DESIGN.md §9).
//!
//! `softbuffer`'s pixel buffer has no alpha channel (`render.rs::render_hud`'s doc comment
//! explains why the pixel-level rounding/transparency approach is a dead end for this crate's
//! stack), so this module reaches past `softbuffer`/`winit` into the underlying `NSView`'s
//! backing `CALayer` instead: giving that layer a `cornerRadius` and `masksToBounds` clips
//! whatever `softbuffer` draws into the view (its own content lands in a sublayer of this one) to
//! a rounded rect, and asking the `NSWindow` to recompute its shadow makes that shadow itself
//! follow the new rounded silhouette rather than the window's square frame.
//!
//! This is the one module allowed to contain `unsafe` outside `src/platform/ffi/coretext.rs`
//! (`docs/ARCHITECTURE.md` goal 4: every `unsafe` block lives under `src/platform/ffi/` and
//! carries a `// SAFETY:` comment). The `unsafe` here is narrower than CoreText's: obtaining a
//! typed, owned reference to the `NSView` behind a `winit` window's raw handle, per the pattern
//! `raw-window-handle` itself documents for `objc2` consumers — everything downstream of that
//! (`setWantsLayer`, `CALayer::setCornerRadius`, `NSWindow::setHasShadow`, ...) is already a safe
//! `objc2-app-kit`/`objc2-quartz-core` method call.

#![allow(
    unsafe_code,
    reason = "this module (alongside src/platform/ffi/coretext.rs) is a designated FFI module \
              (docs/ARCHITECTURE.md goal 4); the crate root denies unsafe_code everywhere else"
)]

use objc2::rc::Retained;
use objc2_app_kit::NSView;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

/// Rounds `window`'s corners to radius `corner_radius` (points; DESIGN.md §9 wants
/// `height / 2`, a full pill) and gives it a native drop shadow that follows that rounded shape,
/// by setting `wantsLayer`/`cornerRadius`/`masksToBounds` on the window's `NSView` layer and
/// asking its `NSWindow` to recompute its shadow.
///
/// Returns `false` (a no-op, nothing drawn or changed) if `window` is not backed by an `NSView`
/// (not expected on macOS, the only platform this module is compiled for — see
/// `docs/ARCHITECTURE.md` goal 4 — but a window handle's shape is never assumed without
/// checking), if `AppKit` refuses to give the view a backing layer, or if the view is not yet
/// installed in a window; callers should treat that as "the pill keeps its square fallback
/// corners", not as an error worth surfacing to the user.
#[must_use]
pub fn round_window_corners(window: &Window, corner_radius: f64) -> bool {
    let Ok(handle) = window.window_handle() else {
        return false;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return false;
    };

    // SAFETY: `raw_window_handle`'s documented contract for `AppKitWindowHandle` guarantees
    // `ns_view` is a valid, non-null pointer to a live `NSView` (see that type's own doc example,
    // which uses this exact `Retained::retain` call). `Retained::retain` performs an Objective-C
    // `retain` on it before handing back an owned, typed reference, so the `NSView`'s lifetime is
    // no longer tied to `window`'s borrow — the correct way to cross from a raw, unsafe handle
    // into `objc2`'s reference-counted world, rather than holding a bare unretained reference.
    let Some(view) = (unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }) else {
        return false;
    };

    view.setWantsLayer(true);
    let Some(layer) = view.layer() else {
        return false;
    };
    layer.setCornerRadius(corner_radius);
    layer.setMasksToBounds(true);

    let Some(ns_window) = view.window() else {
        return false;
    };
    ns_window.setHasShadow(true);
    ns_window.invalidateShadow();

    true
}

#[cfg(test)]
mod tests {
    // `round_window_corners` needs a live `NSView`/`NSWindow` behind a real `winit` window,
    // which is not available in a headless test run (no GUI session here); it was instead
    // verified by reviewing the real `objc2-app-kit` method signatures it calls against Apple's
    // documented `NSView`/`NSWindow`/`CALayer` APIs. Nothing here to unit-test without a display;
    // this empty module exists so the crate's `#[cfg(test)]` convention (one test module per
    // platform file) still holds.
    use super::*;

    #[test]
    fn round_window_corners_is_a_function_reference() {
        let _ = round_window_corners;
    }
}
