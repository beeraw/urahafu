//! Raw CoreText / CoreGraphics calls: the system UI font, weight variants, tabular figures,
//! letter-spacing (tracking) and single-line alpha-mask rasterization (DESIGN.md §5).
//!
//! This is the one module in the crate allowed to contain `unsafe` (`docs/ARCHITECTURE.md`
//! goal 4; the crate root has `#![deny(unsafe_code)]`). Every `unsafe` block below carries a
//! `// SAFETY:` comment. Where the `core-text`/`core-graphics`/`core-foundation` crates expose a
//! safe wrapper for an operation, it is used; raw FFI (still routed through those crates' typed
//! `CFStringRef`/`CFDictionaryRef` etc. constants and functions, never hand-rolled bindings) is
//! used only where no safe wrapper exists — notably font-descriptor attribute layering, which
//! the `core-text` crate exposes only for a bare, untyped `CFDictionary`.
//!
//! No SF Symbols, no embedded or on-disk font file (DESIGN.md §1, §5): every glyph comes from
//! `CTFontCreateUIFontForLanguage`, the public, stable system-font API.

#![allow(
    unsafe_code,
    reason = "this is the crate's designated FFI module (docs/ARCHITECTURE.md goal 4); the \
              crate root denies unsafe_code everywhere else"
)]

use core_foundation::attributed_string::CFMutableAttributedString;
use core_foundation::base::{CFRange, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::base::kCGImageAlphaNone;
use core_graphics::color::CGColor;
use core_graphics::color_space::CGColorSpace;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_text::font::{self as ct_font, CTFont};
use core_text::font_descriptor::{
    self as ct_desc, CTFontDescriptor, kCTFontTraitsAttribute, kCTFontWeightTrait,
};
use core_text::line::CTLine;
use core_text::string_attributes::{
    kCTFontAttributeName, kCTForegroundColorAttributeName, kCTKernAttributeName,
};

/// AAT `kNumberSpacingType` feature type (see `SFNTLayoutTypes.h`): selects figure spacing.
const AAT_NUMBER_SPACING_TYPE: i32 = 6;
/// AAT `kMonospacedNumbersSelector`: fixed-width (tabular) digits, as opposed to proportional.
const AAT_MONOSPACED_NUMBERS_SELECTOR: i32 = 0;
/// Extra padding (in pixels, at the rasterization scale) added around a rasterized line so
/// anti-aliased edges and kerning/tracking overshoot are never clipped by the mask bounds.
const MASK_PADDING_PX: f64 = 2.0;

/// System UI font weight variants used across the overlay (DESIGN.md §5: Light, Regular,
/// Medium, Semibold).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontWeight {
    /// Countdown digit weight.
    Light,
    /// Hint, second line, HUD secondary text weight.
    Regular,
    /// Wordmark weight.
    Medium,
    /// The "urahafu" run inside the hint, and HUD primary label weight.
    Semibold,
}

impl FontWeight {
    /// The normalized CoreText/AppKit weight trait value (`kCTFontWeightTrait`, range roughly
    /// `[-1.0, 1.0]`), matching the values Apple documents for the equivalent `NSFontWeight*`
    /// constants. There is no dedicated "light/medium/semibold system UI font" type, so weight
    /// variants are obtained by layering this trait onto the regular system font's descriptor.
    fn trait_value(self) -> f64 {
        match self {
            Self::Light => -0.4,
            Self::Regular => 0.0,
            Self::Medium => 0.23,
            Self::Semibold => 0.3,
        }
    }
}

