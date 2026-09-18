//! The native settings window (DESIGN.md "Settings window"): a titled `NSWindow`, built once and
//! reused for the app's whole lifetime, showing the same settings the old menu exposed (color,
//! auto-unlock delay, keyboard-only mode, menu bar icon, login item) plus a permission banner and
//! a "Clean" button. No custom drawing (no icons, no `CALayer`s): every visual is a stock `AppKit`
//! control or an `NSBox`, laid out with `NSStackView`s and plain `NSLayoutConstraint` anchors.
//!
//! Safe code only: every `AppKit` call objc2-app-kit 0.3.2 marks `unsafe` (building the window
//! itself, wiring a control's target/action) is wrapped by
//! [`crate::platform::ffi::action_target`] (`docs/ARCHITECTURE.md` goal 4 — every `unsafe` block
//! lives under `src/platform/ffi/`).
//!
//! Two `AppKit` facts this file leans on throughout:
//! - An `NSStackView`'s "alignment" only pins one edge (or centers) an arranged subview along the
//!   cross axis; it never caps the opposite edge. A view that isn't otherwise constrained can grow
//!   past its stack's bounds, which is exactly how the previous layout broke (content overflowing,
//!   the Clean button pushed out of the window). Every view here that must span, or wrap within, a
//!   known width therefore gets an explicit `widthAnchor`/`heightAnchor` constraint (the
//!   `pin_*` helpers below) rather than relying on alignment alone.
//! - `NSBox` is not an `NSControl` (no target/action, no clicks). The color swatches and the Clean
//!   button need both an arbitrary flat fill color/corner radius (which a bordered `NSButton`
//!   cannot do) and a click target, so each is a small stack of overlaid views: an `NSBox` for the
//!   fill/border and a borderless `NSButton` on top of it for the hit target. `NSBox`'s fill/border
//!   colors are looked up again by `AppKit` on every draw, so a dynamic `NSColor` (banner fill,
//!   group box fill, hairline separators) stays correct across light/dark switches with no extra
//!   code; this window's own custom colors (accent teal, pure black/white) are fixed hex values
//!   that don't depend on the appearance, so there is nothing to re-apply either.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::NSColor;
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSBezelStyle, NSBox, NSBoxType, NSButton, NSControlSize,
    NSControlStateValueOff, NSControlStateValueOn, NSFont, NSImageScaling, NSImageView,
    NSLayoutAttribute, NSLayoutConstraintOrientation, NSSegmentedControl, NSStackView, NSSwitch,
    NSTextField, NSUserInterfaceLayoutDirection, NSUserInterfaceLayoutOrientation, NSView,
    NSWindow,
};
use objc2_foundation::{NSArray, NSEdgeInsets, NSPoint, NSRect, NSSize, NSString};

use crate::core::color::CleaningColor;
use crate::core::failsafe::FailsafeDelay;
use crate::core::i18n::{Language, Text};
use crate::core::settings::Settings;
use crate::platform::ffi::action_target::{self, ActionId, ActionTarget};

/// Content width (points; DESIGN.md "Settings window" › "Layout"): every full-width view (group
/// boxes, banner, Clean button) is pinned to exactly this, so the window can never overflow
/// horizontally regardless of how long a translated string is.
const CONTENT_WIDTH: f64 = 400.0;
/// Margin (points) around the content on every side; total window width is
/// `CONTENT_WIDTH + 2 * MARGIN` = 448.
const MARGIN: f64 = 24.0;
/// Spacing (points) between top-level sections in the main vertical stack.
const SPACING: f64 = 18.0;
/// Horizontal padding (points) inside a group-box row, and the inset of the hairline separators
/// between rows.
const ROW_INSET: f64 = 16.0;
/// A group-box row's fixed height (points).
const ROW_HEIGHT: f64 = 44.0;
/// A row's usable width once its own [`ROW_INSET`] padding is subtracted — used to size a row's
/// trailing content that itself needs a known width (the banner's button row).
const ROW_INNER_WIDTH: f64 = CONTENT_WIDTH - 2.0 * ROW_INSET;
/// A color swatch's own size (points; DESIGN.md "Settings window": "26×26 pt rounded squares").
const SWATCH_SIZE: f64 = 26.0;
/// Gap (points) between a swatch and its selection ring.
const SWATCH_RING_GAP: f64 = 2.0;
/// The selection ring's stroke width (points).
const SWATCH_RING_STROKE: f64 = 2.0;
/// Spacing (points) between the views of a group-box row's horizontal stack (label, spacer,
/// control).
const ROW_STACK_SPACING: f64 = 8.0;
/// Width (points) a row's label must leave free besides its control: the two stack gaps around
/// the spacer, plus a little breathing room.
const ROW_CONTROL_GAP: f64 = 2.0 * ROW_STACK_SPACING + 4.0;
/// Width (points) reserved for a small switch plus the gap before it, on a group-box row whose
/// leading side wraps.
const SWITCH_COLUMN_WIDTH: f64 = 64.0;
/// The swatch's own corner radius (points).
const SWATCH_CORNER_RADIUS: f64 = 7.0;
/// The selection ring's overall size (points): the swatch, plus the gap and the ring's own stroke
/// on every side.
const SWATCH_RING_SIZE: f64 = SWATCH_SIZE + 2.0 * (SWATCH_RING_GAP + SWATCH_RING_STROKE);
/// The header's app icon size (points; DESIGN.md "Settings window").
const HEADER_ICON_SIZE: f64 = 52.0;
/// Spacing (points) between the header's icon and its title/tagline text.
const HEADER_SPACING: f64 = 14.0;
/// The Clean button's height (points).
const CLEAN_BUTTON_HEIGHT: f64 = 44.0;
/// Corner radius (points) shared by every rounded box in this window (banner, group boxes, Clean
/// button).
const CORNER_RADIUS: f64 = 10.0;
/// The Clean button's opacity while disabled (permission missing).
const CLEAN_BUTTON_DISABLED_ALPHA: f64 = 0.4;

