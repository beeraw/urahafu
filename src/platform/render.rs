//! Draws a [`SessionView`] into a pixel buffer (DESIGN.md §7-§9).
//!
//! This module contains **no session/timing logic**: every opacity, position and count it draws
//! is read straight off [`SessionView`] (or derived, in pure math, from a value already on it —
//! e.g. multiplying a fade fraction by a fixed DESIGN.md opacity).
//!
//! Two render entry points, matching the two window kinds `overlay.rs` creates:
//! [`render_overlay`] (full-screen: background, countdown, hint, hold-to-unlock button, bar, ...)
//! and [`render_hud`] (the keyboard-only mode's small pill).

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "this module converts constantly between pixel counts (u32/usize), point positions \
              (f32) and signed pixel-rect coordinates (i32); every value is a UI dimension bounded \
              by real screen/window sizes, far below any of these types' precision-loss or overflow \
              ranges, so a per-cast #[allow] everywhere would just be noise"
)]

use std::rc::Rc;

use crate::core::canvas::{Canvas, PixelRect};
use crate::core::color::Rgb;
use crate::core::countdown::CountdownView;
use crate::core::i18n::{self, Language, Text};
use crate::core::layout::{self, Layout, Point as LayoutPoint, ScreenSize, TextAnchor};
use crate::core::session::{SessionView, UnlockButtonView};
use crate::platform::text::{AlphaMask, FontWeight, TextRun};

/// Countdown digit ink opacity (DESIGN.md §7), multiplied by the digit's own fade fraction.
const COUNTDOWN_DIGIT_OPACITY: f32 = 0.60;
/// Countdown label ("Cleaning is about to start") ink opacity (DESIGN.md §7).
const COUNTDOWN_LABEL_OPACITY: f32 = 0.40;
/// Countdown cancel-hint ink opacity (DESIGN.md §7).
const COUNTDOWN_CANCEL_HINT_OPACITY: f32 = 0.30;
/// Unlock hint ink opacity (DESIGN.md §8), multiplied by `SessionView::hint_opacity`.
const HINT_OPACITY: f32 = 0.45;
/// Second line ink opacity (DESIGN.md §8), multiplied by `SessionView::hint_opacity`.
const SECOND_LINE_OPACITY: f32 = 0.28;
/// Wordmark ink opacity (DESIGN.md §8), multiplied by `SessionView::wordmark_opacity`.
const WORDMARK_OPACITY: f32 = 0.10;
/// Pixel-test step indicator ink opacity (DESIGN.md §8).
const PIXEL_STEP_OPACITY: f32 = 0.25;
/// Countdown screen's hold-to-unlock explanation line opacity (DESIGN.md §8: 45%).
const COUNTDOWN_UNLOCK_EXPLANATION_OPACITY: f32 = 0.45;
/// Letter-spacing applied to the wordmark, in em units of its font size (DESIGN.md §5).
const WORDMARK_TRACKING_EM: f32 = 0.3;
/// Letter-spacing applied to the word "urahafu" inside the hint, in em units (DESIGN.md §5).
const HINT_URAHAFU_TRACKING_EM: f32 = 0.05;
/// The literal substring inside the hint text that gets Semibold weight + extra tracking
/// (DESIGN.md §5/§8). Both FR and EN hint strings contain this lowercase word verbatim.
const HINT_EMPHASIS_WORD: &str = "urahafu";

/// A source of rasterized text, abstracting over the real [`crate::platform::text::TextRasterizer`]
/// (CoreText-backed) so this module's layout/compositing logic can be unit-tested with a fake
/// implementation, with no real font rendering involved.
pub trait Rasterize {
    /// Rasterizes (or returns a cached rasterization of) `runs` at `font_size_pt` points, `scale`
    /// pixels per point.
    fn line(&mut self, runs: &[TextRun], font_size_pt: f32, scale: f32) -> Rc<AlphaMask>;
    /// Measures `runs` at `font_size_pt` points, in points (scale-independent).
    fn measure(&mut self, runs: &[TextRun], font_size_pt: f32) -> f32;
}

impl Rasterize for crate::platform::text::TextRasterizer {
    fn line(&mut self, runs: &[TextRun], font_size_pt: f32, scale: f32) -> Rc<AlphaMask> {
        Self::line(self, runs, font_size_pt, scale)
    }

    fn measure(&mut self, runs: &[TextRun], font_size_pt: f32) -> f32 {
        Self::measure(self, runs, font_size_pt)
    }
}

// --- Hold-to-unlock button -------------------------------------------------------------------

/// Unlock button fill (disc) opacity, on top of ink (DESIGN.md §8: "ink at 10% opacity").
const UNLOCK_BUTTON_FILL_OPACITY: f32 = 0.10;
/// Unlock button border opacity, on top of ink (DESIGN.md §8: "1 pt border = ink at
/// 30%").
const UNLOCK_BUTTON_BORDER_OPACITY: f32 = 0.30;
/// Unlock button border stroke width, points (DESIGN.md §8: "1 pt border").
const UNLOCK_BUTTON_BORDER_WIDTH: f32 = 1.0;
/// Gap between the button's own visual edge and the hold-progress ring drawn around it
/// (DESIGN.md §8: "radius slightly outside (radius + 4 pt)").
const UNLOCK_RING_GAP: f32 = 4.0;

/// Full-screen ✕ glyph stroke width, points (DESIGN.md §5: "2 pt stroke").
const UNLOCK_BUTTON_GLYPH_STROKE: f32 = 2.0;
/// Full-screen ✕ glyph half-span along each axis, points: DESIGN.md §5 gives the glyph as "14 pt
/// long" strokes, matching the mockups' ±7 pt diagonal offsets (the glyph fits a 14×14 pt
/// square centered on the button).
const UNLOCK_BUTTON_GLYPH_HALF_SPAN: f32 = 7.0;
/// Full-screen hold-progress ring stroke width, points (DESIGN.md §8: "ring ... 3 pt").
const UNLOCK_BUTTON_RING_WIDTH: f32 = 3.0;
/// Full-screen hold-progress ring opacity, on top of ink (DESIGN.md §8: "ink at 80%").
const UNLOCK_BUTTON_RING_OPACITY: f32 = 0.80;

/// Keyboard-only HUD ✕ glyph stroke width, points (proportioned like the full-screen glyph —
/// DESIGN.md gives no separate number for the HUD glyph's stroke width).
const HUD_UNLOCK_BUTTON_GLYPH_STROKE: f32 = 1.5;
/// Keyboard-only HUD ✕ glyph half-span along each axis, points (DESIGN.md's spec calls for
/// "10 pt" strokes; the glyph fits a 10×10 pt square, mirroring [`UNLOCK_BUTTON_GLYPH_HALF_SPAN`]
/// at the HUD button's smaller scale).
const HUD_UNLOCK_BUTTON_GLYPH_HALF_SPAN: f32 = 5.0;
/// Keyboard-only HUD hold-progress ring stroke width, points (DESIGN.md §9: "same hold
/// ring", drawn at "2 pt" for the smaller HUD button).
const HUD_UNLOCK_BUTTON_RING_WIDTH: f32 = 2.0;