/// One run of text sharing a weight, tracking and tabular-figures setting, used to lay out lines
/// with mixed runs on a single baseline (e.g. hint text in Regular with the word "urahafu" in
/// Semibold, DESIGN.md §8).
#[derive(Debug, Clone)]
pub struct TextRun {
    /// The run's text. ASCII/Latin-1 content only is exercised by this app's strings (the
    /// French accents in e.g. "Échap" are single UTF-16 code units, which is all the character
    /// counting below assumes).
    pub text: String,
    /// This run's font weight.
    pub weight: FontWeight,
    /// Extra tracking (letter-spacing) for this run, in em units of the line's font size (e.g.
    /// `0.05` = 5% of the font size added after each glyph).
    pub tracking_em: f32,
    /// Whether this run's digits use tabular (fixed-width) figures (DESIGN.md §5: countdown and
    /// time displays).
    pub tabular_numbers: bool,
}

impl TextRun {
    /// Convenience constructor for a plain run with no tracking and no tabular figures.
    #[must_use]
    pub fn plain(text: impl Into<String>, weight: FontWeight) -> Self {
        Self {
            text: text.into(),
            weight,
            tracking_em: 0.0,
            tabular_numbers: false,
        }
    }
}

/// An 8-bit coverage mask for one rasterized line of text, in device pixels, plus enough
/// metrics to position it against a baseline anchor.
#[derive(Debug, Clone)]
pub struct RasterizedLine {
    /// Mask width in pixels.
    pub width: u32,
    /// Mask height in pixels.
    pub height: u32,
    /// Distance in pixels from the top of `data` down to the text baseline.
    pub baseline: f32,
    /// 8-bit coverage, one byte per pixel, row-major, top row first (0 = no coverage, 255 =
    /// full coverage) — directly usable with [`crate::core::canvas::Canvas::blit_alpha_mask`].
    pub data: Vec<u8>,
    /// The line's width in points, measured at scale 1.0 regardless of the rasterization scale
    /// passed to [`rasterize`], for layout math that happens in points before any `HiDPI` scaling.
    pub width_points: f32,
}

/// Builds the system UI font at `size_px` (already at the target rasterization scale — CoreText
/// has no separate notion of "device pixels", so scaling is applied by building a larger font
/// rather than via a context transform), `weight`, and optionally with tabular figures.
fn build_font(size_px: f64, weight: FontWeight, tabular_numbers: bool) -> CTFont {
    // SAFETY: `new_ui_font_for_language` wraps `CTFontCreateUIFontForLanguage`, a public
    // CoreText API. `kCTFontSystemFontType` is the library's own valid `CTFontUIFontType`
    // constant, `size_px` is a finite positive value, and `language: None` asks CoreText to use
    // the current locale (always valid). The unsafe FFI call itself lives inside the `core-text`
    // wrapper; nothing unsafe is performed directly in this statement.
    let base = ct_font::new_ui_font_for_language(ct_font::kCTFontSystemFontType, size_px, None);
    let base_desc = base.copy_descriptor();

    let weighted_desc = with_weight_trait(&base_desc, weight);
    let final_desc = if tabular_numbers {
        with_tabular_figures(&weighted_desc)
    } else {
        weighted_desc
    };

    ct_font::new_from_descriptor(&final_desc, size_px)
}

