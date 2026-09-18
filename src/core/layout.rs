//! Pure layout computation: positions and sizes (in points) for a given screen size.
//!
//! Every function here is a pure function of a [`ScreenSize`]; nothing is measured or drawn.
//! The renderer (`src/platform/overlay.rs`) is responsible for multiplying these point values by
//! the display's backing scale factor to get pixel coordinates, and for measuring actual glyph
//! metrics where a size here is a font size rather than a box (see the module-level "coordinate
//! system" note below for why point sizes are not additionally scaled to screen size).
//!
//! ## Coordinate system
//!
//! Origin at the top-left, `x` right, `y` down — the same convention as the pixel buffers in
//! [`crate::core::canvas`], so the renderer can use these values with only a DPI-scale
//! multiplication, no axis flip.
//!
//! ## Why sizes don't scale with screen size
//!
//! DESIGN.md gives its point sizes "relative to a reference screen of 1440×900 pt" — that is the
//! canvas the designs were drawn against, not an instruction to scale point sizes by the actual
//! screen's dimensions. Points are already resolution-independent (a 17 pt label reads the same
//! physical size on a 13" and a 27" display), so every font size and fixed offset below is used
//! as-is regardless of `ScreenSize`; only horizontal/vertical centering adapts to it.

/// The dimensions of the screen (or window) being laid out, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenSize {
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
}

impl ScreenSize {
    /// Creates a screen size.
    #[must_use]
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    /// Horizontal center.
    #[must_use]
    fn center_x(self) -> f32 {
        self.width / 2.0
    }
}

/// A 2D point in points.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// X coordinate.
    pub x: f32,
    /// Y coordinate.
    pub y: f32,
}