/// Draws the hold-to-unlock ✕ button (shared shape for both the full-screen overlay and the
/// keyboard-only HUD): a filled disc, a thin border ring, the ✕ glyph, and — while a hold is in
/// progress — a hold-progress ring drawn slightly outside the button (DESIGN.md §8/§9).
/// `button.scale` (the completion pulse, 1.0 -> 1.1) is applied to the button's radius and glyph
/// around its own center, per DESIGN.md §8. `(center_x, center_y)` is the button's center, in
/// whatever coordinate space the caller's `scale` (points -> pixels) already assumes.
#[allow(
    clippy::too_many_arguments,
    reason = "a single shared button-shape drawer inherently needs the caller's own \
              coordinate/paint context (position, ink, opacity) plus the handful of geometry \
              knobs that differ between the full-screen and HUD button (glyph size, ring width, \
              ring color/opacity) — splitting it further would just move these back to each \
              caller"
)]
fn draw_unlock_button_shape(
    canvas: &mut Canvas<'_>,
    scale: f32,
    center_x: f32,
    center_y: f32,
    button: UnlockButtonView,
    ink: Rgb,
    extra_opacity: f32,
    glyph_half_span: f32,
    glyph_stroke_width: f32,
    ring_width: f32,
    ring_color: Rgb,
    ring_opacity: f32,
) {
    let opacity = button.opacity.clamp(0.0, 1.0) * extra_opacity.clamp(0.0, 1.0);
    if opacity <= 0.0 {
        return;
    }
    let cx = center_x * scale;
    let cy = center_y * scale;
    let radius = (button.radius * button.scale).max(0.0) * scale;

    canvas.fill_circle(cx, cy, radius, ink, UNLOCK_BUTTON_FILL_OPACITY * opacity);
    canvas.stroke_arc(
        cx,
        cy,
        radius,
        UNLOCK_BUTTON_BORDER_WIDTH * scale,
        1.0,
        ink,
        UNLOCK_BUTTON_BORDER_OPACITY * opacity,
    );

    let half = glyph_half_span * button.scale * scale;
    let glyph_width = glyph_stroke_width * scale;
    canvas.stroke_line(
        cx - half,
        cy - half,
        cx + half,
        cy + half,
        glyph_width,
        ink,
        opacity,
    );
    canvas.stroke_line(
        cx + half,
        cy - half,
        cx - half,
        cy + half,
        glyph_width,
        ink,
        opacity,
    );

    if button.hold_progress > 0.0 {
        let ring_radius = radius + UNLOCK_RING_GAP * scale;
        canvas.stroke_arc(
            cx,
            cy,
            ring_radius,
            ring_width * scale,
            button.hold_progress,
            ring_color,
            ring_opacity,
        );
    }
}

/// Draws the full-screen overlay's hold-to-unlock ✕ button at `button.center` (DESIGN.md §8).
fn draw_unlock_button(
    canvas: &mut Canvas<'_>,
    scale: f32,
    button: UnlockButtonView,
    ink: Rgb,
    overlay_opacity: f32,
) {
    draw_unlock_button_shape(
        canvas,
        scale,
        button.center.x,
        button.center.y,
        button,
        ink,
        overlay_opacity,
        UNLOCK_BUTTON_GLYPH_HALF_SPAN,
        UNLOCK_BUTTON_GLYPH_STROKE,
        UNLOCK_BUTTON_RING_WIDTH,
        ink,
        UNLOCK_BUTTON_RING_OPACITY * overlay_opacity.clamp(0.0, 1.0),
    );
}

/// Draws the keyboard-only HUD's small hold-to-unlock ✕ button at `(center_x, center_y)` — in the
/// HUD pill's own local coordinate space (points from the pill's top-left, matching every other
/// `draw_hud_*` helper in this module), not `button.center` (which is in main-screen coordinates,
/// per [`crate::core::session::Session::set_unlock_target`]'s doc comment — this render entry
/// point only ever draws into a buffer sized to the pill itself, with no notion of the pill's
/// screen position). The hold-progress ring uses the HUD's accent color rather than ink
/// (DESIGN.md §4/§9: `#22B8C8`), matching the pill's own fail-safe progress line.
fn draw_hud_unlock_button(
    canvas: &mut Canvas<'_>,
    scale: f32,
    center_x: f32,
    center_y: f32,
    button: UnlockButtonView,
    ink: Rgb,
) {
    // Unlike the full-screen button, the HUD button never shares the hint's fade curve
    // (DESIGN.md §9: "Toujours visible (pas de fondu)") — `button.opacity` still reflects that
    // fade (it comes from the same `Session::view` computation as the full-screen button), so it
    // is overridden here rather than threaded through.
    let button = UnlockButtonView {
        opacity: 1.0,
        ..button
    };
    draw_unlock_button_shape(
        canvas,
        scale,
        center_x,
        center_y,
        button,
        ink,
        1.0,
        HUD_UNLOCK_BUTTON_GLYPH_HALF_SPAN,
        HUD_UNLOCK_BUTTON_GLYPH_STROKE,
        HUD_UNLOCK_BUTTON_RING_WIDTH,
        HUD_ACCENT,
        1.0,
    );
}

/// Whether a full-screen overlay window is the main screen (draws all chrome) or a secondary
/// screen (background fill only — DESIGN.md §8: "Hint, unlock button and remaining-time bar ...
/// only appear on the main display").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenRole {
    /// The screen carrying the countdown/hint/dots/bar chrome.
    Main,
    /// A background-only screen.
    Secondary,
}

/// System light/dark appearance, for the keyboard-only HUD pill (DESIGN.md §9: the HUD follows
/// system appearance, unlike the full-screen overlay's fixed ink-vs-background contrast).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    /// Dark system appearance.
    Dark,
    /// Light system appearance.
    Light,
}

/// Draws one frame of the full-screen cleaning overlay into `buffer` (`pixel_width` ×
/// `pixel_height`, tightly packed, `0x00RRGGBB` per [`crate::core::canvas`]).
///
/// `scale` is the display's backing scale factor (1.0 or 2.0 typically); layout positions from
/// [`crate::core::layout`] are in points and are scaled here before being handed to [`Canvas`].
#[allow(
    clippy::too_many_arguments,
    reason = "a full-frame draw call inherently needs the buffer, its dimensions/scale, which \
              screen role it is, the session state, the language and the text rasterizer — \
              bundling them into a struct would not reduce the coupling, just hide it"
)]
pub fn render_overlay(
    buffer: &mut [u32],
    pixel_width: u32,
    pixel_height: u32,
    scale: f32,
    role: ScreenRole,
    view: &SessionView,
    language: Language,
    rasterizer: &mut impl Rasterize,
) {
    let width = usize_or_zero(pixel_width);
    let height = usize_or_zero(pixel_height);
    if width == 0 || height == 0 {
        return;
    }
    let mut canvas = Canvas::new(buffer, width, height, width);

    // The background is always opaque, regardless of `overlay_opacity`: the unlock fade dims
    // every *other* element to reveal the desktop only once the window itself closes (see
    // `SessionCommand::Close`), never by making the fill translucent mid-frame.
    canvas.fill(view.background);

    if role == ScreenRole::Secondary {
        return;
    }

    let overlay_opacity = view.overlay_opacity.clamp(0.0, 1.0);
    if overlay_opacity <= 0.0 {
        return;
    }

    let screen = ScreenSize::new(pixel_width as f32 / scale, pixel_height as f32 / scale);
    let layout = Layout::new(screen);

    if let Some(countdown) = view.countdown {
        draw_countdown(
            &mut canvas,
            layout,
            scale,
            view.ink,
            language,
            rasterizer,
            countdown,
            overlay_opacity,
        );
        if view.countdown_unlock_hint {
            draw_centered_text(
                &mut canvas,
                rasterizer,
                Text::CountdownUnlockExplanation.get(language),
                FontWeight::Regular,
                layout.countdown_unlock_explanation(),
                scale,
                view.ink,
                COUNTDOWN_UNLOCK_EXPLANATION_OPACITY * overlay_opacity,
            );
        }
        if let Some(button) = view.unlock_button {
            draw_unlock_button(&mut canvas, scale, button, view.ink, overlay_opacity);
        }
        return;
    }

    draw_wordmark(
        &mut canvas,
        layout,
        scale,
        view,
        language,
        rasterizer,
        overlay_opacity,
    );
    if let Some(button) = view.unlock_button {
        draw_unlock_button(&mut canvas, scale, button, view.ink, overlay_opacity);
    }
    draw_hint_block(
        &mut canvas,
        layout,
        scale,
        view,
        language,
        rasterizer,
        overlay_opacity,
    );
    draw_bar(&mut canvas, layout, scale, view, overlay_opacity);
}