/// `NSFontWeightSemibold`'s documented value (`AppKit/NSFontDescriptor.h`; the same constant
/// `UIFontWeightSemibold` uses). Used as a plain `f64` literal rather than the upstream `extern
/// "C"` static so this safe-code-only module never needs an `unsafe` block just to read a global.
const FONT_WEIGHT_SEMIBOLD: f64 = 0.3;

/// Tag identifying which control fired `performAction:` (`ActionId::Tag`,
/// `src/platform/ffi/action_target.rs`), decoded back into a `WindowCommand` by
/// `src/app/window.rs`'s pure app-level mapping (kept there, not here, so it is
/// unit-tested without `AppKit`).
pub mod tag {
    /// The permission banner's "Open System Settings" button.
    pub const GRANT_ACCESS: isize = 1;
    /// The black color swatch.
    pub const COLOR_BLACK: isize = 2;
    /// The auto-unlock delay segmented control.
    pub const FAILSAFE: isize = 3;
    /// The "Keyboard-Only Mode" switch.
    pub const KEYBOARD_ONLY: isize = 4;
    /// The "Show Icon in Menu Bar" switch.
    pub const SHOW_ICON: isize = 5;
    /// The "Open at Login" switch.
    pub const OPEN_AT_LOGIN: isize = 6;
    /// The app main menu's "About Urahafu" item (`crate::platform::app_menu`) — this window no
    /// longer builds an About button itself (DESIGN.md "Settings window": About stays in the app
    /// menu), but the tag stays reserved so the two never collide.
    pub const ABOUT: isize = 7;
    /// The "Clean Screen"/"Clean Keyboard" button.
    pub const CLEAN: isize = 8;
    /// A hidden, zero-size button whose only job is to give the window an Escape key equivalent
    /// (plain `NSWindow`s have none by default); its action is handled exactly like the window's
    /// close button ([`crate::platform::ffi::action_target::ActionId::Close`]).
    pub const ESCAPE_CLOSE: isize = 9;
    /// The white color swatch (11 is `crate::platform::app_menu::tag::QUIT`; this continues right
    /// after it since that module's own tags come right after this one's original range).
    pub const COLOR_WHITE: isize = 12;
}

/// The auto-unlock segmented control's segments, in the order they're added — index -> value.
const FAILSAFE_ORDER: [FailsafeDelay; 3] = [
    FailsafeDelay::Seconds30,
    FailsafeDelay::Seconds60,
    FailsafeDelay::Seconds90,
];

/// The settings window, built once by [`SettingsWindow::new`] and shown/hidden for the app's
/// whole lifetime (`releasedWhenClosed = false`, `docs/ARCHITECTURE.md`-style: the window is
/// never actually closed by `AppKit`, only ordered out).
pub struct SettingsWindow {
    window: Retained<NSWindow>,
    /// The window's delegate and every control's target — kept alive for as long as the
    /// window is, and reused by `crate::platform::app_menu` for the app main menu's items.
    target: Retained<ActionTarget>,
    /// Whether [`SettingsWindow::show`] has centered the window yet (only done once — DESIGN.md
    /// "Settings window" › "Layout": "centered on first show").
    centered: Cell<bool>,
    /// The single top-level vertical stack; [`SettingsWindow::resize_to_fit`] reads its
    /// `fittingSize` after every visibility change.
    main_stack: Retained<NSStackView>,
    banner: Retained<NSView>,
    color_black_ring: Retained<NSBox>,
    color_white_ring: Retained<NSBox>,
    failsafe_segmented: Retained<NSSegmentedControl>,
    keyboard_only_switch: Retained<NSSwitch>,
    show_icon_switch: Retained<NSSwitch>,
    open_at_login_switch: Retained<NSSwitch>,
    open_at_login_label: Retained<NSTextField>,
    open_at_login_note: Retained<NSTextField>,
    /// The Clean button's wrapper (its own `NSBox` fill and the button itself are overlaid on
    /// it); its `alphaValue` is what shows the "disabled" ~40% opacity.
    clean_container: Retained<NSView>,
    clean_button: Retained<NSButton>,
}

/// A plain, fixed-height control frame at the origin; every control here is laid out by its
/// parent `NSStackView` or by explicit constraints, so the frame's origin never matters, only a
/// sane starting size before Auto Layout takes over.
fn frame(width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height))
}

fn label(text: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    NSTextField::labelWithString(&NSString::from_str(text), mtm)
}

