//! winit windows for the cleaning overlay and the keyboard-only HUD (DESIGN.md §8-§9).
//!
//! [`Overlay`] owns one borderless, full-screen `softbuffer` window per connected monitor (full
//! screen mode) or one small always-on-top HUD window (keyboard-only mode), and knows how to
//! redraw a [`SessionView`] into them via `render.rs`. It does **not** own the input-blocking
//! event tap (`src/platform/ffi/event_tap.rs`, `src/platform/input_blocker.rs`, out of scope for
//! this module — see `docs/ARCHITECTURE.md`); [`Overlay::translate_input`] only covers the
//! narrow window of the 3-2-1 countdown, before that tap is installed, so `app.rs` can still let
//! Escape/click cancel the countdown via ordinary winit window events.
//!
//! ## Dock/menu-bar hiding and window level — what was chosen, and why
//!
//! Full-screen mode uses [`WindowExtMacOS::set_simple_fullscreen`] together with
//! [`WindowExtMacOS::set_borderless_game`] (which winit documents as hiding the Dock and menu
//! bar specifically *in* simple/borderless fullscreen — exactly this app's case), rather than
//! `Window::set_fullscreen(Some(Fullscreen::Borderless(..)))`: native fullscreen moves the
//! window into its own Space, which is unwanted for a utility that must cover every monitor at
//! once instantly and return the user to their prior Space the moment it closes. Simple
//! fullscreen has no such transition. This is the officially documented winit mechanism for this
//! exact need, not a workaround; it was not possible to interactively verify Dock/menu-bar
//! hiding across every monitor in this sandboxed environment (no logged-in GUI session with
//! multiple displays available here), so treat it as "implemented per winit's documented
//! contract", not "visually confirmed on real hardware".
//!
//! Window level uses [`WindowLevel::AlwaysOnTop`] (winit's own cross-platform enum). This maps to
//! a standard "floating" `NSWindow` level on macOS, above ordinary app windows; DESIGN.md does
//! not require besting the screen saver or lock screen specifically; there was no need to reach
//! for a raw `NSWindow` level, which would in any case belong in `src/platform/ffi/` (owned by
//! the event-tap/permission effort, not this one) rather than here.
//!
//! ## Rounded corners for the HUD pill — window-level clipping, not pixel-level alpha
//!
//! `softbuffer`'s pixel buffer has no alpha channel at all (confirmed by reading `softbuffer`
//! 0.4's own source — `Buffer` derefs to `[u32]`, and every backend's present path treats the top
//! byte as padding, not alpha): writing `0x00RRGGBB` words never carries per-pixel transparency,
//! so a rounded pill cannot be achieved by drawing softer alpha at the corners in `render.rs`.
//! It *can* be achieved one level up, though: `src/platform/ffi/window.rs::round_window_corners`
//! makes this window `with_transparent(true)` (below) and gives the underlying `NSView`'s backing
//! `CALayer` a `cornerRadius`/`masksToBounds`. `softbuffer`'s own macOS backend (`cg.rs`) adds its
//! *own* `CALayer` as a **sublayer** of the view's root layer (confirmed by reading its source:
//! `setWantsLayer`, `view.layer()`, then `root_layer.addSublayer(&layer)`) rather than replacing
//! the view's layer outright, so rounding/clipping the root layer clips that sublayer's opaque
//! content along with it: the corners outside the radius show the transparent window (the desktop
//! behind it), the interior shows `render_hud`'s ordinary opaque pixels — a real rounded pill,
//! with no per-pixel alpha involved anywhere. `render_hud` itself keeps painting a plain opaque
//! square backing (see its own doc comment); only the window/layer clips it to a pill shape.
//! `NSWindow::invalidateShadow` (also in `window.rs`) makes the window's native drop shadow follow
//! that rounded silhouette instead of the window's square frame.
//!
//! What was verified, and how: the `objc2-app-kit`/`objc2-quartz-core` method signatures used
//! (`setWantsLayer`, `NSView::layer`, `CALayer::setCornerRadius`/`setMasksToBounds`,
//! `NSWindow::setHasShadow`/`invalidateShadow`) and `softbuffer`'s sublayer-not-replace behavior
//! were both confirmed by reading the crates' source in this sandboxed environment. No logged-in
//! GUI session is available here to visually confirm the rounded pill on real hardware, so that
//! remains unverified until it is checked on a real display.

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::platform::macos::WindowExtMacOS;
use winit::window::{Window, WindowId, WindowLevel};

use crate::core::i18n::Language;
use crate::core::layout::{self, Layout, ScreenSize};
use crate::core::session::{InputEvent, KeyKind, SessionView};
use crate::platform::render::{self, Appearance, Rasterize, ScreenRole};