#[allow(
    clippy::too_many_arguments,
    reason = "a private helper threading the same context (canvas/layout/scale/rasterizer/opacity) \
              through every draw step; grouping them into a struct would not reduce the actual \
              coupling, just hide it"
)]
fn draw_countdown(
    canvas: &mut Canvas<'_>,
    layout: Layout,
    scale: f32,
    ink: Rgb,
    language: Language,
    rasterizer: &mut impl Rasterize,
    countdown: CountdownView,
    overlay_opacity: f32,
) {
    let digit_text = countdown.digit.to_string();
    // `CountdownView::scale` (0.92 -> 1.0 ease-out on entry, DESIGN.md §7) is folded into the
    // rasterized font size rather than into a canvas-level image scale (the canvas has no
    // resize-blit primitive): a slightly smaller digit *is* a smaller rasterized glyph.
    let digit_font_size = layout::COUNTDOWN_DIGIT_FONT_SIZE * countdown.scale;
    let digit_runs = [TextRun {
        text: digit_text,
        weight: FontWeight::Light,
        tracking_em: 0.0,
        tabular_numbers: true,
    }];
    let anchor = layout.countdown_digit();
    // Rasterized (rather than handed to `draw_centered_line`) so its real metrics are available
    // below to position the "Cleaning is about to start" label off the digit's *actual* cap top — a fixed point
    // offset from the digit's baseline (this module's previous approach) does not track the
    // digit's real rendered height, which is what let the label run into the top of the glyph
    // for a mockup like `04-countdown` (label baseline barely above the digit's cap, DESIGN.md
    // §7 wants a comfortable gap).
    let digit_mask = rasterizer.line(&digit_runs, digit_font_size, scale);
    draw_mask_centered(
        canvas,
        &digit_mask,
        scale,
        anchor.position,
        ink,
        COUNTDOWN_DIGIT_OPACITY * countdown.opacity * overlay_opacity,
    );

    let digit_cap_top = anchor.position.y - digit_mask.baseline / scale;
    let label_anchor = layout.countdown_label_above(digit_cap_top);
    draw_centered_text(
        canvas,
        rasterizer,
        Text::CountdownStarting.get(language),
        FontWeight::Regular,
        label_anchor,
        scale,
        ink,
        COUNTDOWN_LABEL_OPACITY * countdown.opacity * overlay_opacity,
    );

    let cancel_anchor = layout.countdown_cancel_hint();
    draw_centered_text(
        canvas,
        rasterizer,
        Text::CountdownCancelHint.get(language),
        FontWeight::Regular,
        cancel_anchor,
        scale,
        ink,
        COUNTDOWN_CANCEL_HINT_OPACITY * overlay_opacity,
    );
}

fn draw_wordmark(
    canvas: &mut Canvas<'_>,
    layout: Layout,
    scale: f32,
    view: &SessionView,
    language: Language,
    rasterizer: &mut impl Rasterize,
    overlay_opacity: f32,
) {
    let opacity = view.wordmark_opacity.clamp(0.0, 1.0) * WORDMARK_OPACITY * overlay_opacity;
    if opacity <= 0.0 {
        return;
    }
    let runs = [TextRun {
        text: Text::OverlayWordmark.get(language).to_owned(),
        weight: FontWeight::Medium,
        tracking_em: WORDMARK_TRACKING_EM,
        tabular_numbers: false,
    }];
    let anchor = layout.wordmark();
    draw_centered_line(
        canvas,
        rasterizer,
        &runs,
        anchor.font_size,
        scale,
        anchor.position,
        view.ink,
        opacity,
    );
}

#[allow(
    clippy::too_many_arguments,
    reason = "see draw_countdown: a private helper threading shared draw context"
)]
fn draw_hint_block(
    canvas: &mut Canvas<'_>,
    layout: Layout,
    scale: f32,
    view: &SessionView,
    language: Language,
    rasterizer: &mut impl Rasterize,
    overlay_opacity: f32,
) {
    let hint_fraction = view.hint_opacity.clamp(0.0, 1.0);
    if hint_fraction <= 0.0 {
        return;
    }

    let hint_anchor = layout.hint();
    let hint_runs = split_hint_runs(Text::OverlayHintHoldToUnlock.get(language));
    draw_centered_line(
        canvas,
        rasterizer,
        &hint_runs,
        hint_anchor.font_size,
        scale,
        hint_anchor.position,
        view.ink,
        HINT_OPACITY * hint_fraction * overlay_opacity,
    );

    let second_line_text = i18n::hint_secondary(language, view.remaining_secs);
    let second_line_anchor = layout.second_line();
    draw_centered_text(
        canvas,
        rasterizer,
        &second_line_text,
        FontWeight::Regular,
        second_line_anchor,
        scale,
        view.ink,
        SECOND_LINE_OPACITY * hint_fraction * overlay_opacity,
    );

    if let Some((step, total)) = view.pixel_step {
        let second_line_width = rasterizer.measure(
            &[TextRun::plain(&second_line_text, FontWeight::Regular)],
            12.0,
        );
        let step_text = format!("{step}/{total}");
        let step_anchor = layout.pixel_test_step(second_line_width);
        draw_centered_text(
            canvas,
            rasterizer,
            &step_text,
            FontWeight::Regular,
            step_anchor,
            scale,
            view.ink,
            PIXEL_STEP_OPACITY * overlay_opacity,
        );
    }
}

fn draw_bar(
    canvas: &mut Canvas<'_>,
    layout: Layout,
    scale: f32,
    view: &SessionView,
    overlay_opacity: f32,
) {
    let Some(bar) = view.bar else {
        return;
    };
    let rect = layout.bar();
    let fraction = bar.fraction.clamp(0.0, 1.0);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "bar width is bounded by the screen's pixel width, far below i32::MAX"
    )]
    let pixel_rect = PixelRect::new(
        (rect.x * scale).round() as i32,
        (rect.y * scale).round() as i32,
        (rect.width * scale * fraction).round() as i32,
        (rect.height * scale).round().max(1.0) as i32,
    );
    canvas.fill_rect(pixel_rect, view.ink, bar.opacity * overlay_opacity);
}

/// Splits `text` into runs so the literal word [`HINT_EMPHASIS_WORD`] ("urahafu") gets Semibold
/// weight and extra tracking, with the rest of the text Regular (DESIGN.md §5/§8). Falls back to
/// one plain Regular run if the word is not found (defensive: both shipped translations contain
/// it verbatim).
fn split_hint_runs(text: &str) -> Vec<TextRun> {
    let Some(start) = text.find(HINT_EMPHASIS_WORD) else {
        return vec![TextRun::plain(text, FontWeight::Regular)];
    };
    let end = start + HINT_EMPHASIS_WORD.len();
    let mut runs = Vec::with_capacity(3);
    if !text[..start].is_empty() {
        runs.push(TextRun::plain(&text[..start], FontWeight::Regular));
    }
    runs.push(TextRun {
        text: text[start..end].to_owned(),
        weight: FontWeight::Semibold,
        tracking_em: HINT_URAHAFU_TRACKING_EM,
        tabular_numbers: false,
    });
    if !text[end..].is_empty() {
        runs.push(TextRun::plain(&text[end..], FontWeight::Regular));
    }
    runs
}

/// Rasterizes `text` as a single Regular run and draws it centered on `anchor`.
#[allow(
    clippy::too_many_arguments,
    reason = "see draw_countdown: a private helper threading shared draw context"
)]
fn draw_centered_text(
    canvas: &mut Canvas<'_>,
    rasterizer: &mut impl Rasterize,
    text: &str,
    weight: FontWeight,
    anchor: TextAnchor,
    scale: f32,
    color: Rgb,
    opacity: f32,
) {
    let runs = [TextRun::plain(text, weight)];
    draw_centered_line(
        canvas,
        rasterizer,
        &runs,
        anchor.font_size,
        scale,
        anchor.position,
        color,
        opacity,
    );
}