/// Layers `{ kCTFontTraitsAttribute: { kCTFontWeightTrait: weight } }` onto `desc`, the
/// documented way to obtain a non-default weight of the system UI font (there is no
/// "light"/"medium"/"semibold" `CTFontUIFontType`). Falls back to `desc` unchanged if CoreText
/// refuses the attribute copy (defensive: this has not been observed to happen for the system
/// font, but a font-related system call is never assumed infallible).
fn with_weight_trait(desc: &CTFontDescriptor, weight: FontWeight) -> CTFontDescriptor {
    // SAFETY: `kCTFontWeightTrait` and `kCTFontTraitsAttribute` are CoreText framework global
    // `CFStringRef` constants; reading an extern static of a type that is `Copy` (a raw
    // pointer) is safe as long as the framework is linked and loaded, which building against
    // the `core-text`/`core-graphics` crates guarantees for the whole process lifetime.
    let (weight_key, traits_key) = unsafe { (kCTFontWeightTrait, kCTFontTraitsAttribute) };

    let weight_key = unsafe {
        // SAFETY: `weight_key` is a live, permanently-retained framework constant; wrapping it
        // under the "get" rule (which retains it once more for this owned `CFString`) is the
        // documented way to hold a `CFStringRef` constant as a typed, owned CF object.
        CFString::wrap_under_get_rule(weight_key)
    };
    let traits_key = unsafe {
        // SAFETY: same reasoning as `weight_key` above.
        CFString::wrap_under_get_rule(traits_key)
    };

    let weight_number = CFNumber::from(weight.trait_value());
    let traits_dict: CFDictionary<CFString, CFNumber> =
        CFDictionary::from_CFType_pairs(&[(weight_key, weight_number)]);
    let attrs: CFDictionary<CFString, core_foundation::base::CFType> =
        CFDictionary::from_CFType_pairs(&[(traits_key, traits_dict.as_CFType())]);

    // SAFETY: `CTFontDescriptorCreateCopyWithAttributes` is a public CoreText API.
    // `desc.as_concrete_TypeRef()` is a valid, live `CTFontDescriptorRef` owned by `desc`, and
    // `attrs.as_concrete_TypeRef()` is a valid, live `CFDictionaryRef` owned by `attrs`, both
    // outliving this call. The returned pointer follows the CF "create" rule (a new owned
    // reference, or null on failure), which is respected immediately below: wrapped under the
    // create rule on success, and simply not touched (no leak, since it is null) on failure.
    let copy_ref = unsafe {
        ct_desc::CTFontDescriptorCreateCopyWithAttributes(
            desc.as_concrete_TypeRef(),
            attrs.as_concrete_TypeRef(),
        )
    };
    if copy_ref.is_null() {
        desc.clone()
    } else {
        // SAFETY: `copy_ref` was just checked non-null and was returned under the CF "create"
        // rule by the call above, so this wrapper takes ownership of exactly the one reference
        // count we already own, without an extra retain.
        unsafe { CTFontDescriptor::wrap_under_create_rule(copy_ref) }
    }
}

/// Layers the AAT tabular-figures (`kNumberSpacingType` / `kMonospacedNumbersSelector`) feature
/// onto `desc` (DESIGN.md §5: countdown digits and times use fixed-width figures so the display
/// does not "jump" between values). Falls back to `desc` unchanged on failure.
fn with_tabular_figures(desc: &CTFontDescriptor) -> CTFontDescriptor {
    let feature_type = CFNumber::from(AAT_NUMBER_SPACING_TYPE);
    let feature_selector = CFNumber::from(AAT_MONOSPACED_NUMBERS_SELECTOR);

    // SAFETY: `CTFontDescriptorCreateCopyWithFeature` is a public CoreText API; `desc` and the
    // two `CFNumber`s are valid, live CF objects for the duration of this call. The returned
    // pointer follows the CF "create" rule (owned reference, or null on failure), handled the
    // same way as in `with_weight_trait` above.
    let copy_ref = unsafe {
        ct_desc::CTFontDescriptorCreateCopyWithFeature(
            desc.as_concrete_TypeRef(),
            feature_type.as_concrete_TypeRef(),
            feature_selector.as_concrete_TypeRef(),
        )
    };
    if copy_ref.is_null() {
        desc.clone()
    } else {
        // SAFETY: see the matching call in `with_weight_trait`: `copy_ref` is non-null and was
        // returned under the CF "create" rule, so this wrapper owns exactly that one reference.
        unsafe { CTFontDescriptor::wrap_under_create_rule(copy_ref) }
    }
}

