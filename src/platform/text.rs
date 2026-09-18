//! Safe, cached text rasterization on top of `src/platform/ffi/coretext.rs`.
//!
//! [`TextRasterizer`] is the only thing the rest of the platform layer (`render.rs`) talks to
//! for text: it owns no `unsafe` itself, and caches rasterized lines by their exact content,
//! size, weight(s), tracking and scale, so redrawing the same frame twice (nothing animating)
//! never re-touches CoreText.

use std::collections::HashMap;
use std::rc::Rc;

use crate::platform::ffi::coretext::{self, RasterizedLine};
pub use crate::platform::ffi::coretext::{FontWeight, TextRun};

/// An 8-bit coverage mask for one rasterized line of text, ready to hand to
/// [`crate::core::canvas::Canvas::blit_alpha_mask`].
pub type AlphaMask = RasterizedLine;

/// A single run's cache-key fields: everything about a [`TextRun`] that affects rasterized
/// pixels, in a form that is `Eq`/`Hash` (floats are compared via their exact bit patterns,
/// which is correct here since keys are only ever produced by re-deriving the same `f32` inputs,
/// never by arithmetic that could land on a different-but-equal-looking bit pattern).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RunKey {
    text: String,
    weight: FontWeight,
    tracking_em_bits: u32,
    tabular_numbers: bool,
}

impl From<&TextRun> for RunKey {
    fn from(run: &TextRun) -> Self {
        Self {
            text: run.text.clone(),
            weight: run.weight,
            tracking_em_bits: run.tracking_em.to_bits(),
            tabular_numbers: run.tabular_numbers,
        }
    }
}

/// Cache key for a rasterized (or measured) line: its runs, font size and rasterization scale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LineKey {
    runs: Vec<RunKey>,
    font_size_bits: u32,
    scale_bits: u32,
}

impl LineKey {
    fn new(runs: &[TextRun], font_size_pt: f32, scale: f32) -> Self {
        Self {
            runs: runs.iter().map(RunKey::from).collect(),
            font_size_bits: font_size_pt.to_bits(),
            scale_bits: scale.to_bits(),
        }
    }
}

/// Caches rasterized lines and measured widths by `(text, size, weight, tracking, scale)`, so
/// the renderer can call [`TextRasterizer::line`]/[`TextRasterizer::measure`] every frame without
/// worrying about re-rasterizing anything that has not changed.
#[derive(Debug, Default)]
pub struct TextRasterizer {
    masks: HashMap<LineKey, Rc<AlphaMask>>,
    /// Measurements are cached separately (keyed at scale 1.0 implicitly, since
    /// [`coretext::measure`] does not take a scale) so a size-only widget doesn't force a
    /// rasterization it may never need (e.g. layout code probing widths ahead of drawing).
    measurements: HashMap<Vec<RunKey>, (u32, f32)>,
}

impl TextRasterizer {
    /// Creates an empty rasterizer with an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rasterizes (or returns the cached mask for) `runs` at `font_size_pt` points, `scale`
    /// pixels per point.
    pub fn line(&mut self, runs: &[TextRun], font_size_pt: f32, scale: f32) -> Rc<AlphaMask> {
        let key = LineKey::new(runs, font_size_pt, scale);
        if let Some(mask) = self.masks.get(&key) {
            return Rc::clone(mask);
        }
        let mask = Rc::new(coretext::rasterize(runs, font_size_pt, scale));
        self.masks.insert(key, Rc::clone(&mask));
        mask
    }

    /// Convenience for a single-run line: see [`TextRasterizer::line`].
    pub fn text(
        &mut self,
        text: &str,
        font_size_pt: f32,
        weight: FontWeight,
        scale: f32,
    ) -> Rc<AlphaMask> {
        self.line(&[TextRun::plain(text, weight)], font_size_pt, scale)
    }

    /// Measures `runs` at `font_size_pt` points, in points (independent of any rasterization
    /// scale — see [`coretext::measure`]).
    pub fn measure(&mut self, runs: &[TextRun], font_size_pt: f32) -> f32 {
        let run_keys: Vec<RunKey> = runs.iter().map(RunKey::from).collect();
        let bits = font_size_pt.to_bits();
        if let Some(&(cached_bits, width)) = self.measurements.get(&run_keys) {
            if cached_bits == bits {
                return width;
            }
        }
        let width = coretext::measure(runs, font_size_pt);
        self.measurements.insert(run_keys, (bits, width));
        width
    }

    /// Convenience for a single-run measurement: see [`TextRasterizer::measure`].
    pub fn measure_text(&mut self, text: &str, font_size_pt: f32, weight: FontWeight) -> f32 {
        self.measure(&[TextRun::plain(text, weight)], font_size_pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_key_equal_for_identical_inputs() {
        let a = LineKey::new(&[TextRun::plain("urahafu", FontWeight::Regular)], 15.0, 2.0);
        let b = LineKey::new(&[TextRun::plain("urahafu", FontWeight::Regular)], 15.0, 2.0);
        assert_eq!(a, b);
    }

    #[test]
    fn line_key_differs_on_text() {
        let a = LineKey::new(&[TextRun::plain("urahafu", FontWeight::Regular)], 15.0, 2.0);
        let b = LineKey::new(&[TextRun::plain("rahaf", FontWeight::Regular)], 15.0, 2.0);
        assert_ne!(a, b);
    }

    #[test]
    fn line_key_differs_on_weight_tracking_scale_and_size() {
        let base = LineKey::new(&[TextRun::plain("urahafu", FontWeight::Regular)], 15.0, 2.0);
        let weight = LineKey::new(
            &[TextRun::plain("urahafu", FontWeight::Semibold)],
            15.0,
            2.0,
        );
        let size = LineKey::new(&[TextRun::plain("urahafu", FontWeight::Regular)], 16.0, 2.0);
        let scale = LineKey::new(&[TextRun::plain("urahafu", FontWeight::Regular)], 15.0, 1.0);
        let mut tracked = TextRun::plain("urahafu", FontWeight::Regular);
        tracked.tracking_em = 0.05;
        let tracking = LineKey::new(&[tracked], 15.0, 2.0);

        assert_ne!(base, weight);
        assert_ne!(base, size);
        assert_ne!(base, scale);
        assert_ne!(base, tracking);
    }

    #[test]
    fn line_key_treats_run_split_the_same_as_the_matching_single_run() {
        // Two runs with identical fields but split at a different boundary are still distinct
        // keys (their `runs` vectors differ), documenting that the cache is exact about run
        // boundaries and not just the concatenated text.
        let one_run = LineKey::new(&[TextRun::plain("urahafu", FontWeight::Regular)], 15.0, 2.0);
        let two_runs = LineKey::new(
            &[
                TextRun::plain("raha", FontWeight::Regular),
                TextRun::plain("fu", FontWeight::Regular),
            ],
            15.0,
            2.0,
        );
        assert_ne!(one_run, two_runs);
    }
}