/// Rasterizes `runs` as one line and blits it horizontally centered on `position.x`, with its
/// baseline at `position.y` (both in points, per [`TextAnchor`]'s convention).
#[allow(
    clippy::too_many_arguments,
    reason = "see draw_countdown: a private helper threading shared draw context"
)]
fn draw_centered_line(
    canvas: &mut Canvas<'_>,
    rasterizer: &mut impl Rasterize,
    runs: &[TextRun],
    font_size_pt: f32,
    scale: f32,
    position: LayoutPoint,
    color: Rgb,
    opacity: f32,
) {
    if opacity <= 0.0 || runs.is_empty() {
        return;
    }
    let mask = rasterizer.line(runs, font_size_pt, scale);
    draw_mask_centered(canvas, &mask, scale, position, color, opacity);
}

/// The pixel-space rect an already-rasterized `mask` would paint into, horizontally centered on
/// `position.x` with its baseline at `position.y` (both in points) — shared by
/// [`draw_mask_centered`] (the actual paint) and by tests that need to check two drawn lines for
/// overlap without duplicating this arithmetic.
fn mask_pixel_rect(mask: &AlphaMask, scale: f32, position: LayoutPoint) -> PixelRect {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "mask/baseline pixel dimensions are far below i32::MAX for any real line of UI text"
    )]
    let dest_x = (position.x * scale - mask.width as f32 / 2.0).round() as i32;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "mask/baseline pixel dimensions are far below i32::MAX for any real line of UI text"
    )]
    let dest_y = (position.y * scale - mask.baseline).round() as i32;
    #[allow(
        clippy::cast_possible_wrap,
        reason = "mask pixel dimensions are far below i32::MAX for any real line of UI text"
    )]
    PixelRect::new(dest_x, dest_y, mask.width as i32, mask.height as i32)
}

/// Blits an already-rasterized `mask`, horizontally centered on `position.x` with its baseline at
/// `position.y` (both in points). See [`draw_centered_line`] for the rasterize-then-draw
/// convenience most callers want; this lower-level entry point exists for callers (like
/// [`draw_countdown`]) that need the mask's own metrics (e.g. its measured cap height) before
/// deciding where something *else* goes.
fn draw_mask_centered(
    canvas: &mut Canvas<'_>,
    mask: &AlphaMask,
    scale: f32,
    position: LayoutPoint,
    color: Rgb,
    opacity: f32,
) {
    if opacity <= 0.0 {
        return;
    }
    let rect = mask_pixel_rect(mask, scale, position);
    canvas.blit_alpha_mask(
        &mask.data,
        mask.width as usize,
        mask.height as usize,
        rect.x,
        rect.y,
        color,
        opacity,
    );
}

fn usize_or_zero(value: u32) -> usize {
    usize::try_from(value).unwrap_or(0)
}

// --- Keyboard-only HUD ---------------------------------------------------------------------

/// HUD content left/right padding (points). DESIGN.md §9 fixes the pill's height (44 pt) and
/// corner radius but not its internal padding; this value is this module's own reasonable
/// choice, not a DESIGN.md number.
const HUD_PADDING_X: f32 = 14.0;
/// Gap between adjacent HUD content items (points); same caveat as [`HUD_PADDING_X`].
const HUD_ITEM_GAP: f32 = 10.0;
/// Width of the HUD's vertical separators (points).
const HUD_SEPARATOR_WIDTH: f32 = 1.0;
/// Height of the HUD's vertical separators (points).
const HUD_SEPARATOR_HEIGHT: f32 = 20.0;
/// Size of the keyboard-with-lock glyph (DESIGN.md §9: 18 pt).
const HUD_GLYPH_SIZE: f32 = 18.0;
/// HUD accent color (DESIGN.md §4: `#22B8C8`), used for the glyph and the bottom progress line.
const HUD_ACCENT: Rgb = Rgb::new(0x22, 0xB8, 0xC8);
/// HUD primary label opacity (fully opaque ink, DESIGN.md §9).
const HUD_PRIMARY_OPACITY: f32 = 1.0;
/// HUD secondary text opacity (DESIGN.md §9: 65%).
const HUD_SECONDARY_OPACITY: f32 = 0.65;
/// HUD primary/secondary text font size (DESIGN.md §9).
const HUD_TEXT_FONT_SIZE: f32 = 13.0;
/// HUD bottom accent progress line height (DESIGN.md §9: 2 pt).
const HUD_PROGRESS_LINE_HEIGHT: f32 = 2.0;

/// Computes the HUD pill's content width in points, for sizing its window before creation
/// (`overlay.rs` measures this once per language/state change, not every frame).
///
/// Mirrors [`render_hud`]'s own item order exactly (LTR and RTL alike) — see that function's doc
/// comment for what the order is and why; a mismatch here would size the window wrong for
/// whichever branch drifted.
pub fn hud_content_width_points(
    rasterizer: &mut impl Rasterize,
    view: &SessionView,
    language: Language,
) -> f32 {
    if let Some(countdown) = view.countdown {
        let text = i18n::countdown_locking_in(language, u32::from(countdown.digit));
        let text_width = rasterizer.measure(
            &[TextRun::plain(&text, FontWeight::Semibold)],
            HUD_TEXT_FONT_SIZE,
        );
        // Countdown state only ever shows glyph + text; mirroring their order for RTL (see
        // `render_hud`) doesn't change the total.
        return HUD_PADDING_X * 2.0 + HUD_GLYPH_SIZE + HUD_ITEM_GAP + text_width;
    }

    let primary_width = rasterizer.measure(
        &[TextRun::plain(
            Text::HudKeyboardLocked.get(language),
            FontWeight::Semibold,
        )],
        HUD_TEXT_FONT_SIZE,
    );
    let secondary_width = rasterizer.measure(
        &[TextRun::plain(
            Text::HudHoldToUnlock.get(language),
            FontWeight::Regular,
        )],
        HUD_TEXT_FONT_SIZE,
    );
    let time_text = i18n::hud_time_remaining(language, view.remaining_secs);
    let time_width = rasterizer.measure(
        &[TextRun::plain(&time_text, FontWeight::Regular)],
        HUD_TEXT_FONT_SIZE,
    );
    // The ✕ button itself (`draw_hud_unlock_button`); its ring/glyph draw within this diameter,
    // so no extra width is needed for them.
    let button_width = layout::HUD_UNLOCK_BUTTON_DIAMETER;

    let content = if language.is_rtl() {
        // button, label, time, separator, title, glyph.
        button_width
            + HUD_ITEM_GAP
            + secondary_width
            + HUD_ITEM_GAP
            + time_width
            + HUD_ITEM_GAP
            + HUD_SEPARATOR_WIDTH
            + HUD_ITEM_GAP
            + primary_width
            + HUD_ITEM_GAP
            + HUD_GLYPH_SIZE
    } else {
        // glyph, title, separator, label, button, separator, time.
        HUD_GLYPH_SIZE
            + HUD_ITEM_GAP
            + primary_width
            + HUD_ITEM_GAP
            + HUD_SEPARATOR_WIDTH
            + HUD_ITEM_GAP
            + secondary_width
            + HUD_ITEM_GAP
            + button_width
            + HUD_ITEM_GAP
            + HUD_SEPARATOR_WIDTH
            + HUD_ITEM_GAP
            + time_width
    };

    HUD_PADDING_X * 2.0 + content
}