/// Builds a [`CTLine`] for `runs` at `font_size_px` (each run's font is built at this exact
/// pixel size — see [`build_font`]), returning the line plus its typographic width/ascent/
/// descent in pixels.
fn build_line(runs: &[TextRun], font_size_px: f64) -> (CTLine, f64, f64, f64) {
    let full_text: String = runs.iter().map(|run| run.text.as_str()).collect();
    let cf_string = CFString::new(&full_text);
    let mut attr_string = CFMutableAttributedString::new();
    attr_string.replace_str(&cf_string, CFRange::init(0, 0));

    let white = CGColor::rgb(1.0, 1.0, 1.0, 1.0);
    let mut offset: isize = 0;
    for run in runs {
        let len = run.text.chars().count();
        #[allow(
            clippy::cast_possible_wrap,
            reason = "run text is a short UI string, far below isize::MAX in character count"
        )]
        let range = CFRange::init(offset, len as isize);

        let font = build_font(font_size_px, run.weight, run.tabular_numbers);
        // SAFETY: `kCTFontAttributeName`/`kCTForegroundColorAttributeName`/`kCTKernAttributeName`
        // are CoreText framework global `CFStringRef` constants (see `with_weight_trait` above
        // for why reading them is safe); `set_attribute` itself is the `core-foundation` crate's
        // safe wrapper around `CFAttributedStringSetAttribute`, which retains `value` internally.
        let (font_key, color_key, kern_key) = unsafe {
            (
                kCTFontAttributeName,
                kCTForegroundColorAttributeName,
                kCTKernAttributeName,
            )
        };
        attr_string.set_attribute(range, font_key, &font);
        attr_string.set_attribute(range, color_key, &white);
        if run.tracking_em.abs() > f32::EPSILON {
            let kern = CFNumber::from(f64::from(run.tracking_em) * font_size_px);
            attr_string.set_attribute(range, kern_key, &kern);
        }

        #[allow(
            clippy::cast_possible_wrap,
            reason = "run text is a short UI string, far below isize::MAX in character count"
        )]
        {
            offset += len as isize;
        }
    }

    let line = CTLine::new_with_attributed_string(attr_string.as_concrete_TypeRef());
    let bounds = line.get_typographic_bounds();
    (line, bounds.width, bounds.ascent, bounds.descent)
}

/// Measures `runs` at `font_size_pt` points (rasterization scale 1.0), returning the line's
/// width in points — used for layout centering math, which happens in points before any `HiDPI`
/// scaling is applied.
#[must_use]
pub fn measure(runs: &[TextRun], font_size_pt: f32) -> f32 {
    let (_, width, _, _) = build_line(runs, f64::from(font_size_pt));
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a single line's width in points is far below f32's precision-loss range for \
                  any real UI text"
    )]
    let width = width as f32;
    width
}