/// A wrapping label styled with an explicit point size and color, capped to `max_width` via both
/// `preferredMaxLayoutWidth` (so it reports the right wrapped height) and an explicit width
/// constraint (so it can never grow past its available space — DESIGN.md "Settings window" ›
/// "Layout": "labels that wrap get preferredMaxLayoutWidth / a width constraint equal to the
/// available width").
fn wrapping_label(
    text: &str,
    size: f64,
    color: &NSColor,
    max_width: f64,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    field.setFont(Some(&NSFont::systemFontOfSize(size)));
    field.setTextColor(Some(color));
    field.setPreferredMaxLayoutWidth(max_width);
    pin_width(&field, max_width);
    field
}

/// The accent teal used for the Clean button's fill and a swatch's selection ring (DESIGN.md §4:
/// `#22B8C8`) — a fixed value, not a dynamic system color, so it looks the same in light and dark
/// mode by design and never needs to be re-applied on an appearance change.
fn accent_teal() -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        f64::from(0x22) / 255.0,
        f64::from(0xB8) / 255.0,
        f64::from(0xC8) / 255.0,
        1.0,
    )
}

/// Pins `view`'s width to exactly `width` points.
fn pin_width(view: &NSView, width: f64) {
    view.widthAnchor()
        .constraintEqualToConstant(width)
        .setActive(true);
}

/// Pins `view`'s height to exactly `height` points.
fn pin_height(view: &NSView, height: f64) {
    view.heightAnchor()
        .constraintEqualToConstant(height)
        .setActive(true);
}

/// Pins both of `view`'s dimensions.
fn pin_size(view: &NSView, width: f64, height: f64) {
    pin_width(view, width);
    pin_height(view, height);
}

/// Pins `view`'s four edges to exactly match `container`'s — used to overlay a full-size click
/// target or fill exactly over its wrapper (the Clean button's box and button).
fn pin_edges(view: &NSView, container: &NSView) {
    view.leadingAnchor()
        .constraintEqualToAnchor(&container.leadingAnchor())
        .setActive(true);
    view.trailingAnchor()
        .constraintEqualToAnchor(&container.trailingAnchor())
        .setActive(true);
    view.topAnchor()
        .constraintEqualToAnchor(&container.topAnchor())
        .setActive(true);
    view.bottomAnchor()
        .constraintEqualToAnchor(&container.bottomAnchor())
        .setActive(true);
}

/// Centers `view` within `container` on both axes — used to stack a swatch's ring/fill/button
/// concentrically.
fn pin_center(view: &NSView, container: &NSView) {
    view.centerXAnchor()
        .constraintEqualToAnchor(&container.centerXAnchor())
        .setActive(true);
    view.centerYAnchor()
        .constraintEqualToAnchor(&container.centerYAnchor())
        .setActive(true);
}

/// Adds `view` as a plain (non-arranged) subview of `container`, switches it to Auto Layout, and
/// pins it to `container`'s exact size, centered.
fn overlay_centered(container: &NSView, view: &NSView, size: f64) {
    container.addSubview(view);
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    pin_size(view, size, size);
    pin_center(view, container);
}

/// Adds `view` as a plain (non-arranged) subview of `container`, switches it to Auto Layout, and
/// pins its edges to exactly match `container`'s.
fn overlay_filling(container: &NSView, view: &NSView) {
    container.addSubview(view);
    view.setTranslatesAutoresizingMaskIntoConstraints(false);
    pin_edges(view, container);
}

/// A horizontal `[leading, flexible spacer, trailing]` row, `width` points wide with [`ROW_INSET`]
/// horizontal padding and (if given) a fixed height — the shape every group-box row and the
/// banner's button row share. The spacer's very low content-hugging priority is what lets it
/// absorb the row's slack and push `trailing` to the row's right edge; that only works because the
/// row itself is pinned to a known `width` (an unconstrained row simply collapses to its content's
/// minimum size instead of filling the window — the previous layout's bug).
fn edge_row(
    leading: &NSView,
    trailing: &NSView,
    width: f64,
    height: Option<f64>,
    padded: bool,
    mtm: MainThreadMarker,
) -> Retained<NSStackView> {
    let spacer = NSView::initWithFrame(NSView::alloc(mtm), frame(1.0, 1.0));
    spacer.setContentHuggingPriority_forOrientation(1.0, NSLayoutConstraintOrientation::Horizontal);
    let views: [&NSView; 3] = [leading, &spacer, trailing];
    let row = NSStackView::stackViewWithViews(&NSArray::from_slice(&views), mtm);
    row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
    row.setAlignment(NSLayoutAttribute::CenterY);
    row.setSpacing(ROW_STACK_SPACING);
    if padded {
        row.setEdgeInsets(NSEdgeInsets {
            top: 0.0,
            left: ROW_INSET,
            bottom: 0.0,
            right: ROW_INSET,
        });
    }
    pin_width(&row, width);
    if let Some(height) = height {
        pin_height(&row, height);
    } else if padded {
        // A group-box row whose height can grow (a second line appearing under its label) still
        // never gets shorter than its fixed-height siblings.
        row.heightAnchor()
            .constraintGreaterThanOrEqualToConstant(ROW_HEIGHT)
            .setActive(true);
        row.setEdgeInsets(NSEdgeInsets {
            top: 10.0,
            left: ROW_INSET,
            bottom: 10.0,
            right: ROW_INSET,
        });
    }
    row
}

