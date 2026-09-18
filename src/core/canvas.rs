//! Software pixel drawing into a raw `&mut [u32]` buffer.
//!
//! Every pixel is packed as `0x00RRGGBB` (the format `softbuffer` expects). This module contains
//! all the pixel math the overlay needs — solid fills, alpha-blended rectangles, anti-aliased
//! circles and rounded rectangles, and blitting an 8-bit alpha mask (as produced by a text
//! rasterizer) with a tint color and opacity — and nothing platform-specific: it is exercised
//! entirely with small in-memory buffers in tests.
//!
//! ## Coordinate system
//!
//! Top-left origin, `x` right, `y` down, matching [`crate::core::layout`]. Positions passed in
//! are already in pixels (the renderer has multiplied the point values from `layout` by the
//! display's backing scale factor before calling into this module).

use crate::core::color::Rgb;

/// A mutable view over a pixel buffer, with an explicit stride so the buffer may have padding
/// beyond `width` (as some presentation backends require).
pub struct Canvas<'a> {
    buffer: &'a mut [u32],
    width: usize,
    height: usize,
    stride: usize,
}

/// An axis-aligned pixel rectangle. Signed so callers can pass rectangles that extend past the
/// canvas edges (in either direction) and rely on clipping rather than pre-computing overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
}

impl PixelRect {
    /// Creates a pixel rectangle.
    #[must_use]
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

impl<'a> Canvas<'a> {
    /// Wraps `buffer` as a canvas of `width` × `height` pixels with the given `stride` (pixels
    /// per row, `>= width`).
    ///
    /// # Panics
    ///
    /// Panics if `stride < width` or `buffer` is too small to hold `height` rows of `stride`
    /// pixels — both are programmer errors (a mismatched buffer/geometry pairing), not conditions
    /// that can arise from untrusted input.
    #[must_use]
    pub fn new(buffer: &'a mut [u32], width: usize, height: usize, stride: usize) -> Self {
        assert!(
            stride >= width,
            "stride ({stride}) must be >= width ({width})"
        );
        assert!(
            buffer.len() >= stride * height,
            "buffer too small for {width}x{height} at stride {stride}"
        );
        Self {
            buffer,
            width,
            height,
            stride,
        }
    }

    /// Canvas width in pixels.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// Canvas height in pixels.
    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    /// Fills the whole canvas with a solid, opaque color.
    pub fn fill(&mut self, color: Rgb) {
        let packed = color.to_u32();
        for y in 0..self.height {
            let row_start = y * self.stride;
            self.buffer[row_start..row_start + self.width].fill(packed);
        }
    }

    /// Blends `color` at `opacity` (`[0.0, 1.0]`) over `rect`, clipped to the canvas bounds on
    /// every edge.
    pub fn fill_rect(&mut self, rect: PixelRect, color: Rgb, opacity: f32) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.clip_rect(rect) else {
            return;
        };