impl Point {
    /// Creates a point.
    #[must_use]
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle in points, `(x, y)` at the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Rect {
    /// Creates a rectangle.
    #[must_use]
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A text anchor: a baseline position plus the font size to draw it at. Horizontally centered
/// text is anchored at its own center; the renderer is expected to center the measured glyph run
/// on `position.x`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextAnchor {
    /// Anchor position: `x` is the horizontal center, `y` is the text baseline.
    pub position: Point,
    /// Font size in points.
    pub font_size: f32,
}

/// Countdown digit font size (DESIGN.md §5).
pub const COUNTDOWN_DIGIT_FONT_SIZE: f32 = 160.0;
/// Countdown label ("Cleaning is about to start") font size.
pub const COUNTDOWN_LABEL_FONT_SIZE: f32 = 17.0;
/// Clear space between the digit's measured cap top and the countdown starting label's baseline
/// (DESIGN.md §7: the label sits clearly above the digit, with a comfortable gap — see mockup
/// `04-countdown`). Deliberately measured from the digit's real *cap top*, not its baseline: a
/// fixed offset from the baseline (this module's previous approach) does not account for the
/// digit's actual glyph height, and drifted into overlapping the top of tall digits like "3" —
/// see [`Layout::countdown_label_above`] and its caller in `render.rs`, which measures the real
/// rasterized cap top before calling in here.
const COUNTDOWN_LABEL_CLEAR_SPACE_ABOVE_DIGIT_CAP: f32 = 24.0;
/// Countdown cancel hint font size.
pub const COUNTDOWN_CANCEL_HINT_FONT_SIZE: f32 = 13.0;
/// Minimum clear space required between the countdown cancel hint's text and the hold-to-unlock
/// button's *hit* circle (not just its smaller visual one), per DESIGN.md §7 (see
/// [`Layout::countdown_cancel_hint`] for why this replaced a fixed distance from the bottom
/// edge).
const COUNTDOWN_CANCEL_HINT_GAP_ABOVE_BUTTON: f32 = 16.0;
/// Conservative approximation, as a fraction of font size, of how far a short line of
/// regular-weight body text descends below its own baseline. This module works in pure geometry
/// with no rasterizer (see the module doc comment), so it cannot measure the real descent of the
/// system font in every supported script; this ratio intentionally over-estimates a normal
/// Latin line's real descent, which only makes the clearance computed from it more conservative,
/// never less.
const COUNTDOWN_CANCEL_HINT_DESCENT_RATIO: f32 = 0.35;

/// Unlock hint font size.
pub const HINT_FONT_SIZE: f32 = 15.0;
/// Second line (shortcuts / time remaining) font size.
pub const SECOND_LINE_FONT_SIZE: f32 = 12.0;
/// Second line vertical offset below the hint's baseline (unchanged from the typed-sequence
/// layout).
const SECOND_LINE_BELOW_HINT: f32 = 22.0;
/// Wordmark font size.
pub const WORDMARK_FONT_SIZE: f32 = 15.0;
/// Pixel test step indicator font size.
pub const PIXEL_TEST_STEP_FONT_SIZE: f32 = 11.0;
/// Horizontal gap placed between the second line and the pixel-test step indicator that sits
/// beside it (DESIGN.md §8: "appears next to the second line"; the renderer positions it
/// immediately after the measured width of the second line's text plus this gap).
pub const PIXEL_TEST_STEP_GAP: f32 = 8.0;

/// Height of the time-remaining bar.
pub const BAR_HEIGHT: f32 = 2.0;

/// The hold-to-unlock button's visual diameter (spec: "44 pt").
pub const UNLOCK_BUTTON_DIAMETER: f32 = 44.0;
/// The hold-to-unlock button's visual radius.
pub const UNLOCK_BUTTON_RADIUS: f32 = UNLOCK_BUTTON_DIAMETER / 2.0;
/// The hold-to-unlock button's (circular) hit target diameter (spec: "hit target 64 pt").
pub const UNLOCK_BUTTON_HIT_DIAMETER: f32 = 64.0;
/// The hold-to-unlock button's hit target radius.
pub const UNLOCK_BUTTON_HIT_RADIUS: f32 = UNLOCK_BUTTON_HIT_DIAMETER / 2.0;
/// Gap between the second line's baseline and the top of the button's *visual* circle (spec:
/// "20 pt gap").
const UNLOCK_BUTTON_GAP_ABOVE_SECOND_LINE: f32 = 20.0;
/// Minimum clear space between the bottom of the button's visual circle and the screen's bottom
/// edge (spec: "bottom margin ≥ 56 pt").
const UNLOCK_BUTTON_BOTTOM_MARGIN: f32 = 56.0;

/// Countdown's hold-to-unlock explanation line font size (spec: "15 pt").
pub const COUNTDOWN_UNLOCK_EXPLANATION_FONT_SIZE: f32 = 15.0;
/// Vertical offset of the countdown's hold-to-unlock explanation line below the countdown
/// digit's own anchor (spec: "under the digit area"). This is a fixed offset from the digit's
/// anchor position (screen center), not from its measured glyph metrics — unlike
/// [`Layout::countdown_label_above`], which sits *above* the digit and does need the real
/// rasterized cap top to avoid overlapping a tall digit, a line placed comfortably *below* the
/// digit's center has no such risk for any digit 1-9 at this font's size.
const COUNTDOWN_UNLOCK_EXPLANATION_BELOW_DIGIT: f32 = 130.0;

/// The keyboard-only HUD's hold-to-unlock button visual diameter (spec: "28 pt").
pub const HUD_UNLOCK_BUTTON_DIAMETER: f32 = 28.0;
/// The keyboard-only HUD's hold-to-unlock button visual radius.
pub const HUD_UNLOCK_BUTTON_RADIUS: f32 = HUD_UNLOCK_BUTTON_DIAMETER / 2.0;
/// The keyboard-only HUD's hold-to-unlock button hit target diameter (spec: "hit 36 pt").
pub const HUD_UNLOCK_BUTTON_HIT_DIAMETER: f32 = 36.0;
/// The keyboard-only HUD's hold-to-unlock button hit target radius.
pub const HUD_UNLOCK_BUTTON_HIT_RADIUS: f32 = HUD_UNLOCK_BUTTON_HIT_DIAMETER / 2.0;
/// Horizontal padding between the HUD button's visual edge and the pill's right edge (DESIGN.md
/// gives no exact number for this; this module's own reasonable choice, matching
/// [`super::layout`]'s existing `HUD_PADDING_X`-style values).
const HUD_UNLOCK_BUTTON_TRAILING_PADDING: f32 = 12.0;

/// Height of the keyboard-only HUD pill (DESIGN.md §9).
pub const HUD_PILL_HEIGHT: f32 = 44.0;
/// Assumed menu bar height the HUD sits under (DESIGN.md §9 mockups; standard macOS menu bar).
pub const MENU_BAR_HEIGHT: f32 = 24.0;
/// Gap between the menu bar and the top of the HUD pill.
pub const HUD_PILL_GAP_BELOW_MENU_BAR: f32 = 12.0;
/// HUD pill corner radius: half its height, giving a fully rounded (pill) shape.
pub const HUD_PILL_CORNER_RADIUS: f32 = HUD_PILL_HEIGHT / 2.0;

/// Computes every layout position for a given screen size.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    screen: ScreenSize,
}