/// A group-box row's leading label: wraps within whatever width `trailing` (the row's control)
/// leaves free, so a long translation (Finnish "Automaattinen lukituksen poisto" next to the
/// 30/60/90 s selector) goes onto a second line instead of running under the control.
fn row_label(text: &str, trailing: &NSView, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let available = ROW_INNER_WIDTH - trailing.fittingSize().width - ROW_CONTROL_GAP;
    wrapping_label(text, 13.0, &NSColor::labelColor(), available, mtm)
}

/// A group-box row: `leading` (a label, or a small vertical label+note stack) on the left, `trailing`
/// (a control) on the right, [`ROW_INSET`] padding, `height` if the row should be a fixed
/// [`ROW_HEIGHT`] (omitted for the "Open at Login" row, whose height must grow when its "needs
/// icon" note becomes visible).
fn box_row(
    leading: &NSView,
    trailing: &NSView,
    height: Option<f64>,
    mtm: MainThreadMarker,
) -> Retained<NSStackView> {
    edge_row(leading, trailing, CONTENT_WIDTH, height, true, mtm)
}

/// A hairline divider between two group-box rows: an `NSBox` of `boxType: .separator`, which
/// draws itself using the dynamic `separatorColor` (correct in light and dark automatically, no
/// re-apply logic needed) — inset by [`ROW_INSET`] on each side to line up with the rows' own
/// padding.
fn hairline_separator(mtm: MainThreadMarker) -> Retained<NSBox> {
    let separator = NSBox::initWithFrame(NSBox::alloc(mtm), frame(ROW_INNER_WIDTH, 1.0));
    separator.setBoxType(NSBoxType::Separator);
    pin_width(&separator, ROW_INNER_WIDTH);
    pin_height(&separator, 1.0);
    separator
}

/// A rounded box behind `content`: a plain container view holding an `NSBox` (custom type, no
/// border, `fill`, rounded corners) as a background, with `content` pinned on top of it to all four
/// edges. `content` (a stack) is what gives the container its size. The fill is deliberately NOT
/// the `NSBox`'s own `contentView`: an `NSBox` resizes its content view with autoresizing masks and
/// never derives its own size from it, so a box sized that way collapses to zero height and its
/// rows spill over the rest of the window.
fn rounded_container(content: &NSView, fill: &NSColor, mtm: MainThreadMarker) -> Retained<NSView> {
    let container = NSView::initWithFrame(NSView::alloc(mtm), frame(CONTENT_WIDTH, 0.0));
    let background = NSBox::initWithFrame(NSBox::alloc(mtm), frame(CONTENT_WIDTH, 0.0));
    background.setBoxType(NSBoxType::Custom);
    background.setBorderWidth(0.0);
    background.setCornerRadius(CORNER_RADIUS);
    background.setFillColor(fill);
    background.setContentViewMargins(NSSize::new(0.0, 0.0));
    overlay_filling(&container, &background);
    overlay_filling(&container, content);
    container
}

/// A rounded "group box" (System Settings-style): a vertical stack of `rows` (already including
/// any [`hairline_separator`]s between them, in order) on a dynamic fill that is correct in light
/// and dark mode with no re-apply logic.
fn group_box(rows: &[&NSView], mtm: MainThreadMarker) -> Retained<NSView> {
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(rows), mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setAlignment(NSLayoutAttribute::CenterX);
    stack.setSpacing(0.0);
    pin_width(&stack, CONTENT_WIDTH);
    rounded_container(&stack, &NSColor::quaternarySystemFillColor(), mtm)
}

/// The permission banner: a [`rounded_container`] (systemOrange at ~15% alpha — dynamic, correct in dark mode)
/// containing `text` and, below it, `button` right-aligned via [`edge_row`].
fn banner_box(text: &NSTextField, button: &NSButton, mtm: MainThreadMarker) -> Retained<NSView> {
    let button_row = edge_row(
        &NSView::initWithFrame(NSView::alloc(mtm), frame(0.0, 0.0)),
        button,
        ROW_INNER_WIDTH,
        None,
        false,
        mtm,
    );
    let views: [&NSView; 2] = [text, &button_row];
    let stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&views), mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setAlignment(NSLayoutAttribute::Leading);
    stack.setSpacing(8.0);
    stack.setEdgeInsets(NSEdgeInsets {
        top: ROW_INSET,
        left: ROW_INSET,
        bottom: ROW_INSET,
        right: ROW_INSET,
    });
    pin_width(&stack, CONTENT_WIDTH);
    rounded_container(
        &stack,
        &NSColor::systemOrangeColor().colorWithAlphaComponent(0.15),
        mtm,
    )
}