        for y in y0..y1 {
            for x in x0..x1 {
                #[allow(
                    clippy::cast_possible_wrap,
                    clippy::cast_possible_truncation,
                    reason = "clip_rect bounds x/y by the canvas size, which fits in i32"
                )]
                self.blend_pixel(x as i32, y as i32, color, opacity);
            }
        }
    }

    /// Draws an anti-aliased filled circle centered at (`center_x`, `center_y`) with `radius`,
    /// tinted `color` at `opacity`. Pixels near the edge get partial coverage instead of a hard,
    /// jagged boundary.
    pub fn fill_circle(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius: f32,
        color: Rgb,
        opacity: f32,
    ) {
        if radius <= 0.0 || opacity <= 0.0 {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);

        #[allow(
            clippy::cast_precision_loss,
            reason = "canvas dimensions fit exactly in f32 for any real display"
        )]
        let (width_f, height_f) = (self.width as f32, self.height as f32);
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by radius/canvas size, well within i64 range"
        )]
        let min_x = (center_x - radius - 1.0).floor().max(0.0) as i64;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by radius/canvas size, well within i64 range"
        )]
        let min_y = (center_y - radius - 1.0).floor().max(0.0) as i64;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by radius/canvas size, well within i64 range"
        )]
        let max_x = (center_x + radius + 1.0).ceil().min(width_f) as i64;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by radius/canvas size, well within i64 range"
        )]
        let max_y = (center_y + radius + 1.0).ceil().min(height_f) as i64;

        for py in min_y..max_y {
            for px in min_x..max_x {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "pixel coordinates are small integers"
                )]
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let dist = ((fx - center_x).powi(2) + (fy - center_y).powi(2)).sqrt();
                // Coverage ramps linearly over a 1px band straddling the edge, giving a soft but
                // crisp anti-aliased boundary.
                let coverage = (radius + 0.5 - dist).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "bounded by min/max clamps above"
                    )]
                    self.blend_pixel(px as i32, py as i32, color, opacity * coverage);
                }
            }
        }
    }

    /// Draws an anti-aliased filled rounded rectangle, tinted `color` at `opacity`.
    pub fn fill_rounded_rect(
        &mut self,
        rect: PixelRect,
        corner_radius: f32,
        color: Rgb,
        opacity: f32,
    ) {
        if opacity <= 0.0 || rect.width <= 0 || rect.height <= 0 {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        #[allow(
            clippy::cast_precision_loss,
            reason = "rect dimensions are small integers"
        )]
        let half_size = (rect.width as f32 / 2.0, rect.height as f32 / 2.0);
        let radius = corner_radius.min(half_size.0).min(half_size.1).max(0.0);
        #[allow(
            clippy::cast_precision_loss,
            reason = "rect coordinates are small integers"
        )]
        let center = (rect.x as f32 + half_size.0, rect.y as f32 + half_size.1);

        let Some((x0, y0, x1, y1)) = self.clip_rect(PixelRect::new(
            rect.x - 1,
            rect.y - 1,
            rect.width + 2,
            rect.height + 2,
        )) else {
            return;
        };

        for y in y0..y1 {
            for x in x0..x1 {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "pixel coordinates are small integers"
                )]
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let dist = rounded_rect_sdf(
                    fx - center.0,
                    fy - center.1,
                    half_size.0,
                    half_size.1,
                    radius,
                );
                let coverage = (0.5 - dist).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    #[allow(
                        clippy::cast_possible_wrap,
                        clippy::cast_possible_truncation,
                        reason = "clip_rect bounds x/y by the canvas size, which fits in i32"
                    )]
                    self.blend_pixel(x as i32, y as i32, color, opacity * coverage);
                }
            }
        }
    }

    /// Draws an anti-aliased **stroked** (outline-only) rounded rectangle, `stroke_width` pixels
    /// wide and centered on the rounded rect's own edge (half the stroke inside it, half
    /// outside), tinted `color` at `opacity`. Used for glyphs drawn from canvas primitives
    /// rather than a bitmap (DESIGN.md §1: no embedded icon assets) that need an outline rather
    /// than a filled shape, e.g. the keyboard-only HUD's keyboard glyph (DESIGN.md §9).
    pub fn stroke_rounded_rect(
        &mut self,
        rect: PixelRect,
        corner_radius: f32,
        stroke_width: f32,
        color: Rgb,
        opacity: f32,
    ) {
        if opacity <= 0.0 || rect.width <= 0 || rect.height <= 0 || stroke_width <= 0.0 {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        #[allow(
            clippy::cast_precision_loss,
            reason = "rect dimensions are small integers"
        )]
        let half_size = (rect.width as f32 / 2.0, rect.height as f32 / 2.0);
        let radius = corner_radius.min(half_size.0).min(half_size.1).max(0.0);
        #[allow(
            clippy::cast_precision_loss,
            reason = "rect coordinates are small integers"
        )]
        let center = (rect.x as f32 + half_size.0, rect.y as f32 + half_size.1);

        // The stroke band extends `stroke_width / 2` (plus a pixel of anti-aliasing) on either
        // side of the shape's edge, so the scan region needs that much extra padding beyond the
        // shape's own bounding box.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "stroke widths are small UI dimensions, far below i32::MAX"
        )]
        let pad = (stroke_width / 2.0 + 1.0).ceil() as i32;
        let Some((x0, y0, x1, y1)) = self.clip_rect(PixelRect::new(
            rect.x - pad,
            rect.y - pad,
            rect.width + pad * 2,
            rect.height + pad * 2,
        )) else {
            return;
        };

        let half_stroke = stroke_width / 2.0;
        for y in y0..y1 {
            for x in x0..x1 {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "pixel coordinates are small integers"
                )]
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let dist = rounded_rect_sdf(
                    fx - center.0,
                    fy - center.1,
                    half_size.0,
                    half_size.1,
                    radius,
                );
                // Distance *from the edge line* (`dist == 0`), not from the shape's interior:
                // this is what turns the filled-shape SDF into a stroke — a band of coverage
                // straddling the boundary instead of everything inside it.
                let coverage = (half_stroke + 0.5 - dist.abs()).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    #[allow(
                        clippy::cast_possible_wrap,
                        clippy::cast_possible_truncation,
                        reason = "clip_rect bounds x/y by the canvas size, which fits in i32"
                    )]
                    self.blend_pixel(x as i32, y as i32, color, opacity * coverage);
                }
            }
        }
    }

    /// Draws an anti-aliased, round-capped line segment from (`x0`, `y0`) to (`x1`, `y1`),
    /// `stroke_width` pixels wide, tinted `color` at `opacity`. Used for glyphs drawn from canvas
    /// primitives rather than a bitmap (DESIGN.md §1: no embedded icon assets), e.g. the
    /// hold-to-unlock button's ✕ glyph (DESIGN.md §5/§8).
    #[allow(
        clippy::too_many_arguments,
        reason = "a line-segment stroke inherently needs both endpoints, its width, and its paint"
    )]
    pub fn stroke_line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stroke_width: f32,
        color: Rgb,
        opacity: f32,
    ) {
        if opacity <= 0.0 || stroke_width <= 0.0 {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let half = stroke_width / 2.0;

        let min_x = x0.min(x1) - half - 1.0;
        let max_x = x0.max(x1) + half + 1.0;
        let min_y = y0.min(y1) - half - 1.0;
        let max_y = y0.max(y1) + half + 1.0;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by the segment's own coordinates plus a small pad, well within i32"
        )]
        let Some((cx0, cy0, cx1, cy1)) = self.clip_rect(PixelRect::new(
            min_x.floor() as i32,
            min_y.floor() as i32,
            (max_x - min_x).ceil() as i32,
            (max_y - min_y).ceil() as i32,
        )) else {
            return;
        };

        let (dx, dy) = (x1 - x0, y1 - y0);
        let len_sq = dx.mul_add(dx, dy * dy);
        for y in cy0..cy1 {
            for x in cx0..cx1 {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "pixel coordinates are small integers"
                )]
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let dist = distance_to_segment(fx, fy, x0, y0, dx, dy, len_sq);
                let coverage = (half + 0.5 - dist).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    #[allow(
                        clippy::cast_possible_wrap,
                        clippy::cast_possible_truncation,
                        reason = "clip_rect bounds x/y by the canvas size, which fits in i32"
                    )]
                    self.blend_pixel(x as i32, y as i32, color, opacity * coverage);
                }
            }
        }
    }

    /// Draws an anti-aliased circular arc stroke, `stroke_width` pixels wide, centered at
    /// (`center_x`, `center_y`) with `radius`, sweeping **clockwise from 12 o'clock** over
    /// `fraction * 360°` (`fraction` clamped to `[0.0, 1.0]`), tinted `color` at `opacity`, with a
    /// round cap at the sweep's leading end. `fraction <= 0.0` paints nothing; `fraction >= 1.0`
    /// paints a full ring (no cap needed, since there is no gap to cap). Used for the
    /// hold-to-unlock button's hold-progress ring (DESIGN.md §8: "progress ring ... clockwise
    /// from 12 o'clock").
    #[allow(
        clippy::too_many_arguments,
        reason = "an arc stroke inherently needs the center, radius, width, sweep and paint"
    )]
    pub fn stroke_arc(
        &mut self,
        center_x: f32,
        center_y: f32,
        radius: f32,
        stroke_width: f32,
        fraction: f32,
        color: Rgb,
        opacity: f32,
    ) {
        let fraction = fraction.clamp(0.0, 1.0);
        if opacity <= 0.0 || radius <= 0.0 || stroke_width <= 0.0 || fraction <= 0.0 {
            return;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let half_stroke = stroke_width / 2.0;
        let sweep = fraction * std::f32::consts::TAU;
        let full_ring = fraction >= 1.0;

        // Round cap at the sweep's leading (end) point; the start is fixed at 12 o'clock and
        // needs no separate cap treatment at this stroke width. Not needed for a full ring, since
        // there is no gap there to cap.
        let cap = if full_ring {
            None
        } else {
            Some((
                center_x + radius * sweep.sin(),
                center_y - radius * sweep.cos(),
            ))
        };

        let pad = half_stroke + 1.0;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by radius/canvas size, well within i32 range"
        )]
        let Some((x0, y0, x1, y1)) = self.clip_rect(PixelRect::new(
            (center_x - radius - pad).floor() as i32,
            (center_y - radius - pad).floor() as i32,
            (2.0 * (radius + pad)).ceil() as i32,
            (2.0 * (radius + pad)).ceil() as i32,
        )) else {
            return;
        };

        for y in y0..y1 {
            for x in x0..x1 {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "pixel coordinates are small integers"
                )]
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (dx, dy) = (fx - center_x, fy - center_y);
                let dist = dx.hypot(dy);
                let radial_coverage = (half_stroke + 0.5 - (dist - radius).abs()).clamp(0.0, 1.0);

                let angular_coverage = if full_ring {
                    1.0
                } else {
                    let mut theta = dx.atan2(-dy);
                    if theta < 0.0 {
                        theta += std::f32::consts::TAU;
                    }
                    // One pixel's worth of arc-length in radians, for a soft trailing edge
                    // instead of a hard angular cutoff.
                    let edge_aa = (1.0 / radius.max(1.0)).min(sweep.max(1e-6));
                    if theta <= sweep {
                        1.0
                    } else if theta <= sweep + edge_aa {
                        1.0 - (theta - sweep) / edge_aa
                    } else {
                        0.0
                    }
                };

                let mut coverage = radial_coverage * angular_coverage;
                if let Some((cap_x, cap_y)) = cap {
                    let cap_dist = (fx - cap_x).hypot(fy - cap_y);
                    let cap_coverage = (half_stroke + 0.5 - cap_dist).clamp(0.0, 1.0);
                    coverage = coverage.max(cap_coverage);
                }

                if coverage > 0.0 {
                    #[allow(
                        clippy::cast_possible_wrap,
                        clippy::cast_possible_truncation,
                        reason = "clip_rect bounds x/y by the canvas size, which fits in i32"
                    )]
                    self.blend_pixel(x as i32, y as i32, color, opacity * coverage);
                }
            }
        }
    }

    /// Blits an 8-bit alpha coverage mask (as produced by a text rasterizer: one byte per pixel,
    /// 0 = transparent, 255 = fully covered) at `(dest_x, dest_y)`, tinted `color` at `opacity`,
    /// clipped to the canvas on every edge (including a mask that starts off-canvas, extends past
    /// it, or is entirely outside it).
    #[allow(
        clippy::too_many_arguments,
        reason = "a mask blit inherently has this many independent parameters"
    )]
    #[allow(
        clippy::similar_names,
        reason = "src_x0/src_y0/src_x1/src_y1 and dst_x/dst_y are paired x/y coordinates; giving them \
                  deliberately dissimilar names would make the geometry harder to follow, not easier"
    )]
    pub fn blit_alpha_mask(
        &mut self,
        mask: &[u8],
        mask_width: usize,
        mask_height: usize,
        dest_x: i32,
        dest_y: i32,
        color: Rgb,
        opacity: f32,
    ) {
        if opacity <= 0.0 || mask_width == 0 || mask_height == 0 {
            return;
        }
        debug_assert_eq!(
            mask.len(),
            mask_width * mask_height,
            "mask buffer size must match mask_width * mask_height"
        );
        let opacity = opacity.clamp(0.0, 1.0);

        #[allow(
            clippy::cast_possible_wrap,
            clippy::cast_possible_truncation,
            reason = "mask dimensions are small (glyph-sized), far below i32::MAX"
        )]
        let (mask_w, mask_h) = (mask_width as i32, mask_height as i32);

        // Intersect the mask's destination rect with the canvas.
        let src_x0 = (-dest_x).max(0);
        let src_y0 = (-dest_y).max(0);
        #[allow(
            clippy::cast_possible_wrap,
            clippy::cast_possible_truncation,
            reason = "canvas dimensions fit comfortably in i32 for any real display"
        )]
        let (canvas_w, canvas_h) = (self.width as i32, self.height as i32);
        let src_x1 = mask_w.min(canvas_w - dest_x);
        let src_y1 = mask_h.min(canvas_h - dest_y);

        if src_x0 >= src_x1 || src_y0 >= src_y1 {
            return;
        }

        for src_y in src_y0..src_y1 {
            for src_x in src_x0..src_x1 {
                #[allow(
                    clippy::cast_sign_loss,
                    clippy::cast_possible_truncation,
                    reason = "src_x/src_y are within [0, mask_w/h) here"
                )]
                let mask_index = (src_y as usize) * mask_width + (src_x as usize);
                let coverage = f32::from(mask[mask_index]) / 255.0;
                if coverage <= 0.0 {
                    continue;
                }
                let dst_x = dest_x + src_x;
                let dst_y = dest_y + src_y;
                self.blend_pixel(dst_x, dst_y, color, opacity * coverage);
            }
        }
    }

    /// Clips `rect` to the canvas bounds, returning `(x0, y0, x1, y1)` in pixel indices with
    /// `x0 <= x1`, `y0 <= y1`, or `None` if the rectangle does not overlap the canvas at all.
    fn clip_rect(&self, rect: PixelRect) -> Option<(usize, usize, usize, usize)> {
        if rect.width <= 0 || rect.height <= 0 {
            return None;
        }
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = rect.x.saturating_add(rect.width).min(self.width_i32());
        let y1 = rect.y.saturating_add(rect.height).min(self.height_i32());
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        #[allow(
            clippy::cast_sign_loss,
            reason = "x0/y0/x1/y1 are non-negative after the checks above"
        )]
        Some((x0 as usize, y0 as usize, x1 as usize, y1 as usize))
    }

    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        reason = "canvas dimensions fit comfortably in i32 for any real display"
    )]
    fn width_i32(&self) -> i32 {
        self.width as i32
    }

    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        reason = "canvas dimensions fit comfortably in i32 for any real display"
    )]
    fn height_i32(&self) -> i32 {
        self.height as i32
    }

    /// Alpha-blends `color` at coverage `alpha` (`[0.0, 1.0]`) into the pixel at `(x, y)`. A
    /// no-op if `(x, y)` is outside the canvas.
    fn blend_pixel(&mut self, x: i32, y: i32, color: Rgb, alpha: f32) {
        if x < 0 || y < 0 || x >= self.width_i32() || y >= self.height_i32() {
            return;
        }
        #[allow(clippy::cast_sign_loss, reason = "x/y are checked non-negative above")]
        let index = (y as usize) * self.stride + (x as usize);
        let Some(existing) = self.buffer.get(index).copied() else {
            return;
        };
        let old = Rgb::new(
            u8::try_from((existing >> 16) & 0xFF).unwrap_or(0),
            u8::try_from((existing >> 8) & 0xFF).unwrap_or(0),
            u8::try_from(existing & 0xFF).unwrap_or(0),
        );
        let blended = Rgb::new(
            blend_channel(old.r, color.r, alpha),
            blend_channel(old.g, color.g, alpha),
            blend_channel(old.b, color.b, alpha),
        );
        self.buffer[index] = blended.to_u32();
    }
}