/// Which kind of window(s) an [`Overlay`] manages (DESIGN.md §8 vs §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    /// One borderless, full-screen window per connected monitor.
    FullScreen,
    /// One small, top-center, always-on-top HUD pill on the main screen.
    KeyboardOnly,
}

/// Errors constructing or drawing into an [`Overlay`]'s window(s).
#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    /// Creating a winit window failed.
    #[error("failed to create the overlay window: {0}")]
    Window(#[from] winit::error::OsError),
    /// Creating or resizing a `softbuffer` surface failed.
    #[error("failed to set up the overlay's pixel surface: {0}")]
    Surface(#[from] softbuffer::SoftBufferError),
    /// No monitor was available to open a full-screen window on.
    #[error("no monitor available to open the cleaning overlay on")]
    NoMonitor,
}

/// One managed window plus its `softbuffer` presentation surface.
struct Pane {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    role: ScreenRole,
}

/// Owns the cleaning overlay's window(s) (full-screen chrome, or the keyboard-only HUD pill) and
/// draws [`SessionView`]s into them.
pub struct Overlay {
    mode: OverlayMode,
    panes: Vec<Pane>,
    /// The primary (main) monitor's size, in logical points — the same screen `core::layout`
    /// positions everything against, in both modes (in `KeyboardOnly` mode the HUD window itself
    /// is much smaller than this, but its position is still computed against the full screen —
    /// see `Layout::hud_pill`).
    main_screen: ScreenSize,
}

impl Overlay {
    /// Opens the overlay's window(s) for `mode` on `event_loop`.
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError`] if no monitor is available (full-screen mode) or if creating a
    /// window or its `softbuffer` surface fails.
    pub fn open(event_loop: &ActiveEventLoop, mode: OverlayMode) -> Result<Self, OverlayError> {
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .ok_or(OverlayError::NoMonitor)?;
        let scale = monitor.scale_factor();
        let main_screen = ScreenSize::new(
            f32_from_physical(monitor.size().width, scale),
            f32_from_physical(monitor.size().height, scale),
        );

        let panes = match mode {
            OverlayMode::FullScreen => open_full_screen_panes(event_loop)?,
            OverlayMode::KeyboardOnly => vec![open_hud_pane(event_loop)?],
        };
        Ok(Self {
            mode,
            panes,
            main_screen,
        })
    }

    /// This overlay's mode.
    #[must_use]
    pub fn mode(&self) -> OverlayMode {
        self.mode
    }

    /// The primary monitor's size in logical points — what `core::layout::Layout` should be
    /// constructed with to compute the hold-to-unlock button's geometry, in either mode (see the
    /// field's own doc comment).
    #[must_use]
    pub fn main_screen_size(&self) -> ScreenSize {
        self.main_screen
    }

    /// The window-local-to-main-screen-point conversion inputs for `window_id`: its current
    /// outer position (physical pixels, in the same global space `core::layout`/the input
    /// blocker's `CGEventGetLocation` both already report points in) and its scale factor.
    /// `None` if `window_id` is not one of this overlay's windows, or if the platform can't
    /// currently report the window's position.
    #[must_use]
    pub fn window_origin_and_scale(&self, window_id: WindowId) -> Option<((f64, f64), f64)> {
        let pane = self
            .panes
            .iter()
            .find(|pane| pane.window.id() == window_id)?;
        let position = pane.window.inner_position().ok()?;
        Some((
            (f64::from(position.x), f64::from(position.y)),
            pane.window.scale_factor(),
        ))
    }

    /// Redraws every managed window from `view`.
    ///
    /// `appearance` only matters for [`OverlayMode::KeyboardOnly`] (DESIGN.md §9: the HUD follows
    /// system light/dark appearance, unlike the full-screen overlay).
    ///
    /// # Errors
    ///
    /// Returns [`OverlayError::Surface`] if presenting any window's pixel buffer fails.
    pub fn redraw(
        &mut self,
        view: &SessionView,
        language: Language,
        appearance: Appearance,
        rasterizer: &mut impl Rasterize,
    ) -> Result<(), OverlayError> {
        match self.mode {
            OverlayMode::FullScreen => self.redraw_full_screen(view, language, rasterizer),
            OverlayMode::KeyboardOnly => self.redraw_hud(view, language, appearance, rasterizer),
        }
    }

    fn redraw_full_screen(
        &mut self,
        view: &SessionView,
        language: Language,
        rasterizer: &mut impl Rasterize,
    ) -> Result<(), OverlayError> {
        for pane in &mut self.panes {
            let size = pane.window.inner_size();
            let scale = pane.window.scale_factor();
            let (Some(width), Some(height)) =
                (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
            else {
                continue;
            };
            pane.surface.resize(width, height)?;
            let mut buffer = pane.surface.buffer_mut()?;
            #[allow(
                clippy::cast_possible_truncation,
                reason = "a display's backing scale factor is always a small positive number"
            )]
            render::render_overlay(
                &mut buffer,
                size.width,
                size.height,
                scale as f32,
                pane.role,
                view,
                language,
                rasterizer,
            );
            buffer.present()?;
        }
        Ok(())
    }

    fn redraw_hud(
        &mut self,
        view: &SessionView,
        language: Language,
        appearance: Appearance,
        rasterizer: &mut impl Rasterize,
    ) -> Result<(), OverlayError> {
        let Some(pane) = self.panes.first_mut() else {
            return Ok(());
        };

        // The HUD's width depends on its (language-dependent) content, so it is resized to fit
        // before every redraw; this is cheap (measurement is cached by the rasterizer) and keeps
        // `render_hud` itself free of any layout decisions beyond drawing at a given size.
        let scale = pane.window.scale_factor();
        let content_width = render::hud_content_width_points(rasterizer, view, language);
        let logical_width = content_width.max(160.0);
        let logical_size =
            LogicalSize::new(f64::from(logical_width), f64::from(layout::HUD_PILL_HEIGHT));
        let _ = pane.window.request_inner_size(logical_size);
        recenter_hud(&pane.window);

        let size = pane.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        pane.surface.resize(width, height)?;
        let mut buffer = pane.surface.buffer_mut()?;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a display's backing scale factor is always a small positive number"
        )]
        render::render_hud(
            &mut buffer,
            size.width,
            size.height,
            scale as f32,
            appearance,
            view,
            language,
            rasterizer,
        );
        buffer.present()?;
        Ok(())
    }

    /// Closes every managed window.
    pub fn close(&mut self) {
        self.panes.clear();
    }

    /// Whether `window_id` belongs to one of this overlay's windows.
    #[must_use]
    pub fn owns(&self, window_id: WindowId) -> bool {
        self.panes.iter().any(|pane| pane.window.id() == window_id)
    }

    /// Requests a redraw of every managed window (typically called from `AboutToWait`/an
    /// animation timer, ahead of the next real [`Overlay::redraw`]).
    pub fn request_redraw(&self) {
        for pane in &self.panes {
            pane.window.request_redraw();
        }
    }

    /// Translates a raw winit window event into a [`crate::core::session::InputEvent`], for use only
    /// during [`crate::core::session::SessionPhase::Countdown`] — before the real input-blocker
    /// event tap is installed, ordinary window events are the only way to observe Escape/click
    /// to cancel. Once the tap is active, input arrives through
    /// `crate::platform::input_blocker`, not through this function; calling it after that point
    /// is harmless (it just returns events nobody forwards) but pointless.
    #[must_use]
    pub fn translate_input(event: &WindowEvent) -> Option<InputEvent> {
        match event {
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape) => Some(InputEvent::KeyDown {
                        kind: KeyKind::Escape,
                        repeat: event.repeat,
                    }),
                    _ => None,
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => {
                // The countdown-only cancel path only cares that a pointer-down happened, not
                // where: `Session::handle_input`'s `Countdown` arm cancels on any
                // `PointerDown`, ignoring its coordinates.
                Some(InputEvent::PointerDown { x: 0.0, y: 0.0 })
            }
            _ => None,
        }
    }
}

/// Opens one borderless, full-screen, menu-bar/Dock-hiding, always-on-top window per connected
/// monitor. The first monitor iterated is treated as "main" (DESIGN.md §8: chrome only on the
/// main screen) — winit does not expose which monitor macOS considers the menu-bar screen, so
/// this uses `ActiveEventLoop::primary_monitor()` when available, falling back to the first
/// monitor `available_monitors()` yields.
fn open_full_screen_panes(event_loop: &ActiveEventLoop) -> Result<Vec<Pane>, OverlayError> {
    let monitors: Vec<_> = event_loop.available_monitors().collect();
    if monitors.is_empty() {
        return Err(OverlayError::NoMonitor);
    }
    let primary_position = event_loop
        .primary_monitor()
        .map(|monitor| monitor.position());

    let mut panes = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        let is_main = primary_position == Some(monitor.position());
        let role = if is_main {
            ScreenRole::Main
        } else {
            ScreenRole::Secondary
        };

        let attrs = Window::default_attributes()
            .with_decorations(false)
            .with_transparent(false)
            .with_resizable(false)
            .with_title("Urahafu")
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(PhysicalPosition::new(
                monitor.position().x,
                monitor.position().y,
            ))
            .with_inner_size(monitor.size());

        let window = Rc::new(event_loop.create_window(attrs)?);
        // Simple fullscreen + "borderless game" is winit's documented way to also hide the Dock
        // and menu bar (see this module's doc comment for why native fullscreen is not used).
        window.set_simple_fullscreen(true);
        window.set_borderless_game(true);
        window.set_cursor_visible(false);

        let context = softbuffer::Context::new(Rc::clone(&window))?;
        let surface = softbuffer::Surface::new(&context, Rc::clone(&window))?;

        panes.push(Pane {
            window,
            surface,
            role,
        });
    }
    Ok(panes)
}

/// Opens the keyboard-only mode's small HUD window, top-center of the main screen, 12 pt below
/// the menu bar (DESIGN.md §9). Its final size depends on its (language-dependent) content, so it
/// starts at a placeholder size and [`Overlay::redraw_hud`] resizes it to fit before every frame.
fn open_hud_pane(event_loop: &ActiveEventLoop) -> Result<Pane, OverlayError> {
    let monitor = event_loop
        .primary_monitor()
        .or_else(|| event_loop.available_monitors().next())
        .ok_or(OverlayError::NoMonitor)?;

    let placeholder_width = 320.0;
    let attrs = Window::default_attributes()
        .with_decorations(false)
        // Transparent so the window-level rounded-corner clip (below) reveals the desktop, not a
        // black/white square, outside the pill's radius — see this module's doc comment.
        .with_transparent(true)
        .with_resizable(false)
        .with_title("Urahafu")
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_inner_size(LogicalSize::new(
            placeholder_width,
            f64::from(layout::HUD_PILL_HEIGHT),
        ))
        .with_active(false);

    let window = Rc::new(event_loop.create_window(attrs)?);
    position_hud(&window, &monitor, placeholder_width);
    // Best-effort: on any failure this just keeps the pill's square fallback corners (see
    // `round_window_corners`'s own doc comment for when that happens); the pill is still fully
    // usable either way, so this is not treated as an `OverlayError`. The radius is fixed
    // (DESIGN.md §9: half of the pill's constant height) and set once here, not per-redraw: only
    // the pill's *width* changes as its content changes, and a `CALayer`'s `cornerRadius` does
    // not depend on its frame size.
    let _ = crate::platform::ffi::window::round_window_corners(
        &window,
        f64::from(layout::HUD_PILL_CORNER_RADIUS),
    );

    let context = softbuffer::Context::new(Rc::clone(&window))?;
    let surface = softbuffer::Surface::new(&context, Rc::clone(&window))?;

    Ok(Pane {
        window,
        surface,
        role: ScreenRole::Main,
    })
}

/// Repositions `window` so it stays horizontally centered on its current monitor after a resize
/// (its width changes with its content; DESIGN.md §9 always centers it).
fn recenter_hud(window: &Window) {
    let Some(monitor) = window.current_monitor() else {
        return;
    };
    let size = window.inner_size();
    position_hud_pixels(window, &monitor, size.width);
}

fn position_hud(window: &Window, monitor: &winit::monitor::MonitorHandle, logical_width: f64) {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "logical HUD widths are small positive UI sizes"
    )]
    let width_px = (logical_width * monitor.scale_factor()) as u32;
    position_hud_pixels(window, monitor, width_px);
}

fn position_hud_pixels(window: &Window, monitor: &winit::monitor::MonitorHandle, width_px: u32) {
    let scale = monitor.scale_factor();
    let screen = ScreenSize::new(
        f32_from_physical(monitor.size().width, scale),
        f32_from_physical(monitor.size().height, scale),
    );
    let layout = Layout::new(screen);
    let pill = layout.hud_pill(f32_from_physical(width_px, scale));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "monitor-relative window positions are small screen-sized integers"
    )]
    let position = PhysicalPosition::new(
        monitor.position().x + (pill.x * scale as f32) as i32,
        monitor.position().y + (pill.y * scale as f32) as i32,
    );
    window.set_outer_position(position);
}

fn f32_from_physical(value: u32, scale: f64) -> f32 {
    let logical = f64::from(value) / scale;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "logical monitor sizes in points are far below f32's precision-loss range"
    )]
    let logical = logical as f32;
    logical
}

#[cfg(test)]
mod tests {
    // `Overlay::open` needs a live `ActiveEventLoop`/display connection, which is not available
    // in a headless test run (and DESIGN-fidelity/window-behavior checks belong to the
    // `examples/render_preview.rs` visual pass and manual verification instead — see the final
    // report). What *is* unit-testable here without a display is the pure input-translation
    // helper.
    use super::*;

    #[test]
    fn translate_input_is_a_pure_function_reference() {
        // `Overlay::translate_input` has no state; this test exists so the module compiles a
        // `#[cfg(test)]` block even though its real behavior can only be exercised against a
        // live `winit::event::WindowEvent`, which requires a platform event loop to construct.
        let _ = Overlay::translate_input;
    }
}