/// One clickable color swatch: a filled, rounded square (`fill`) with a hairline border, a
/// slightly larger ring (hidden unless selected — [`SettingsWindow::update`] toggles it) showing
/// the accent selection outline, and a borderless button covering the whole thing for the click
/// target and keyboard focus (an `NSBox` isn't an `NSControl`, so it can't be one itself). Returns
/// `(container, ring)` — the container is what goes in the color row, the ring is what `update`
/// shows/hides.
fn color_swatch(
    fill: &NSColor,
    accessibility_label: &str,
    tag: isize,
    target: &ActionTarget,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Retained<NSBox>) {
    let container = NSView::initWithFrame(
        NSView::alloc(mtm),
        frame(SWATCH_RING_SIZE, SWATCH_RING_SIZE),
    );
    pin_size(&container, SWATCH_RING_SIZE, SWATCH_RING_SIZE);

    let ring = NSBox::initWithFrame(NSBox::alloc(mtm), frame(SWATCH_RING_SIZE, SWATCH_RING_SIZE));
    ring.setBoxType(NSBoxType::Custom);
    ring.setBorderWidth(SWATCH_RING_STROKE);
    ring.setBorderColor(&accent_teal());
    ring.setCornerRadius(SWATCH_CORNER_RADIUS + SWATCH_RING_GAP + SWATCH_RING_STROKE);
    ring.setFillColor(&NSColor::clearColor());
    ring.setContentViewMargins(NSSize::new(0.0, 0.0));
    ring.setHidden(true);
    overlay_centered(&container, &ring, SWATCH_RING_SIZE);

    let swatch = NSBox::initWithFrame(NSBox::alloc(mtm), frame(SWATCH_SIZE, SWATCH_SIZE));
    swatch.setBoxType(NSBoxType::Custom);
    swatch.setBorderWidth(1.0);
    swatch.setBorderColor(&NSColor::separatorColor());
    swatch.setCornerRadius(SWATCH_CORNER_RADIUS);
    swatch.setFillColor(fill);
    swatch.setContentViewMargins(NSSize::new(0.0, 0.0));
    overlay_centered(&container, &swatch, SWATCH_SIZE);

    let button = NSButton::initWithFrame(
        NSButton::alloc(mtm),
        frame(SWATCH_RING_SIZE, SWATCH_RING_SIZE),
    );
    button.setBordered(false);
    button.setTitle(&NSString::from_str(""));
    button.setTag(tag);
    action_target::wire_control(&button, target);
    button.setAccessibilityLabel(Some(&NSString::from_str(accessibility_label)));
    overlay_centered(&container, &button, SWATCH_RING_SIZE);

    (container, ring)
}

/// The Clean button: a borderless, white-titled `NSButton` overlaid on a teal-filled `NSBox` (a
/// bordered `NSButton` can't take an arbitrary flat fill color and corner radius; an `NSBox` isn't
/// a control) — full content width, [`CLEAN_BUTTON_HEIGHT`] tall, Return as its key equivalent.
/// Returns `(container, button)`; [`SettingsWindow::update`] toggles the container's `alphaValue`
/// for the disabled ~40%-opacity look and the button's own `isEnabled` to make it unclickable.
fn clean_button(
    target: &ActionTarget,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Retained<NSButton>) {
    let container = NSView::initWithFrame(
        NSView::alloc(mtm),
        frame(CONTENT_WIDTH, CLEAN_BUTTON_HEIGHT),
    );
    pin_size(&container, CONTENT_WIDTH, CLEAN_BUTTON_HEIGHT);

    let fill = NSBox::initWithFrame(NSBox::alloc(mtm), frame(CONTENT_WIDTH, CLEAN_BUTTON_HEIGHT));
    fill.setBoxType(NSBoxType::Custom);
    fill.setBorderWidth(0.0);
    fill.setCornerRadius(CORNER_RADIUS);
    fill.setFillColor(&accent_teal());
    fill.setContentViewMargins(NSSize::new(0.0, 0.0));
    overlay_filling(&container, &fill);

    let button = NSButton::initWithFrame(
        NSButton::alloc(mtm),
        frame(CONTENT_WIDTH, CLEAN_BUTTON_HEIGHT),
    );
    button.setBordered(false);
    button.setFont(Some(&NSFont::systemFontOfSize_weight(
        14.0,
        FONT_WEIGHT_SEMIBOLD,
    )));
    button.setContentTintColor(Some(&NSColor::whiteColor()));
    button.setKeyEquivalent(&NSString::from_str("\r"));
    button.setTag(tag::CLEAN);
    action_target::wire_control(&button, target);
    overlay_filling(&container, &button);

    (container, button)
}