/// Signed-distance-ish estimate (Inigo Quilez's rounded-box formula) of the distance from a
/// point at `(dx, dy)` relative to a box's center, to the edge of a rounded box of half-extents
/// `half_w` × `half_h` and corner radius `radius`. Negative inside, positive outside, in pixels.
fn rounded_rect_sdf(dx: f32, dy: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let qx = dx.abs() - half_w + radius;
    let qy = dy.abs() - half_h + radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// Distance from point (`px`, `py`) to the line segment starting at (`x0`, `y0`) with direction
/// (`dx`, `dy`) (i.e. ending at `(x0 + dx, y0 + dy)`) and squared length `len_sq`, clamping the
/// projection to the segment so points beyond either end measure to the nearest endpoint (the cap
/// of a round-capped stroke).
fn distance_to_segment(px: f32, py: f32, x0: f32, y0: f32, dx: f32, dy: f32, len_sq: f32) -> f32 {
    if len_sq <= f32::EPSILON {
        return (px - x0).hypot(py - y0);
    }
    let t = (((px - x0) * dx + (py - y0) * dy) / len_sq).clamp(0.0, 1.0);
    let (cx, cy) = (x0 + t * dx, y0 + t * dy);
    (px - cx).hypot(py - cy)
}

/// Blends one 8-bit channel: `old * (1 - alpha) + new * alpha`, rounded to the nearest integer.
fn blend_channel(old: u8, new: u8, alpha: f32) -> u8 {
    let value = f32::from(old).mul_add(1.0 - alpha, f32::from(new) * alpha);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is clamped to [0, 255] just above"
    )]
    let rounded = value.round().clamp(0.0, 255.0) as u8;
    rounded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(buffer: &[u32], stride: usize, x: usize, y: usize) -> Rgb {
        let word = buffer[y * stride + x];
        Rgb::new(
            u8::try_from((word >> 16) & 0xFF).unwrap(),
            u8::try_from((word >> 8) & 0xFF).unwrap(),
            u8::try_from(word & 0xFF).unwrap(),
        )
    }

    #[test]
    fn fill_sets_every_pixel() {
        let mut buffer = vec![0u32; 16];
        let mut canvas = Canvas::new(&mut buffer, 4, 4, 4);
        canvas.fill(Rgb::new(10, 20, 30));
        for &word in &buffer {
            assert_eq!(word, Rgb::new(10, 20, 30).to_u32());
        }
    }

    #[test]
    fn fill_respects_stride_padding() {
        let mut buffer = vec![0u32; 4 * 4];
        {
            let mut canvas = Canvas::new(&mut buffer, 3, 4, 4);
            canvas.fill(Rgb::WHITE);
        }
        // Column 3 (the padding column) must be untouched.
        for y in 0..4 {
            assert_eq!(buffer[y * 4 + 3], 0, "padding column should not be written");
        }
        for y in 0..4 {
            for x in 0..3 {
                assert_eq!(get(&buffer, 4, x, y), Rgb::WHITE);
            }
        }
    }

    #[test]
    fn fill_rect_blends_at_partial_opacity() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 4];
        let mut canvas = Canvas::new(&mut buffer, 2, 2, 2);
        canvas.fill_rect(PixelRect::new(0, 0, 2, 2), Rgb::WHITE, 0.5);
        let pixel = get(&buffer, 2, 0, 0);
        assert_eq!(pixel, Rgb::new(128, 128, 128));
    }

    #[test]
    fn fill_rect_clips_to_canvas_on_every_edge() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 9];
        let mut canvas = Canvas::new(&mut buffer, 3, 3, 3);
        // Rect straddling all four edges, only the center pixel is fully inside.
        canvas.fill_rect(PixelRect::new(-5, -5, 11, 11), Rgb::WHITE, 1.0);
        for y in 0..3 {
            for x in 0..3 {
                assert_eq!(get(&buffer, 3, x, y), Rgb::WHITE);
            }
        }
    }

    #[test]
    fn fill_rect_entirely_off_canvas_is_a_no_op() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 4];
        let mut canvas = Canvas::new(&mut buffer, 2, 2, 2);
        canvas.fill_rect(PixelRect::new(10, 10, 5, 5), Rgb::WHITE, 1.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }

    #[test]
    fn fill_rect_zero_opacity_is_a_no_op() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 4];
        let mut canvas = Canvas::new(&mut buffer, 2, 2, 2);
        canvas.fill_rect(PixelRect::new(0, 0, 2, 2), Rgb::WHITE, 0.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }

    #[test]
    fn fill_circle_center_is_opaque_and_far_corner_untouched() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 20 * 20];
        let mut canvas = Canvas::new(&mut buffer, 20, 20, 20);
        canvas.fill_circle(10.0, 10.0, 8.0, Rgb::WHITE, 1.0);
        assert_eq!(get(&buffer, 20, 10, 10), Rgb::WHITE);
        assert_eq!(get(&buffer, 20, 0, 0), Rgb::BLACK);
    }

    #[test]
    fn fill_circle_edge_is_anti_aliased() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 20 * 20];
        let mut canvas = Canvas::new(&mut buffer, 20, 20, 20);
        canvas.fill_circle(10.0, 10.0, 8.0, Rgb::WHITE, 1.0);
        // Right at the radius boundary, expect partial (not 0, not 255) coverage on some pixel.
        let edge = get(&buffer, 20, 17, 10);
        assert!(
            edge.r > 0 && edge.r < 255,
            "expected partial coverage at the circle edge, got {edge}"
        );
    }

    #[test]
    fn fill_circle_clips_when_partially_off_canvas() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 10 * 10];
        let mut canvas = Canvas::new(&mut buffer, 10, 10, 10);
        // Center outside the canvas; should not panic and should still paint the visible part.
        canvas.fill_circle(-2.0, 5.0, 6.0, Rgb::WHITE, 1.0);
        assert_eq!(get(&buffer, 10, 0, 5), Rgb::WHITE);
    }

    #[test]
    fn fill_rounded_rect_center_is_opaque() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 40 * 20];
        let mut canvas = Canvas::new(&mut buffer, 40, 20, 40);
        canvas.fill_rounded_rect(PixelRect::new(5, 2, 30, 16), 8.0, Rgb::WHITE, 1.0);
        assert_eq!(get(&buffer, 40, 20, 10), Rgb::WHITE);
    }

    #[test]
    fn fill_rounded_rect_corner_is_rounded_off() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 40 * 20];
        let mut canvas = Canvas::new(&mut buffer, 40, 20, 40);
        canvas.fill_rounded_rect(PixelRect::new(5, 2, 30, 16), 8.0, Rgb::WHITE, 1.0);
        // The extreme corner pixel of the bounding box should be untouched by a rounded rect.
        assert_eq!(get(&buffer, 40, 5, 2), Rgb::BLACK);
    }

    #[test]
    fn stroke_rounded_rect_center_is_untouched() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 40 * 20];
        let mut canvas = Canvas::new(&mut buffer, 40, 20, 40);
        canvas.stroke_rounded_rect(PixelRect::new(5, 2, 30, 16), 4.0, 2.0, Rgb::WHITE, 1.0);
        // Dead center of a stroked (not filled) shape must stay background: only a band near the
        // edge is painted.
        assert_eq!(get(&buffer, 40, 20, 10), Rgb::BLACK);
    }

    #[test]
    fn stroke_rounded_rect_paints_a_band_at_the_edge() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 40 * 20];
        let mut canvas = Canvas::new(&mut buffer, 40, 20, 40);
        canvas.stroke_rounded_rect(PixelRect::new(5, 2, 30, 16), 4.0, 2.0, Rgb::WHITE, 1.0);
        // Top edge, away from any corner: should be painted by the stroke.
        let edge = get(&buffer, 40, 20, 2);
        assert_ne!(
            edge,
            Rgb::BLACK,
            "expected the stroke to paint the shape's edge"
        );
    }

    #[test]
    fn stroke_rounded_rect_outside_the_padded_bounds_is_untouched() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 40 * 20];
        let mut canvas = Canvas::new(&mut buffer, 40, 20, 40);
        canvas.stroke_rounded_rect(PixelRect::new(5, 2, 30, 16), 4.0, 2.0, Rgb::WHITE, 1.0);
        // Far corner of the canvas, well outside the rect plus any stroke padding.
        assert_eq!(get(&buffer, 40, 0, 0), Rgb::BLACK);
        assert_eq!(get(&buffer, 40, 39, 19), Rgb::BLACK);
    }

    #[test]
    fn stroke_rounded_rect_zero_opacity_is_a_no_op() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 40 * 20];
        let mut canvas = Canvas::new(&mut buffer, 40, 20, 40);
        canvas.stroke_rounded_rect(PixelRect::new(5, 2, 30, 16), 4.0, 2.0, Rgb::WHITE, 0.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }

    #[test]
    fn stroke_rounded_rect_zero_width_is_a_no_op() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 40 * 20];
        let mut canvas = Canvas::new(&mut buffer, 40, 20, 40);
        canvas.stroke_rounded_rect(PixelRect::new(5, 2, 30, 16), 4.0, 0.0, Rgb::WHITE, 1.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }

    #[test]
    fn blit_alpha_mask_paints_fully_covered_pixels() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 10 * 10];
        let mut canvas = Canvas::new(&mut buffer, 10, 10, 10);
        let mask = [255u8, 0, 0, 255]; // 2x2, diagonal covered
        canvas.blit_alpha_mask(&mask, 2, 2, 3, 3, Rgb::WHITE, 1.0);
        assert_eq!(get(&buffer, 10, 3, 3), Rgb::WHITE);
        assert_eq!(get(&buffer, 10, 4, 3), Rgb::BLACK);
        assert_eq!(get(&buffer, 10, 3, 4), Rgb::BLACK);
        assert_eq!(get(&buffer, 10, 4, 4), Rgb::WHITE);
    }

    #[test]
    fn blit_alpha_mask_clips_negative_origin() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 4 * 4];
        let mut canvas = Canvas::new(&mut buffer, 4, 4, 4);
        let mask = [255u8; 4]; // 2x2, fully covered
        canvas.blit_alpha_mask(&mask, 2, 2, -1, -1, Rgb::WHITE, 1.0);
        // Only the bottom-right pixel of the mask (at canvas (0,0)) should land on the canvas.
        assert_eq!(get(&buffer, 4, 0, 0), Rgb::WHITE);
    }

    #[test]
    fn blit_alpha_mask_clips_past_far_edge() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 4 * 4];
        let mut canvas = Canvas::new(&mut buffer, 4, 4, 4);
        let mask = [255u8; 4]; // 2x2, fully covered
        canvas.blit_alpha_mask(&mask, 2, 2, 3, 3, Rgb::WHITE, 1.0);
        // Only the top-left pixel of the mask (at canvas (3,3)) should land on the canvas.
        assert_eq!(get(&buffer, 4, 3, 3), Rgb::WHITE);
    }

    #[test]
    fn blit_alpha_mask_entirely_off_canvas_is_a_no_op() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 4 * 4];
        let mut canvas = Canvas::new(&mut buffer, 4, 4, 4);
        let mask = [255u8; 4];
        canvas.blit_alpha_mask(&mask, 2, 2, 100, 100, Rgb::WHITE, 1.0);
        canvas.blit_alpha_mask(&mask, 2, 2, -100, -100, Rgb::WHITE, 1.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }

    #[test]
    fn blit_alpha_mask_respects_opacity_and_coverage() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 4];
        let mut canvas = Canvas::new(&mut buffer, 2, 2, 2);
        let mask = [128u8]; // 1x1, 50% coverage
        canvas.blit_alpha_mask(&mask, 1, 1, 0, 0, Rgb::WHITE, 0.5);
        // Effective alpha ~= 0.5 * (128/255) ~= 0.251 -> ~64
        let pixel = get(&buffer, 2, 0, 0);
        assert!(
            (i32::from(pixel.r) - 64).abs() <= 2,
            "unexpected blended value: {pixel}"
        );
    }

    #[test]
    #[should_panic(expected = "stride")]
    fn new_panics_on_stride_smaller_than_width() {
        let mut buffer = vec![0u32; 4];
        let _ = Canvas::new(&mut buffer, 4, 1, 2);
    }

    #[test]
    fn stroke_line_paints_the_midpoint() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 20 * 20];
        let mut canvas = Canvas::new(&mut buffer, 20, 20, 20);
        canvas.stroke_line(2.0, 10.0, 18.0, 10.0, 4.0, Rgb::WHITE, 1.0);
        assert_eq!(get(&buffer, 20, 10, 10), Rgb::WHITE);
    }

    #[test]
    fn stroke_line_is_round_capped() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 20 * 20];
        let mut canvas = Canvas::new(&mut buffer, 20, 20, 20);
        // A horizontal segment from x=5 to x=15, 4px wide (half = 2). A point just past the
        // right endpoint but within the cap's radius should still be covered.
        canvas.stroke_line(5.0, 10.0, 15.0, 10.0, 4.0, Rgb::WHITE, 1.0);
        assert_ne!(
            get(&buffer, 20, 16, 10),
            Rgb::BLACK,
            "round cap should extend coverage just past the endpoint"
        );
        // Far beyond the cap's radius: untouched.
        assert_eq!(get(&buffer, 20, 19, 10), Rgb::BLACK);
    }

    #[test]
    fn stroke_line_far_away_is_untouched() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 20 * 20];
        let mut canvas = Canvas::new(&mut buffer, 20, 20, 20);
        canvas.stroke_line(2.0, 10.0, 18.0, 10.0, 4.0, Rgb::WHITE, 1.0);
        assert_eq!(get(&buffer, 20, 10, 0), Rgb::BLACK);
        assert_eq!(get(&buffer, 20, 10, 19), Rgb::BLACK);
    }

    #[test]
    fn stroke_line_zero_width_is_a_no_op() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 20 * 20];
        let mut canvas = Canvas::new(&mut buffer, 20, 20, 20);
        canvas.stroke_line(2.0, 10.0, 18.0, 10.0, 0.0, Rgb::WHITE, 1.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }

    #[test]
    fn stroke_arc_zero_fraction_paints_nothing() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 60 * 60];
        let mut canvas = Canvas::new(&mut buffer, 60, 60, 60);
        canvas.stroke_arc(30.0, 30.0, 20.0, 3.0, 0.0, Rgb::WHITE, 1.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }

    #[test]
    fn stroke_arc_full_fraction_paints_a_complete_ring() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 60 * 60];
        let mut canvas = Canvas::new(&mut buffer, 60, 60, 60);
        canvas.stroke_arc(30.0, 30.0, 20.0, 3.0, 1.0, Rgb::WHITE, 1.0);
        // Sample all four cardinal points of the ring: top, right, bottom, left.
        assert_ne!(get(&buffer, 60, 30, 10), Rgb::BLACK, "top of the ring");
        assert_ne!(get(&buffer, 60, 50, 30), Rgb::BLACK, "right of the ring");
        assert_ne!(get(&buffer, 60, 30, 50), Rgb::BLACK, "bottom of the ring");
        assert_ne!(get(&buffer, 60, 10, 30), Rgb::BLACK, "left of the ring");
        // Dead center stays background: only the ring band is painted.
        assert_eq!(get(&buffer, 60, 30, 30), Rgb::BLACK);
    }

    #[test]
    fn stroke_arc_quarter_fraction_only_paints_the_top_right_quadrant() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 60 * 60];
        let mut canvas = Canvas::new(&mut buffer, 60, 60, 60);
        // Sweeps clockwise from 12 o'clock over 25% of the circle: top to 3 o'clock (right).
        canvas.stroke_arc(30.0, 30.0, 20.0, 3.0, 0.25, Rgb::WHITE, 1.0);
        assert_ne!(get(&buffer, 60, 30, 10), Rgb::BLACK, "top (sweep start)");
        assert_ne!(get(&buffer, 60, 50, 30), Rgb::BLACK, "right (sweep end)");
        assert_eq!(get(&buffer, 60, 30, 50), Rgb::BLACK, "bottom untouched");
        assert_eq!(get(&buffer, 60, 10, 30), Rgb::BLACK, "left untouched");
    }

    #[test]
    fn stroke_arc_zero_opacity_is_a_no_op() {
        let mut buffer = vec![Rgb::BLACK.to_u32(); 60 * 60];
        let mut canvas = Canvas::new(&mut buffer, 60, 60, 60);
        canvas.stroke_arc(30.0, 30.0, 20.0, 3.0, 1.0, Rgb::WHITE, 0.0);
        for &word in &buffer {
            assert_eq!(word, Rgb::BLACK.to_u32());
        }
    }
}