impl Layout {
    /// Creates a layout calculator for `screen`.
    #[must_use]
    pub fn new(screen: ScreenSize) -> Self {
        Self { screen }
    }

    /// The countdown digit (3-2-1), centered on screen.
    #[must_use]
    pub fn countdown_digit(&self) -> TextAnchor {
        TextAnchor {
            position: Point::new(self.screen.center_x(), self.screen.height / 2.0),
            font_size: COUNTDOWN_DIGIT_FONT_SIZE,
        }
    }

    /// The countdown starting label, `COUNTDOWN_LABEL_CLEAR_SPACE_ABOVE_DIGIT_CAP` pt above
    /// `digit_cap_top` — the countdown digit's real, measured cap top (in points, same
    /// coordinate system as this layout), not a value this module can compute on its own: unlike
    /// every other position here, it depends on the actual rasterized glyph's metrics (the
    /// digit's entrance-animation scale changes its rendered size — DESIGN.md §7), which only the
    /// caller in `render.rs` has, from its text rasterizer.
    #[must_use]
    pub fn countdown_label_above(&self, digit_cap_top: f32) -> TextAnchor {
        TextAnchor {
            position: Point::new(
                self.screen.center_x(),
                digit_cap_top - COUNTDOWN_LABEL_CLEAR_SPACE_ABOVE_DIGIT_CAP,
            ),
            font_size: COUNTDOWN_LABEL_FONT_SIZE,
        }
    }

    /// The "Esc or click to cancel" hint during the countdown, positioned *above* the
    /// hold-to-unlock button (DESIGN.md §7 / mockup `04-countdown`) with at least
    /// `COUNTDOWN_CANCEL_HINT_GAP_ABOVE_BUTTON` pt of clear space to the button's hit circle.
    ///
    /// The button is drawn at its final, shared-with-the-locked-screen position
    /// ([`Layout::unlock_button`]) throughout the countdown — that is the point of showing it
    /// early. An earlier revision placed this hint at a fixed 72 pt from the bottom edge instead,
    /// which put its baseline inside the button's 64 pt hit circle once the button sat at that
    /// real position; anchoring to the button itself keeps the two apart regardless of screen
    /// size.
    #[must_use]
    pub fn countdown_cancel_hint(&self) -> TextAnchor {
        let button = self.unlock_button();
        let button_hit_top = button.y - UNLOCK_BUTTON_HIT_RADIUS;
        let descent = COUNTDOWN_CANCEL_HINT_FONT_SIZE * COUNTDOWN_CANCEL_HINT_DESCENT_RATIO;
        let baseline_y = button_hit_top - COUNTDOWN_CANCEL_HINT_GAP_ABOVE_BUTTON - descent;
        TextAnchor {
            position: Point::new(self.screen.center_x(), baseline_y),
            font_size: COUNTDOWN_CANCEL_HINT_FONT_SIZE,
        }
    }

    /// The hold-to-unlock ✕ button, centered horizontally, sitting at the bottom of the stack:
    /// its visual circle's bottom edge is `UNLOCK_BUTTON_BOTTOM_MARGIN` above the screen's
    /// bottom edge.
    #[must_use]
    pub fn unlock_button(&self) -> Point {
        let center_y = self.screen.height - UNLOCK_BUTTON_BOTTOM_MARGIN - UNLOCK_BUTTON_RADIUS;
        Point::new(self.screen.center_x(), center_y)
    }

    /// The second line (dead-pixel test shortcut + remaining time), `UNLOCK_BUTTON_GAP_ABOVE_SECOND_LINE`
    /// above the top of the unlock button's visual circle.
    #[must_use]
    pub fn second_line(&self) -> TextAnchor {
        let button = self.unlock_button();
        let button_top = button.y - UNLOCK_BUTTON_RADIUS;
        TextAnchor {
            position: Point::new(
                self.screen.center_x(),
                button_top - UNLOCK_BUTTON_GAP_ABOVE_SECOND_LINE,
            ),
            font_size: SECOND_LINE_FONT_SIZE,
        }
    }