impl SettingsWindow {
    /// Builds the window and every control in it (DESIGN.md "Settings window" › "Layout").
    /// `on_action` is called for every decoded [`ActionId`] — control actions, the close
    /// button/Cmd-W, and the Dock/Finder "reopen" Apple Event, whose handler this also registers.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "constructing every control in the layout's fixed, ordered structure is \
                  inherently a long, linear sequence (mirrors platform::tray::Tray::new, which \
                  carries the same allow for the same reason)"
    )]
    pub fn new(
        mtm: MainThreadMarker,
        language: Language,
        on_action: impl Fn(ActionId) + 'static,
    ) -> Self {
        let target = ActionTarget::new(mtm, on_action);
        action_target::register_reopen_handler(&target);

        // 1. Header: app icon + title/tagline.
        let icon_image = NSApplication::sharedApplication(mtm).applicationIconImage();
        let tagline_max_width = if icon_image.is_some() {
            CONTENT_WIDTH - HEADER_ICON_SIZE - HEADER_SPACING
        } else {
            CONTENT_WIDTH
        };
        let title_label = label("Urahafu", mtm);
        title_label.setFont(Some(&NSFont::systemFontOfSize_weight(
            15.0,
            FONT_WEIGHT_SEMIBOLD,
        )));
        let tagline_label = wrapping_label(
            Text::AboutTagline.get(language),
            12.0,
            &NSColor::secondaryLabelColor(),
            tagline_max_width,
            mtm,
        );
        let title_views: [&NSView; 2] = [&title_label, &tagline_label];
        let title_stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&title_views), mtm);
        title_stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        title_stack.setAlignment(NSLayoutAttribute::Leading);
        title_stack.setSpacing(2.0);

        let header: Retained<NSStackView> = if let Some(icon) = icon_image {
            let image_view = NSImageView::imageViewWithImage(&icon, mtm);
            image_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            pin_size(&image_view, HEADER_ICON_SIZE, HEADER_ICON_SIZE);
            let header_views: [&NSView; 2] = [&image_view, &title_stack];
            let row = NSStackView::stackViewWithViews(&NSArray::from_slice(&header_views), mtm);
            row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
            row.setAlignment(NSLayoutAttribute::CenterY);
            row.setSpacing(HEADER_SPACING);
            row
        } else {
            title_stack
        };

        // 2. Permission banner (hidden by default; `update` shows/hides it).
        let banner_text = wrapping_label(
            Text::WindowPermissionText.get(language),
            12.0,
            &NSColor::labelColor(),
            ROW_INNER_WIDTH,
            mtm,
        );
        let banner_button = NSButton::initWithFrame(NSButton::alloc(mtm), frame(180.0, 24.0));
        banner_button.setBezelStyle(NSBezelStyle::Push);
        banner_button.setTitle(&NSString::from_str(
            Text::FirstLaunchOpenSettings.get(language),
        ));
        banner_button.setTag(tag::GRANT_ACCESS);
        action_target::wire_control(&banner_button, &target);
        let banner = banner_box(&banner_text, &banner_button, mtm);
        banner.setHidden(true);

        // 3. Group 1: Color, Auto-Unlock.
        let (color_black_container, color_black_ring) = color_swatch(
            &NSColor::blackColor(),
            Text::MenuColorBlack.get(language),
            tag::COLOR_BLACK,
            &target,
            mtm,
        );
        let (color_white_container, color_white_ring) = color_swatch(
            &NSColor::whiteColor(),
            Text::MenuColorWhite.get(language),
            tag::COLOR_WHITE,
            &target,
            mtm,
        );
        let swatch_views: [&NSView; 2] = [&color_black_container, &color_white_container];
        let swatches_row =
            NSStackView::stackViewWithViews(&NSArray::from_slice(&swatch_views), mtm);
        swatches_row.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        swatches_row.setAlignment(NSLayoutAttribute::CenterY);
        swatches_row.setSpacing(8.0);
        let color_label = row_label(Text::MenuColor.get(language), &swatches_row, mtm);
        let color_row = box_row(&color_label, &swatches_row, None, mtm);

        let failsafe_segmented =
            NSSegmentedControl::initWithFrame(NSSegmentedControl::alloc(mtm), frame(180.0, 24.0));
        failsafe_segmented.setSegmentCount(isize::try_from(FAILSAFE_ORDER.len()).unwrap_or(0));
        for (index, delay) in FAILSAFE_ORDER.iter().enumerate() {
            let segment_label =
                Text::WindowSeconds.format(language, &[("seconds", &delay.seconds().to_string())]);
            failsafe_segmented.setLabel_forSegment(
                &NSString::from_str(&segment_label),
                isize::try_from(index).unwrap_or(0),
            );
        }
        failsafe_segmented.setTag(tag::FAILSAFE);
        action_target::wire_control(&failsafe_segmented, &target);
        let failsafe_label =
            row_label(Text::MenuAutoUnlock.get(language), &failsafe_segmented, mtm);
        let failsafe_row = box_row(&failsafe_label, &failsafe_segmented, None, mtm);

        let group1_separator = hairline_separator(mtm);
        let group1_rows: [&NSView; 3] = [&color_row, &group1_separator, &failsafe_row];
        let group1_box = group_box(&group1_rows, mtm);

        // 4. Group 2: Keyboard-Only Mode, Show Icon in Menu Bar, Open at Login.
        let keyboard_only_switch = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), frame(38.0, 22.0));
        keyboard_only_switch.setControlSize(NSControlSize::Small);
        keyboard_only_switch.setTag(tag::KEYBOARD_ONLY);
        action_target::wire_control(&keyboard_only_switch, &target);
        let keyboard_only_label = row_label(
            Text::MenuKeyboardOnlyMode.get(language),
            &keyboard_only_switch,
            mtm,
        );
        let keyboard_only_row = box_row(&keyboard_only_label, &keyboard_only_switch, None, mtm);

        let show_icon_switch = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), frame(38.0, 22.0));
        show_icon_switch.setControlSize(NSControlSize::Small);
        show_icon_switch.setTag(tag::SHOW_ICON);
        action_target::wire_control(&show_icon_switch, &target);
        let show_icon_label = row_label(Text::MenuShowIcon.get(language), &show_icon_switch, mtm);
        let show_icon_row = box_row(&show_icon_label, &show_icon_switch, None, mtm);

        let open_at_login_switch = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), frame(38.0, 22.0));
        open_at_login_switch.setControlSize(NSControlSize::Small);
        open_at_login_switch.setTag(tag::OPEN_AT_LOGIN);
        action_target::wire_control(&open_at_login_switch, &target);
        let open_at_login_label = row_label(
            Text::MenuOpenAtLogin.get(language),
            &open_at_login_switch,
            mtm,
        );
        // The note shares its row with the switch: leave the switch and a gap beside it, or the
        // pinned note width would push the switch out of the row.
        let open_at_login_note = wrapping_label(
            Text::WindowOpenAtLoginNeedsIcon.get(language),
            11.0,
            &NSColor::tertiaryLabelColor(),
            ROW_INNER_WIDTH - SWITCH_COLUMN_WIDTH,
            mtm,
        );
        open_at_login_note.setHidden(true);
        let open_at_login_leading_views: [&NSView; 2] = [&open_at_login_label, &open_at_login_note];
        let open_at_login_leading = NSStackView::stackViewWithViews(
            &NSArray::from_slice(&open_at_login_leading_views),
            mtm,
        );
        open_at_login_leading.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        open_at_login_leading.setAlignment(NSLayoutAttribute::Leading);
        open_at_login_leading.setSpacing(2.0);
        let open_at_login_row = box_row(&open_at_login_leading, &open_at_login_switch, None, mtm);

        let group2_separator_1 = hairline_separator(mtm);
        let group2_separator_2 = hairline_separator(mtm);
        let group2_rows: [&NSView; 5] = [
            &keyboard_only_row,
            &group2_separator_1,
            &show_icon_row,
            &group2_separator_2,
            &open_at_login_row,
        ];
        let group2_box = group_box(&group2_rows, mtm);

        // 5. Help.
        let unlock_explanation = wrapping_label(
            Text::CountdownUnlockExplanation.get(language),
            11.0,
            &NSColor::secondaryLabelColor(),
            CONTENT_WIDTH,
            mtm,
        );
        let pixel_test_hint = wrapping_label(
            Text::WindowPixelTestHint.get(language),
            11.0,
            &NSColor::secondaryLabelColor(),
            CONTENT_WIDTH,
            mtm,
        );

        // 6. Clean button.
        let (clean_container, clean_button) = clean_button(&target, mtm);

        // A hidden, zero-size button giving the window an Escape key equivalent (DESIGN.md
        // "Settings window" › "Closing the window": Esc closes it, like the red button/Cmd-W).
        let escape_button = NSButton::initWithFrame(NSButton::alloc(mtm), frame(0.0, 0.0));
        escape_button.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        escape_button.setTag(tag::ESCAPE_CLOSE);
        escape_button.setHidden(true);
        action_target::wire_control(&escape_button, &target);

        // Assemble the main vertical stack.
        let main_views: [&NSView; 6] = [
            &header,
            &banner,
            &group1_box,
            &group2_box,
            &unlock_explanation,
            &pixel_test_hint,
        ];
        let main_stack = NSStackView::stackViewWithViews(&NSArray::from_slice(&main_views), mtm);
        main_stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        main_stack.setAlignment(NSLayoutAttribute::Leading);
        main_stack.setSpacing(SPACING);
        main_stack.setCustomSpacing_afterView(6.0, &unlock_explanation);
        main_stack.addArrangedSubview(&clean_container);
        main_stack.addArrangedSubview(&escape_button);
        main_stack.setEdgeInsets(NSEdgeInsets {
            top: MARGIN,
            left: MARGIN,
            bottom: MARGIN,
            right: MARGIN,
        });

        main_stack.layoutSubtreeIfNeeded();
        let content_size = main_stack.fittingSize();

        let window = action_target::new_settings_window(mtm, content_size, "Urahafu");
        window.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*target)));
        if let Some(content_view) = window.contentView() {
            content_view.addSubview(&main_stack);
            main_stack.setTranslatesAutoresizingMaskIntoConstraints(false);
            pin_edges(&main_stack, &content_view);
            if language.is_rtl() {
                content_view
                    .setUserInterfaceLayoutDirection(NSUserInterfaceLayoutDirection::RightToLeft);
            }
        }

        Self {
            window,
            target,
            centered: Cell::new(false),
            main_stack,
            banner,
            color_black_ring,
            color_white_ring,
            failsafe_segmented,
            keyboard_only_switch,
            show_icon_switch,
            open_at_login_switch,
            open_at_login_label,
            open_at_login_note,
            clean_container,
            clean_button,
        }
    }

    /// Shows the window, activating the app (needed for an accessory app to reliably come to the
    /// front) and centering it the first time only.
    pub fn show(&self, mtm: MainThreadMarker) {
        if !self.centered.replace(true) {
            self.window.center();
        }
        NSApplication::sharedApplication(mtm).activate();
        self.window.makeKeyAndOrderFront(None);
    }

    /// Hides the window without closing it (`orderOut:`) — it stays alive, ready to be shown
    /// again.
    pub fn hide(&self) {
        self.window.orderOut(None);
    }

    /// Whether the window is currently visible.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.window.isVisible()
    }

    /// This window's own [`ActionTarget`] — reused by the app main menu
    /// (`crate::platform::app_menu`) for the items it routes through the same target/action
    /// mechanism as this window's controls, rather than building a second instance.
    #[must_use]
    pub fn action_target(&self) -> &ActionTarget {
        &self.target
    }

    /// Refreshes every control from `settings`/`login_enabled`/`trusted`: the selected color's
    /// ring, the auto-unlock segment, the switches, the permission banner's visibility, the "Open
    /// at Login" switch's enabled state and note, and the Clean button's label/enabled state —
    /// then resizes the window to fit (DESIGN.md "Settings window" › "Layout": every visibility
    /// change is followed by a resize that keeps the window's top-left corner fixed).
    pub fn update(
        &self,
        settings: &Settings,
        login_enabled: bool,
        trusted: bool,
        language: Language,
    ) {
        self.color_black_ring
            .setHidden(settings.color != CleaningColor::Black);
        self.color_white_ring
            .setHidden(settings.color != CleaningColor::White);

        let failsafe_index = FAILSAFE_ORDER
            .iter()
            .position(|&f| f == settings.failsafe)
            .unwrap_or(0);
        self.failsafe_segmented
            .setSelectedSegment(isize::try_from(failsafe_index).unwrap_or(0));

        set_switch(&self.keyboard_only_switch, settings.keyboard_only);
        set_switch(&self.show_icon_switch, settings.show_menu_bar_icon);
        set_switch(&self.open_at_login_switch, login_enabled);
        self.open_at_login_switch
            .setEnabled(settings.show_menu_bar_icon);
        self.open_at_login_note
            .setHidden(settings.show_menu_bar_icon);
        let open_at_login_color = if settings.show_menu_bar_icon {
            NSColor::labelColor()
        } else {
            NSColor::tertiaryLabelColor()
        };
        self.open_at_login_label
            .setTextColor(Some(&open_at_login_color));

        self.banner.setHidden(trusted);
        self.clean_button.setEnabled(trusted);
        self.clean_container.setAlphaValue(if trusted {
            1.0
        } else {
            CLEAN_BUTTON_DISABLED_ALPHA
        });
        let clean_label = if settings.keyboard_only {
            Text::MenuCleanKeyboard.get(language)
        } else {
            Text::MenuCleanScreen.get(language)
        };
        self.clean_button.setTitle(&NSString::from_str(clean_label));

        self.resize_to_fit();
    }

    /// Resizes the window to `main_stack`'s fitting size, keeping its top-left corner fixed
    /// (DESIGN.md "Settings window" › "Layout") — called after every visibility change
    /// ([`SettingsWindow::update`]).
    fn resize_to_fit(&self) {
        self.main_stack.layoutSubtreeIfNeeded();
        let fitting = self.main_stack.fittingSize();
        let old_frame = self.window.frame();
        let top_left = NSPoint::new(
            old_frame.origin.x,
            old_frame.origin.y + old_frame.size.height,
        );
        self.window.setContentSize(fitting);
        self.window.setFrameTopLeftPoint(top_left);
    }

    /// The auto-unlock segmented control's current selection.
    #[must_use]
    pub fn selected_failsafe(&self) -> FailsafeDelay {
        let index = usize::try_from(self.failsafe_segmented.selectedSegment()).unwrap_or(0);
        FAILSAFE_ORDER
            .get(index)
            .copied()
            .unwrap_or(FailsafeDelay::Seconds60)
    }

    /// Whether the "Keyboard-Only Mode" switch is currently on.
    #[must_use]
    pub fn keyboard_only_checked(&self) -> bool {
        switch_checked(&self.keyboard_only_switch)
    }

    /// Whether the "Show Icon in Menu Bar" switch is currently on.
    #[must_use]
    pub fn show_icon_checked(&self) -> bool {
        switch_checked(&self.show_icon_switch)
    }

    /// Whether the "Open at Login" switch is currently on.
    #[must_use]
    pub fn open_at_login_checked(&self) -> bool {
        switch_checked(&self.open_at_login_switch)
    }
}

fn set_switch(switch: &NSSwitch, checked: bool) {
    switch.setState(if checked {
        NSControlStateValueOn
    } else {
        NSControlStateValueOff
    });
}

fn switch_checked(switch: &NSSwitch) -> bool {
    switch.state() == NSControlStateValueOn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failsafe_order_is_ascending() {
        assert_eq!(FAILSAFE_ORDER[0], FailsafeDelay::Seconds30);
        assert_eq!(FAILSAFE_ORDER[1], FailsafeDelay::Seconds60);
        assert_eq!(FAILSAFE_ORDER[2], FailsafeDelay::Seconds90);
    }

    #[test]
    fn tags_are_all_distinct() {
        let tags = [
            tag::GRANT_ACCESS,
            tag::COLOR_BLACK,
            tag::FAILSAFE,
            tag::KEYBOARD_ONLY,
            tag::SHOW_ICON,
            tag::OPEN_AT_LOGIN,
            tag::ABOUT,
            tag::CLEAN,
            tag::ESCAPE_CLOSE,
            tag::COLOR_WHITE,
        ];
        for (i, a) in tags.iter().enumerate() {
            for b in &tags[i + 1..] {
                assert_ne!(a, b, "duplicate tag value {a}");
            }
        }
    }
}