/// Draws one frame of the keyboard-only HUD pill into `buffer` (sized exactly to the pill:
/// `pixel_width` × `pixel_height` = `layout::HUD_PILL_HEIGHT` tall, at `scale`).
///
/// This function always paints an **opaque** backing across the whole buffer (the pill's
/// DESIGN.md §9 background color, with its `rgba()` alpha dropped) and square corners:
/// `softbuffer`'s pixel format has no alpha channel (`0x00RRGGBB`, matching
/// [`crate::core::canvas`]'s documented convention), so there is no per-pixel transparency this
/// function could paint at the corners even if it tried. The rounded pill shape DESIGN.md §9
/// asks for still happens, just not here: `platform::overlay::open_hud_pane` makes the window
/// itself `with_transparent(true)` and `platform::ffi::window::round_window_corners` clips the
/// underlying `NSView`'s layer to a rounded rect, so the square corners this function paints get
/// clipped away by the window/layer, revealing the (transparent) window and the desktop behind
/// it — see `overlay.rs`'s module doc comment for the full mechanism and what was verified.
///
/// ## Item order and RTL mirroring (DESIGN.md §1's RTL note)
///
/// Left-to-right languages draw, left to right: glyph, title ("Keyboard locked"), separator,
/// label ("Hold the button 2s"), ✕ button, separator, time remaining. For [`Language::is_rtl`]
/// languages (Arabic), the whole order is mirrored to: ✕ button, label, time remaining,
/// separator, title, glyph — the button moves to the left end so it stays reachable first, and
/// the status cluster (title + glyph) moves to the right end, rather than a naive left-right
/// reversal of the LTR list. The bottom accent progress line mirrors too (fills from the right
/// in RTL instead of the left). [`hud_content_width_points`] must be kept in lock-step with
/// whichever order is drawn here, and [`layout::hud_unlock_button`]'s `rtl` parameter must return
/// the same mirrored button position so hit-testing (`crate::app::clean::update_unlock_target`)
/// matches what is actually drawn. The hold-progress ring itself is deliberately **not**
/// mirrored — it always fills clockwise from 12 o'clock, in both directions (see
/// `draw_unlock_button_shape`).
#[allow(
    clippy::too_many_arguments,
    reason = "see render_overlay: a full-frame draw call inherently needs all of these"
)]
#[allow(
    clippy::too_many_lines,
    reason = "one linear layout pass over the HUD's fixed DESIGN.md §9 content list (glyph, \
              label, separator, secondary text, button, separator, time, progress line), drawn \
              twice — once for LTR order, once mirrored for RTL (see the doc comment above); \
              splitting either pass into more helpers would only relocate these lines, not \
              reduce the coupling between them (each item's x position depends on the previous \
              item's measured width)"
)]
pub fn render_hud(
    buffer: &mut [u32],
    pixel_width: u32,
    pixel_height: u32,
    scale: f32,
    appearance: Appearance,
    view: &SessionView,
    language: Language,
    rasterizer: &mut impl Rasterize,
) {
    let width = usize_or_zero(pixel_width);
    let height = usize_or_zero(pixel_height);
    if width == 0 || height == 0 {
        return;
    }
    let mut canvas = Canvas::new(buffer, width, height, width);

    let (background, border, ink) = hud_colors(appearance);
    canvas.fill(background);
    // A thin 1 px border, approximating DESIGN.md §9's border (its own alpha is dropped for the
    // same reason as the background — see the doc comment above).
    canvas.fill_rect(PixelRect::new(0, 0, width as i32, 1), border, 1.0);
    canvas.fill_rect(
        PixelRect::new(0, height as i32 - 1, width as i32, 1),
        border,
        1.0,
    );
    canvas.fill_rect(PixelRect::new(0, 0, 1, height as i32), border, 1.0);
    canvas.fill_rect(
        PixelRect::new(width as i32 - 1, 0, 1, height as i32),
        border,
        1.0,
    );

    let center_y = pixel_height as f32 / scale / 2.0;
    let width_pt = pixel_width as f32 / scale;
    let rtl = language.is_rtl();

    if let Some(countdown) = view.countdown {
        let text = i18n::countdown_locking_in(language, u32::from(countdown.digit));
        if rtl {
            // Mirrored: text leads (left), glyph trails (right) — DESIGN.md §1's RTL note; the
            // hold ring (drawn elsewhere, on the full-screen/HUD unlock buttons) deliberately
            // keeps its clockwise direction regardless of language, see `draw_unlock_button_shape`.
            draw_hud_text(
                &mut canvas,
                rasterizer,
                &text,
                FontWeight::Semibold,
                HUD_PADDING_X,
                center_y,
                scale,
                ink,
                HUD_PRIMARY_OPACITY,
            );
            draw_hud_glyph(
                &mut canvas,
                scale,
                width_pt - HUD_PADDING_X - HUD_GLYPH_SIZE,
                center_y,
                background,
            );
        } else {
            draw_hud_glyph(&mut canvas, scale, HUD_PADDING_X, center_y, background);
            draw_hud_text(
                &mut canvas,
                rasterizer,
                &text,
                FontWeight::Semibold,
                HUD_PADDING_X + HUD_GLYPH_SIZE + HUD_ITEM_GAP,
                center_y,
                scale,
                ink,
                HUD_PRIMARY_OPACITY,
            );
        }
        return;
    }

    let primary = Text::HudKeyboardLocked.get(language);
    let secondary = Text::HudHoldToUnlock.get(language);
    let time_text = i18n::hud_time_remaining(language, view.remaining_secs);

    if rtl {
        // Mirrored item order (DESIGN.md §1's RTL note): button, label, time, separator, title,
        // glyph — grouping the interactive control with its instructions and countdown on the
        // left, status (title + glyph) on the right, rather than a naive left-right reversal of
        // the LTR list. `hud_content_width_points` must be kept in lock-step with this order.
        let mut x = HUD_PADDING_X;

        if let Some(button) = view.unlock_button {
            draw_hud_unlock_button(
                &mut canvas,
                scale,
                x + layout::HUD_UNLOCK_BUTTON_RADIUS,
                center_y,
                button,
                ink,
            );
        }
        x += layout::HUD_UNLOCK_BUTTON_DIAMETER + HUD_ITEM_GAP;

        let secondary_width = draw_hud_text_measured(
            &mut canvas,
            rasterizer,
            secondary,
            FontWeight::Regular,
            x,
            center_y,
            scale,
            ink,
            HUD_SECONDARY_OPACITY,
        );
        x += secondary_width + HUD_ITEM_GAP;

        let time_width = draw_hud_text_measured(
            &mut canvas,
            rasterizer,
            &time_text,
            FontWeight::Regular,
            x,
            center_y,
            scale,
            ink,
            HUD_SECONDARY_OPACITY,
        );
        x += time_width + HUD_ITEM_GAP;

        draw_hud_separator(&mut canvas, scale, x, center_y, border);
        x += HUD_SEPARATOR_WIDTH + HUD_ITEM_GAP;

        let primary_width = draw_hud_text_measured(
            &mut canvas,
            rasterizer,
            primary,
            FontWeight::Semibold,
            x,
            center_y,
            scale,
            ink,
            HUD_PRIMARY_OPACITY,
        );
        x += primary_width + HUD_ITEM_GAP;

        draw_hud_glyph(&mut canvas, scale, x, center_y, background);
    } else {
        let mut x = HUD_PADDING_X;
        draw_hud_glyph(&mut canvas, scale, x, center_y, background);
        x += HUD_GLYPH_SIZE + HUD_ITEM_GAP;

        let primary_width = draw_hud_text_measured(
            &mut canvas,
            rasterizer,
            primary,
            FontWeight::Semibold,
            x,
            center_y,
            scale,
            ink,
            HUD_PRIMARY_OPACITY,
        );
        x += primary_width + HUD_ITEM_GAP;
        draw_hud_separator(&mut canvas, scale, x, center_y, border);
        x += HUD_SEPARATOR_WIDTH + HUD_ITEM_GAP;

        let secondary_width = draw_hud_text_measured(
            &mut canvas,
            rasterizer,
            secondary,
            FontWeight::Regular,
            x,
            center_y,
            scale,
            ink,
            HUD_SECONDARY_OPACITY,
        );
        x += secondary_width + HUD_ITEM_GAP;

        if let Some(button) = view.unlock_button {
            draw_hud_unlock_button(
                &mut canvas,
                scale,
                x + layout::HUD_UNLOCK_BUTTON_RADIUS,
                center_y,
                button,
                ink,
            );
        }
        x += layout::HUD_UNLOCK_BUTTON_DIAMETER + HUD_ITEM_GAP;
        draw_hud_separator(&mut canvas, scale, x, center_y, border);
        x += HUD_SEPARATOR_WIDTH + HUD_ITEM_GAP;

        draw_hud_text(
            &mut canvas,
            rasterizer,
            &time_text,
            FontWeight::Regular,
            x,
            center_y,
            scale,
            ink,
            HUD_SECONDARY_OPACITY,
        );
    }

    if let Some(bar) = view.bar {
        let fraction = bar.fraction.clamp(0.0, 1.0);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the HUD pill's pixel width is far below i32::MAX"
        )]
        let filled_width = (pixel_width as f32 * fraction).round() as i32;
        // LTR fills from the left (dwindling toward the right as time passes); RTL mirrors that
        // to fill from the right (DESIGN.md §1's RTL note), dwindling toward the left.
        let bar_x = if rtl {
            pixel_width as i32 - filled_width
        } else {
            0
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the HUD pill's pixel width is far below i32::MAX"
        )]
        let rect = PixelRect::new(
            bar_x,
            (pixel_height as f32 - HUD_PROGRESS_LINE_HEIGHT * scale).round() as i32,
            filled_width,
            (HUD_PROGRESS_LINE_HEIGHT * scale).round().max(1.0) as i32,
        );
        canvas.fill_rect(rect, HUD_ACCENT, 1.0);
    }
}