/// Rasterizes `runs` at `font_size_pt` points, `scale` pixels per point (1.0 for a standard
/// display, 2.0 for `HiDPI`), into an 8-bit coverage mask.
///
/// Renders white text on a black background into an alpha-free `DeviceGray` bitmap context: the
/// resulting grayscale byte *is* the coverage value directly (no color, no inversion needed),
/// since the background is solid black (0 = no coverage) and glyphs are solid white (255 = full
/// coverage), with CoreText's own anti-aliasing producing the partial-coverage gray values in
/// between.
#[allow(
    clippy::similar_names,
    reason = "`font_size_pt` (the parameter, in points) and `size_px` (the same size, converted \
              to pixels for the actual rasterization) are deliberately named to mirror each \
              other — that pairing is the point, not an accident to rename away"
)]
#[must_use]
pub fn rasterize(runs: &[TextRun], font_size_pt: f32, scale: f32) -> RasterizedLine {
    let scale = scale.max(0.01);
    let size_px = f64::from(font_size_pt) * f64::from(scale);
    let (line, width, ascent, descent) = build_line(runs, size_px);
    let width_points = measure(runs, font_size_pt);

    let width_px = (width + MASK_PADDING_PX * 2.0).ceil().max(1.0);
    let height_px = (ascent + descent + MASK_PADDING_PX * 2.0).ceil().max(1.0);
    #[allow(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "width_px/height_px are ceil()'d from non-negative typographic metrics, far \
                  below usize overflow for any real line of UI text"
    )]
    let (width_usize, height_usize) = (width_px as usize, height_px as usize);
    #[allow(
        clippy::cast_possible_truncation,
        reason = "ascent plus a couple of pixels of padding is far below f32's precision-loss \
                  range for any real font size used by this app"
    )]
    let baseline = (ascent + MASK_PADDING_PX) as f32;

    let color_space = CGColorSpace::create_device_gray();
    let mut ctx = CGContext::create_bitmap_context(
        None,
        width_usize,
        height_usize,
        8,
        width_usize,
        &color_space,
        kCGImageAlphaNone,
    );
    // Disable CoreText's stem-darkening "font smoothing": it is tuned for subpixel color
    // rendering and would skew the plain grayscale coverage values this mask relies on.
    ctx.set_allows_font_smoothing(false);
    ctx.set_should_smooth_fonts(false);
    ctx.set_should_antialias(true);
    ctx.set_gray_fill_color(0.0, 1.0);
    ctx.fill_rect(CGRect::new(
        &CGPoint::new(0.0, 0.0),
        &CGSize::new(width_px, height_px),
    ));
    ctx.set_text_position(MASK_PADDING_PX, descent + MASK_PADDING_PX);
    line.draw(&ctx);
    ctx.flush();

    let data = ctx.data().to_vec();

    #[allow(
        clippy::cast_possible_truncation,
        reason = "a single rasterized line is far below u32::MAX in either dimension"
    )]
    RasterizedLine {
        width: width_usize as u32,
        height: height_usize as u32,
        baseline,
        data,
        width_points,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests only run on macOS (the whole `platform` module is `cfg(target_os =
    // "macos")`), and exercise the real CoreText/CoreGraphics APIs — no fakes involved, since
    // that is exactly what needs checking here.

    #[test]
    fn measure_returns_positive_width_for_non_empty_text() {
        let runs = vec![TextRun::plain("urahafu", FontWeight::Regular)];
        assert!(measure(&runs, 15.0) > 0.0);
    }

    #[test]
    fn heavier_weight_or_tracking_does_not_shrink_width() {
        let regular = vec![TextRun::plain("urahafu", FontWeight::Regular)];
        let mut semibold = TextRun::plain("urahafu", FontWeight::Semibold);
        semibold.tracking_em = 0.05;
        let tracked = vec![semibold];
        assert!(measure(&tracked, 15.0) >= measure(&regular, 15.0));
    }

    #[test]
    fn rasterize_produces_a_non_empty_mask_with_some_coverage() {
        let runs = vec![TextRun::plain("3", FontWeight::Light)];
        let mask = rasterize(&runs, 160.0, 2.0);
        assert!(mask.width > 0 && mask.height > 0);
        assert_eq!(mask.data.len(), (mask.width * mask.height) as usize);
        assert!(
            mask.data.iter().any(|&byte| byte > 0),
            "expected at least one covered pixel for a rasterized digit"
        );
    }

    #[test]
    #[allow(
        clippy::cast_precision_loss,
        reason = "a rasterized line's pixel height is a couple hundred pixels at most, far below \
                  f32's precision-loss range"
    )]
    fn rasterize_scale_roughly_doubles_pixel_height() {
        let runs = vec![TextRun::plain("Ay", FontWeight::Regular)];
        let at_1x = rasterize(&runs, 20.0, 1.0);
        let at_2x = rasterize(&runs, 20.0, 2.0);
        assert!((at_2x.height as f32) > (at_1x.height as f32) * 1.5);
    }

    #[test]
    fn mixed_run_line_measures_wider_than_either_run_alone() {
        let hint_only = vec![TextRun::plain("Type ", FontWeight::Regular)];
        let mut word = vec![TextRun::plain("Type ", FontWeight::Regular)];
        word.push(TextRun::plain("urahafu", FontWeight::Semibold));
        assert!(measure(&word, 15.0) > measure(&hint_only, 15.0));
    }
}