    /// The unlock hint ("Hold ✕ for 2 seconds to unlock"), `SECOND_LINE_BELOW_HINT` above the
    /// second line.
    #[must_use]
    pub fn hint(&self) -> TextAnchor {
        let second_line = self.second_line();
        TextAnchor {
            position: Point::new(
                second_line.position.x,
                second_line.position.y - SECOND_LINE_BELOW_HINT,
            ),
            font_size: HINT_FONT_SIZE,
        }
    }

    /// The wordmark ("urahafu"), shown with the initial hint. Centered on screen both ways: the
    /// hint occupies the lower part of the screen, so a screen-centered wordmark reads as a
    /// separate, higher element rather than crowding the hint block.
    #[must_use]
    pub fn wordmark(&self) -> TextAnchor {
        TextAnchor {
            position: Point::new(self.screen.center_x(), self.screen.height / 2.0),
            font_size: WORDMARK_FONT_SIZE,
        }
    }

    /// The countdown screen's hold-to-unlock explanation line ("To unlock, hold ✕ for
    /// 2 seconds"), under the digit area.
    #[must_use]
    pub fn countdown_unlock_explanation(&self) -> TextAnchor {
        let digit = self.countdown_digit();
        TextAnchor {
            position: Point::new(
                digit.position.x,
                digit.position.y + COUNTDOWN_UNLOCK_EXPLANATION_BELOW_DIGIT,
            ),
            font_size: COUNTDOWN_UNLOCK_EXPLANATION_FONT_SIZE,
        }
    }

    /// The time-remaining bar: full width, [`BAR_HEIGHT`] tall, flush with the bottom edge.
    #[must_use]
    pub fn bar(&self) -> Rect {
        Rect::new(
            0.0,
            self.screen.height - BAR_HEIGHT,
            self.screen.width,
            BAR_HEIGHT,
        )
    }

    /// The pixel-test step indicator ("2/5"), anchored at the same baseline as the second line,
    /// `after_second_line_width` (the renderer's measured width of the second line's text) plus
    /// [`PIXEL_TEST_STEP_GAP`] to its right.
    #[must_use]
    pub fn pixel_test_step(&self, after_second_line_width: f32) -> TextAnchor {
        let second_line = self.second_line();
        TextAnchor {
            position: Point::new(
                second_line.position.x + after_second_line_width / 2.0 + PIXEL_TEST_STEP_GAP,
                second_line.position.y,
            ),
            font_size: PIXEL_TEST_STEP_FONT_SIZE,
        }
    }

    /// The keyboard-only HUD pill's bounding box, of the given `width` (the renderer measures
    /// its content to determine width; this layout only fixes height and vertical position).
    #[must_use]
    pub fn hud_pill(&self, width: f32) -> Rect {
        let top = MENU_BAR_HEIGHT + HUD_PILL_GAP_BELOW_MENU_BAR;
        Rect::new(
            self.screen.center_x() - width / 2.0,
            top,
            width,
            HUD_PILL_HEIGHT,
        )
    }
}