/// `(background, border, ink)` for the HUD pill in `appearance` (DESIGN.md §9). The design's
/// `rgba()` values are flattened to opaque colors, since this canvas format has no alpha
/// channel — see [`render_hud`]'s doc comment for why.
fn hud_colors(appearance: Appearance) -> (Rgb, Rgb, Rgb) {
    match appearance {
        Appearance::Dark => (Rgb::new(28, 28, 30), Rgb::new(60, 60, 62), Rgb::WHITE),
        Appearance::Light => (Rgb::new(246, 246, 248), Rgb::new(214, 214, 217), Rgb::BLACK),
    }
}

/// Draws the hand-drawn keyboard-with-lock glyph (DESIGN.md §1: no SF Symbols, no embedded
/// image; DESIGN.md §9: 18 pt, accent color) at `left` (points, left edge), vertically centered
/// on `center_y` (points): a rounded-rect keyboard outline with two rows of small key squares and
/// a space bar, plus a padlock overlapping its top-right corner (mockup `09-keyboard-only-dark`/
/// `-light`) — all drawn from [`Canvas`] primitives, never a bitmap.
///
/// `pill_background` knocks out a small disc behind the padlock (also matching the mockups) so it
/// reads as sitting in front of the keyboard glyph rather than merging into its outline.
#[allow(
    clippy::similar_names,
    reason = "`lock_cx`/`lock_cy` are deliberately paired x/y coordinates of the same point (the \
              lock badge's center); giving them dissimilar names would make the geometry harder \
              to follow, not easier"
)]
fn draw_hud_glyph(
    canvas: &mut Canvas<'_>,
    scale: f32,
    left: f32,
    center_y: f32,
    pill_background: Rgb,
) {
    let size = HUD_GLYPH_SIZE;
    let top = center_y - size / 2.0;

    // Keyboard body: a stroked (outline-only) rounded rect, per DESIGN.md's "clavier maison"
    // description — not a filled shape, which used to read as a plain teal blob rather than a
    // keyboard.
    let kb_top = top + size * 0.16;
    let kb_height = size * 0.52;
    let kb_rect = pixel_rect(scale, left, kb_top, size, kb_height);
    canvas.stroke_rounded_rect(kb_rect, 2.2 * scale, 1.2 * scale, HUD_ACCENT, 1.0);

    // Two rows of small key squares, plus a wider space-bar rect along the bottom — enough detail
    // to read as a keyboard at 18 pt without trying to depict individual real keys.
    let key_size = size * 0.09;
    let key_gap = size * 0.145;
    let first_key_x = left + size * 0.14;
    let key_corner = 0.6 * scale;
    for row_y in [kb_top + kb_height * 0.22, kb_top + kb_height * 0.52] {
        for i in 0..5_u32 {
            #[allow(
                clippy::cast_precision_loss,
                reason = "row has exactly 5 keys, far below f32's precision-loss range"
            )]
            let x = first_key_x + i as f32 * key_gap;
            let key_rect = pixel_rect(scale, x, row_y, key_size, key_size);
            canvas.fill_rounded_rect(key_rect, key_corner, HUD_ACCENT, 1.0);
        }
    }
    let space_rect = pixel_rect(
        scale,
        first_key_x,
        kb_top + kb_height * 0.76,
        size * 0.55,
        key_size * 0.7,
    );
    canvas.fill_rounded_rect(space_rect, key_corner, HUD_ACCENT, 1.0);

    // Padlock, overlapping the keyboard's top-right corner (mockup `09-keyboard-only-dark`/
    // `-light`): a knockout disc in the pill's own background color first (so the lock reads as
    // sitting in front of the keyboard outline rather than merging into it), then the shackle
    // (an arc, approximated by a stroked rounded rect tall enough that its straight lower legs
    // get covered by the body drawn on top of it, leaving only the curved top peeking above the
    // body — the usual trick for a padlock glyph without a dedicated arc primitive), then the
    // solid rounded-rect body on top.
    let lock_cx = left + size * 0.88;
    let lock_cy = top + size * 0.18;
    let knockout_radius = size * 0.34;
    canvas.fill_circle(
        lock_cx * scale,
        lock_cy * scale,
        knockout_radius * scale,
        pill_background,
        1.0,
    );

    let shackle_width = size * 0.22;
    let shackle_height = size * 0.26;
    let shackle_rect = pixel_rect(
        scale,
        lock_cx - shackle_width / 2.0,
        lock_cy - shackle_height * 0.62,
        shackle_width,
        shackle_height,
    );
    canvas.stroke_rounded_rect(
        shackle_rect,
        shackle_width / 2.0 * scale,
        1.3 * scale,
        HUD_ACCENT,
        1.0,
    );

    let body_width = size * 0.34;
    let body_height = size * 0.26;
    let body_rect = pixel_rect(
        scale,
        lock_cx - body_width / 2.0,
        lock_cy + shackle_height * 0.02,
        body_width,
        body_height,
    );
    canvas.fill_rounded_rect(body_rect, 1.4 * scale, HUD_ACCENT, 1.0);
}

/// Converts a `(left, top, width, height)` box in points to a [`PixelRect`] at `scale`, rounding
/// each edge independently (rather than rounding width/height directly) so adjacent glyph pieces
/// laid out edge-to-edge in points don't accumulate a visible pixel-rounding gap between them.
fn pixel_rect(scale: f32, left: f32, top: f32, width: f32, height: f32) -> PixelRect {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "HUD glyph pixel positions are far below i32::MAX"
    )]
    let (x0, y0, x1, y1) = (
        (left * scale).round() as i32,
        (top * scale).round() as i32,
        ((left + width) * scale).round() as i32,
        ((top + height) * scale).round() as i32,
    );
    PixelRect::new(x0, y0, x1 - x0, y1 - y0)
}

fn draw_hud_separator(canvas: &mut Canvas<'_>, scale: f32, x: f32, center_y: f32, color: Rgb) {
    let top = center_y - HUD_SEPARATOR_HEIGHT / 2.0;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "HUD pixel positions are far below i32::MAX"
    )]
    let rect = PixelRect::new(
        (x * scale).round() as i32,
        (top * scale).round() as i32,
        (HUD_SEPARATOR_WIDTH * scale).round().max(1.0) as i32,
        (HUD_SEPARATOR_HEIGHT * scale).round() as i32,
    );
    canvas.fill_rect(rect, color, 1.0);
}

/// Draws `text` left-aligned starting at `left` (points), vertically centered on `center_y`
/// (points), returning its measured width in points.
#[allow(
    clippy::too_many_arguments,
    reason = "see draw_countdown: a private helper threading shared draw context"
)]
fn draw_hud_text_measured(
    canvas: &mut Canvas<'_>,
    rasterizer: &mut impl Rasterize,
    text: &str,
    weight: FontWeight,
    left: f32,
    center_y: f32,
    scale: f32,
    color: Rgb,
    opacity: f32,
) -> f32 {
    let runs = [TextRun::plain(text, weight)];
    let width = rasterizer.measure(&runs, HUD_TEXT_FONT_SIZE);
    let mask = rasterizer.line(&runs, HUD_TEXT_FONT_SIZE, scale);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "HUD pixel positions are far below i32::MAX"
    )]
    let dest_x = (left * scale).round() as i32;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "HUD pixel positions are far below i32::MAX"
    )]
    let dest_y = (center_y * scale - mask.baseline + mask.height as f32 / 2.0).round() as i32;
    canvas.blit_alpha_mask(
        &mask.data,
        mask.width as usize,
        mask.height as usize,
        dest_x,
        dest_y,
        color,
        opacity,
    );
    width
}

#[allow(
    clippy::too_many_arguments,
    reason = "see draw_countdown: a private helper threading shared draw context"
)]
fn draw_hud_text(
    canvas: &mut Canvas<'_>,
    rasterizer: &mut impl Rasterize,
    text: &str,
    weight: FontWeight,
    left: f32,
    center_y: f32,
    scale: f32,
    color: Rgb,
    opacity: f32,
) {
    let _ = draw_hud_text_measured(
        canvas, rasterizer, text, weight, left, center_y, scale, color, opacity,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::color::CleaningColor;
    use crate::core::session::BarView;
    use std::collections::HashMap;

    /// A fake [`Rasterize`] that produces deterministic, trivially-shaped masks (a solid
    /// rectangle per character) with no real font involved, so `render.rs`'s own compositing
    /// logic (positions, opacities, clipping) can be tested without CoreText.
    #[derive(Default)]
    struct FakeRasterizer {
        calls: HashMap<String, u32>,
        /// Every individual run seen across all `line()` calls, as `(text, weight)` — unlike
        /// `calls` (keyed by the whole line's concatenated text), this lets tests assert on one
        /// specific run inside a mixed-weight line (e.g. the "urahafu" run inside the hint).
        runs_seen: Vec<(String, FontWeight)>,
    }

    impl Rasterize for FakeRasterizer {
        fn line(&mut self, runs: &[TextRun], font_size_pt: f32, scale: f32) -> Rc<AlphaMask> {
            let text: String = runs.iter().map(|r| r.text.as_str()).collect();
            *self.calls.entry(text.clone()).or_insert(0) += 1;
            self.runs_seen
                .extend(runs.iter().map(|r| (r.text.clone(), r.weight)));
            let char_w = (font_size_pt * scale * 0.6).max(1.0);
            let height = (font_size_pt * scale).max(1.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let width = (char_w * text.chars().count().max(1) as f32) as u32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let height = height as u32;
            Rc::new(AlphaMask {
                width: width.max(1),
                height: height.max(1),
                baseline: height as f32 * 0.8,
                data: vec![255u8; (width.max(1) * height.max(1)) as usize],
                width_points: width as f32 / scale,
            })
        }

        fn measure(&mut self, runs: &[TextRun], font_size_pt: f32) -> f32 {
            let text: String = runs.iter().map(|r| r.text.as_str()).collect();
            font_size_pt * 0.6 * text.chars().count().max(1) as f32
        }
    }

    fn base_view() -> SessionView {
        SessionView {
            background: Rgb::BLACK,
            ink: Rgb::WHITE,
            keyboard_only: false,
            countdown: None,
            hint_opacity: 1.0,
            wordmark_opacity: 0.0,
            unlock_button: None,
            countdown_unlock_hint: false,
            bar: Some(BarView {
                fraction: 0.5,
                opacity: 0.12,
            }),
            pixel_step: None,
            remaining_secs: 42,
            overlay_opacity: 1.0,
        }
    }

    #[test]
    fn background_fills_whole_buffer_when_secondary_screen() {
        let mut buffer = vec![0u32; 20 * 10];
        let view = base_view();
        let mut fake = FakeRasterizer::default();
        render_overlay(
            &mut buffer,
            20,
            10,
            1.0,
            ScreenRole::Secondary,
            &view,
            Language::ENGLISH,
            &mut fake,
        );
        assert!(
            buffer
                .iter()
                .all(|&p| p == CleaningColor::Black.rgb().to_u32())
        );
        assert!(
            fake.calls.is_empty(),
            "secondary screen must not rasterize any text"
        );
    }

    #[test]
    fn zero_overlay_opacity_draws_only_background() {
        let mut buffer = vec![0u32; 200 * 100];
        let view = SessionView {
            overlay_opacity: 0.0,
            ..base_view()
        };
        let mut fake = FakeRasterizer::default();
        render_overlay(
            &mut buffer,
            200,
            100,
            1.0,
            ScreenRole::Main,
            &view,
            Language::ENGLISH,
            &mut fake,
        );
        assert!(buffer.iter().all(|&p| p == Rgb::BLACK.to_u32()));
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn bar_width_matches_fraction() {
        let mut buffer = vec![0u32; 200 * 100];
        let view = SessionView {
            hint_opacity: 0.0,
            bar: Some(BarView {
                fraction: 0.25,
                opacity: 1.0,
            }),
            ..base_view()
        };
        let mut fake = FakeRasterizer::default();
        render_overlay(
            &mut buffer,
            200,
            100,
            1.0,
            ScreenRole::Main,
            &view,
            Language::ENGLISH,
            &mut fake,
        );
        let bar_row = 99 * 200;
        let lit_pixels = buffer[bar_row..bar_row + 200]
            .iter()
            .filter(|&&p| p == Rgb::WHITE.to_u32())
            .count();
        // 25% of 200px = 50px, full opacity so no partial blending to account for.
        assert_eq!(lit_pixels, 50);
    }

    #[test]
    fn hint_text_is_rasterized() {
        let mut buffer = vec![0u32; 400 * 300];
        let view = base_view();
        let mut fake = FakeRasterizer::default();
        render_overlay(
            &mut buffer,
            400,
            300,
            1.0,
            ScreenRole::Main,
            &view,
            Language::ENGLISH,
            &mut fake,
        );
        assert!(
            fake.runs_seen
                .iter()
                .any(|(text, _)| text.contains("close button")),
            "the hold-to-unlock hint text should have been rasterized"
        );
    }

    #[test]
    fn split_hint_runs_isolates_the_word_urahafu() {
        let runs = split_hint_runs("Type urahafu to unlock");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[1].text, "urahafu");
        assert_eq!(runs[1].weight, FontWeight::Semibold);
        assert!((runs[1].tracking_em - HINT_URAHAFU_TRACKING_EM).abs() < 1e-6);
        assert_eq!(runs[0].weight, FontWeight::Regular);
        assert_eq!(runs[2].weight, FontWeight::Regular);
    }

    #[test]
    fn hud_render_does_not_panic_and_paints_something() {
        let mut buffer = vec![0u32; 400 * 44];
        let view = base_view();
        let mut fake = FakeRasterizer::default();
        render_hud(
            &mut buffer,
            400,
            44,
            1.0,
            Appearance::Dark,
            &view,
            Language::ENGLISH,
            &mut fake,
        );
        assert!(buffer.iter().any(|&p| p != 0));
    }

    #[test]
    fn hud_content_width_is_positive() {
        let view = base_view();
        let mut fake = FakeRasterizer::default();
        let width = hud_content_width_points(&mut fake, &view, Language::ENGLISH);
        assert!(width > 0.0);
    }

    /// The HUD pill's ✕ button must sit to the right of the title text for left-to-right
    /// languages, and mirror to the left of it for RTL ones (DESIGN.md §1's RTL note; see
    /// `render_hud`'s doc comment for the full item order). This renders the pill for real with
    /// the fake rasterizer and locates the button by scanning a row that only the button's disc
    /// reaches (`center_y - 12`: the button's 14pt visual radius still paints there, while every
    /// text line and the glyph — each at most ~9pt tall above `center_y` — do not), then compares
    /// it against the title's expected left edge, computed the same way
    /// `hud_content_width_points`/`render_hud` do (measuring with the same fake rasterizer).
    #[test]
    fn hud_unlock_button_is_left_of_title_in_rtl_and_right_of_it_in_ltr() {
        for (language, button_should_be_left_of_title) in [
            (Language::ENGLISH, false),
            (Language::from_tag("ar").unwrap(), true),
        ] {
            let view = SessionView {
                unlock_button: Some(UnlockButtonView {
                    center: LayoutPoint::new(0.0, 0.0),
                    radius: layout::HUD_UNLOCK_BUTTON_RADIUS,
                    hit_radius: layout::HUD_UNLOCK_BUTTON_HIT_RADIUS,
                    opacity: 1.0,
                    hold_progress: 0.0,
                    scale: 1.0,
                }),
                ..base_view()
            };
            let mut fake = FakeRasterizer::default();
            let content_width = hud_content_width_points(&mut fake, &view, language).max(160.0);
            let pixel_width = content_width.round() as u32;
            let pixel_height = layout::HUD_PILL_HEIGHT as u32;
            let mut buffer = vec![0u32; (pixel_width * pixel_height) as usize];
            render_hud(
                &mut buffer,
                pixel_width,
                pixel_height,
                1.0,
                Appearance::Dark,
                &view,
                language,
                &mut fake,
            );

            let center_y = pixel_height as usize / 2;
            let scan_row = center_y - 12;
            let row_width = pixel_width as usize;
            let background = hud_colors(Appearance::Dark).0.to_u32();
            // Exclude the outermost columns: `render_hud` paints a 1px border along every edge
            // (including the left/right columns of every row), which would otherwise be picked
            // up as "painted" here alongside the button.
            let is_painted = |x: usize| buffer[scan_row * row_width + x] != background;
            let button_first = (1..row_width - 1)
                .find(|&x| is_painted(x))
                .expect("button must paint at this row");
            let button_last = (1..row_width - 1)
                .rev()
                .find(|&x| is_painted(x))
                .expect("button must paint at this row");
            let button_center_x = (button_first + button_last) as f32 / 2.0;

            // The title's expected left edge, following `render_hud`'s own item order.
            let mut fake_for_measure = FakeRasterizer::default();
            let primary_width = fake_for_measure.measure(
                &[TextRun::plain(
                    Text::HudKeyboardLocked.get(language),
                    FontWeight::Semibold,
                )],
                HUD_TEXT_FONT_SIZE,
            );
            let title_left_x = if language.is_rtl() {
                content_width - HUD_PADDING_X - HUD_GLYPH_SIZE - HUD_ITEM_GAP - primary_width
            } else {
                HUD_PADDING_X + HUD_GLYPH_SIZE + HUD_ITEM_GAP
            };
            let title_center_x = title_left_x + primary_width / 2.0;

            if button_should_be_left_of_title {
                assert!(
                    button_center_x < title_center_x,
                    "expected the button (x={button_center_x}) left of the title \
                     (x={title_center_x}) for {language:?}"
                );
            } else {
                assert!(
                    button_center_x > title_center_x,
                    "expected the button (x={button_center_x}) right of the title \
                     (x={title_center_x}) for {language:?}"
                );
            }
        }
    }

    /// Regression test for a countdown label overlapping the top of the countdown digit
    /// (mockup `04-countdown`): the label used to sit a fixed 110 pt above the digit's
    /// *baseline*, which for a 160 pt digit left only ~2 pt of clearance above its actual cap
    /// top — this renders the digit and label with the real (fake, in this test) rasterizer and
    /// checks their painted pixel rects on the screen's vertical center line don't touch.
    #[test]
    fn countdown_label_does_not_overlap_the_digit() {
        let width = 400usize;
        let height = 600usize;
        let mut buffer = vec![0u32; width * height];
        let view = SessionView {
            countdown: Some(CountdownView {
                digit: 3,
                opacity: 1.0,
                scale: 1.0,
            }),
            ..base_view()
        };
        let mut fake = FakeRasterizer::default();
        render_overlay(
            &mut buffer,
            width as u32,
            height as u32,
            1.0,
            ScreenRole::Main,
            &view,
            Language::ENGLISH,
            &mut fake,
        );

        // During the countdown, `draw_countdown` is the only thing painted (`render_overlay`
        // returns right after it), and every element it draws — digit, label, cancel hint — is
        // horizontally centered, so the screen's center column crosses all of them. Top to
        // bottom that's: label, digit, cancel hint (near the very bottom); collect the painted
        // (non-background) row spans on that column and check the first two (label, digit) are
        // disjoint.
        let center_x = width / 2;
        let mut segments: Vec<(usize, usize)> = Vec::new();
        let mut in_segment = false;
        for y in 0..height {
            let painted = buffer[y * width + center_x] != Rgb::BLACK.to_u32();
            if painted {
                if in_segment {
                    segments.last_mut().expect("in_segment implies non-empty").1 = y;
                } else {
                    segments.push((y, y));
                    in_segment = true;
                }
            } else {
                in_segment = false;
            }
        }

        assert!(
            segments.len() >= 2,
            "expected at least a label segment and a digit segment on the center column, got \
             {segments:?}"
        );
        let (label_start, label_end) = segments[0];
        let (digit_start, digit_end) = segments[1];
        assert!(
            label_end < digit_start,
            "label (rows {label_start}..={label_end}) must not overlap the digit (rows \
             {digit_start}..={digit_end})"
        );
    }

    /// Regression test for a "text is invisible on the white cleaning screen" report that turned
    /// out to be caused by the `examples/render_preview.rs` snapshot sampling the hint at
    /// `hint_opacity == 0.0` (the very first frame of a re-show fade-in), not by any ink/color
    /// bug in this module's blending. This test pins down the actual contract `draw_hint_block`
    /// must uphold regardless of background color: with `hint_opacity` at its steady-state value,
    /// hint text must actually paint pixels, and those pixels must move *away* from the
    /// background toward `view.ink` — darker than a white background, lighter than a black one —
    /// never a no-op.
    #[test]
    fn hint_text_darkens_white_background_and_lightens_black_background() {
        for (background, ink) in [(Rgb::WHITE, Rgb::BLACK), (Rgb::BLACK, Rgb::WHITE)] {
            let mut buffer = vec![0u32; 400 * 300];
            let view = SessionView {
                background,
                ink,
                hint_opacity: 1.0,
                ..base_view()
            };
            let mut fake = FakeRasterizer::default();
            render_overlay(
                &mut buffer,
                400,
                300,
                1.0,
                ScreenRole::Main,
                &view,
                Language::ENGLISH,
                &mut fake,
            );

            let layout = Layout::new(ScreenSize::new(400.0, 300.0));
            let hint = layout.hint();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (x, y) = (hint.position.x as usize, hint.position.y as usize - 4);
            let pixel = buffer[y * 400 + x];
            let painted = Rgb::new(
                u8::try_from((pixel >> 16) & 0xFF).unwrap_or(0),
                u8::try_from((pixel >> 8) & 0xFF).unwrap_or(0),
                u8::try_from(pixel & 0xFF).unwrap_or(0),
            );
            assert_ne!(
                painted, background,
                "hint text must actually paint pixels over {background} (found untouched \
                 background) — hint_opacity is 1.0 here, not the fade-in's opacity-0 frame"
            );
            if background == Rgb::WHITE {
                assert!(
                    painted.r < background.r,
                    "hint ink must darken a white background, got {painted}"
                );
            } else {
                assert!(
                    painted.r > background.r,
                    "hint ink must lighten a black background, got {painted}"
                );
            }
        }
    }
}