/// The keyboard-only HUD's hold-to-unlock ✕ button center, vertically centered on `pill` (the
/// HUD pill's own bounding box, from [`Layout::hud_pill`]). Horizontally: at the right end for
/// left-to-right languages, or mirrored to the left end when `rtl` is `true` — `render_hud`
/// mirrors the whole pill's content order for RTL languages (DESIGN.md §9, §1 RTL note), and the
/// button must move with it so this hit-testing target still matches what is actually drawn (see
/// `render::render_hud`'s module doc comment for the full item order in both directions). A free
/// function rather than a [`Layout`] method: `pill` and `rtl` alone (not the screen size) fully
/// determine this position.
#[must_use]
pub fn hud_unlock_button(pill: Rect, rtl: bool) -> Point {
    let x = if rtl {
        pill.x + HUD_UNLOCK_BUTTON_TRAILING_PADDING + HUD_UNLOCK_BUTTON_RADIUS
    } else {
        pill.x + pill.width - HUD_UNLOCK_BUTTON_TRAILING_PADDING - HUD_UNLOCK_BUTTON_RADIUS
    };
    Point::new(x, pill.y + pill.height / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERENCE: ScreenSize = ScreenSize {
        width: 1440.0,
        height: 900.0,
    };

    #[test]
    fn countdown_digit_is_centered() {
        let layout = Layout::new(REFERENCE);
        let anchor = layout.countdown_digit();
        assert!((anchor.position.x - 720.0).abs() < 1e-3);
        assert!((anchor.position.y - 450.0).abs() < 1e-3);
        assert!((anchor.font_size - 160.0).abs() < 1e-3);
    }

    #[test]
    fn countdown_label_sits_24pt_above_the_given_digit_cap_top() {
        let layout = Layout::new(REFERENCE);
        let digit_cap_top = 300.0;
        let label = layout.countdown_label_above(digit_cap_top);
        assert!((digit_cap_top - label.position.y - 24.0).abs() < 1e-3);
        assert!((label.position.x - layout.countdown_digit().position.x).abs() < 1e-3);
    }

    #[test]
    fn cancel_hint_sits_above_the_unlock_button() {
        let layout = Layout::new(REFERENCE);
        let hint = layout.countdown_cancel_hint();
        let button = layout.unlock_button();
        assert!(hint.position.y < button.y - UNLOCK_BUTTON_HIT_RADIUS);
    }

    /// Regression test: the cancel hint used to sit at a fixed 72 pt from the bottom edge, which
    /// on a 900 pt-tall reference screen put its baseline (828) inside the unlock button's hit
    /// circle (center 822, hit radius 32 -> spans 790..854) once the button was drawn at its
    /// real, shared-with-the-locked-screen position — see `Layout::countdown_cancel_hint`'s doc
    /// comment. Checks a conservative text box around the hint's baseline (a full font-size worth
    /// of ascent above, `COUNTDOWN_CANCEL_HINT_DESCENT_RATIO` below — both deliberately generous
    /// over-estimates, see that constant's doc comment) never intersects the button's hit circle,
    /// with at least `COUNTDOWN_CANCEL_HINT_GAP_ABOVE_BUTTON` pt to spare, across every screen
    /// size this app is expected to run on.
    #[test]
    fn cancel_hint_text_box_does_not_intersect_the_unlock_button_hit_circle() {
        for screen in [
            ScreenSize::new(1440.0, 900.0),
            ScreenSize::new(1280.0, 800.0),
            ScreenSize::new(1728.0, 1117.0),
            ScreenSize::new(2560.0, 1440.0),
        ] {
            let layout = Layout::new(screen);
            let hint = layout.countdown_cancel_hint();
            let button = layout.unlock_button();

            let hint_box_top = hint.position.y - hint.font_size;
            let hint_box_bottom =
                hint.position.y + hint.font_size * COUNTDOWN_CANCEL_HINT_DESCENT_RATIO;
            let button_hit_top = button.y - UNLOCK_BUTTON_HIT_RADIUS;
            let button_hit_bottom = button.y + UNLOCK_BUTTON_HIT_RADIUS;

            assert!(
                hint_box_top < hint_box_bottom,
                "sanity: the hint's text box must be non-empty on a {}x{} screen",
                screen.width,
                screen.height
            );
            assert!(
                hint_box_bottom + COUNTDOWN_CANCEL_HINT_GAP_ABOVE_BUTTON <= button_hit_top,
                "cancel hint box (bottom {hint_box_bottom}) must clear the unlock button's hit \
                 circle (top {button_hit_top}) by at least \
                 {COUNTDOWN_CANCEL_HINT_GAP_ABOVE_BUTTON}pt on a {}x{} screen",
                screen.width,
                screen.height
            );
            // Redundant with the gap check above given the hint sits above the button, but spelled
            // out explicitly since it is the literal property this test is named after.
            assert!(
                hint_box_bottom < button_hit_top || hint_box_top > button_hit_bottom,
                "cancel hint text box must not intersect the unlock button's hit circle on a \
                 {}x{} screen",
                screen.width,
                screen.height
            );
        }
    }

    #[test]
    fn second_line_is_22pt_below_hint() {
        let layout = Layout::new(REFERENCE);
        let hint = layout.hint();
        let second = layout.second_line();
        assert!((second.position.y - hint.position.y - 22.0).abs() < 1e-3);
    }

    #[test]
    fn unlock_button_is_centered_horizontally() {
        let layout = Layout::new(REFERENCE);
        let button = layout.unlock_button();
        assert!((button.x - REFERENCE.width / 2.0).abs() < 1e-3);
    }

    #[test]
    fn unlock_button_bottom_margin_is_at_least_56pt() {
        let layout = Layout::new(REFERENCE);
        let button = layout.unlock_button();
        let bottom_edge = button.y + UNLOCK_BUTTON_RADIUS;
        assert!(REFERENCE.height - bottom_edge >= 56.0 - 1e-3);
    }

    #[test]
    fn second_line_sits_20pt_above_the_unlock_button() {
        let layout = Layout::new(REFERENCE);
        let button = layout.unlock_button();
        let button_top = button.y - UNLOCK_BUTTON_RADIUS;
        let second_line = layout.second_line();
        assert!((button_top - second_line.position.y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn bottom_stack_does_not_overlap_top_to_bottom() {
        // hint < second line < unlock button top < unlock button bottom < screen bottom.
        let layout = Layout::new(REFERENCE);
        let hint = layout.hint();
        let second_line = layout.second_line();
        let button = layout.unlock_button();
        assert!(hint.position.y < second_line.position.y);
        assert!(second_line.position.y < button.y - UNLOCK_BUTTON_RADIUS);
        assert!(button.y + UNLOCK_BUTTON_RADIUS < REFERENCE.height);
    }

    #[test]
    fn countdown_unlock_explanation_sits_below_the_digit_and_above_the_cancel_hint() {
        let layout = Layout::new(REFERENCE);
        let digit = layout.countdown_digit();
        let explanation = layout.countdown_unlock_explanation();
        let cancel_hint = layout.countdown_cancel_hint();
        assert!(explanation.position.y > digit.position.y);
        assert!(explanation.position.y < cancel_hint.position.y);
        assert!((explanation.position.x - digit.position.x).abs() < 1e-3);
    }

    #[test]
    fn hud_unlock_button_sits_at_the_right_end_of_the_pill() {
        let layout = Layout::new(REFERENCE);
        let pill = layout.hud_pill(400.0);
        let button = hud_unlock_button(pill, false);
        assert!((button.y - (pill.y + pill.height / 2.0)).abs() < 1e-3);
        assert!(button.x < pill.x + pill.width);
        assert!(button.x + HUD_UNLOCK_BUTTON_RADIUS <= pill.x + pill.width + 1e-3);
    }

    #[test]
    fn hud_unlock_button_mirrors_to_the_left_end_of_the_pill_when_rtl() {
        let layout = Layout::new(REFERENCE);
        let pill = layout.hud_pill(400.0);
        let ltr = hud_unlock_button(pill, false);
        let rtl = hud_unlock_button(pill, true);
        assert!(
            (rtl.y - ltr.y).abs() < 1e-3,
            "vertical position is unaffected by rtl"
        );
        assert!(
            rtl.x < ltr.x,
            "the rtl button must sit to the left of the ltr one"
        );
        assert!(rtl.x - HUD_UNLOCK_BUTTON_RADIUS >= pill.x - 1e-3);
        assert!(rtl.x > pill.x);
    }

    #[test]
    fn bar_spans_full_width_and_is_2pt_tall() {
        let layout = Layout::new(REFERENCE);
        let bar = layout.bar();
        assert!((bar.width - REFERENCE.width).abs() < 1e-3);
        assert!((bar.height - 2.0).abs() < 1e-3);
        assert!((bar.y + bar.height - REFERENCE.height).abs() < 1e-3);
    }

    #[test]
    fn hud_pill_is_44pt_tall_12pt_under_menu_bar() {
        let layout = Layout::new(REFERENCE);
        let pill = layout.hud_pill(400.0);
        assert!((pill.height - 44.0).abs() < 1e-3);
        assert!((pill.y - 36.0).abs() < 1e-3);
        assert!((pill.width - 400.0).abs() < 1e-3);
    }

    #[test]
    fn hud_pill_is_centered_horizontally() {
        let layout = Layout::new(REFERENCE);
        let pill = layout.hud_pill(400.0);
        let center = pill.x + pill.width / 2.0;
        assert!((center - REFERENCE.width / 2.0).abs() < 1e-3);
    }

    #[test]
    fn layout_adapts_to_a_different_screen_size() {
        let layout = Layout::new(ScreenSize::new(2560.0, 1440.0));
        let anchor = layout.countdown_digit();
        assert!((anchor.position.x - 1280.0).abs() < 1e-3);
        assert!(
            (anchor.font_size - 160.0).abs() < 1e-3,
            "font size should not scale with screen size"
        );
    }
}
