//! Easing functions used to animate opacity, scale and position over time.
//!
//! Every function takes `t` in `[0.0, 1.0]` (elapsed fraction of a transition) and returns a
//! value in `[0.0, 1.0]`, clamped at the boundaries so callers never have to clamp `t` themselves.

/// Clamps `t` to `[0.0, 1.0]`.
#[must_use]
fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}

/// Linear interpolation: `t` maps directly to the output.
///
/// Used for the simple fades (hint, wordmark, overlay) that DESIGN.md does not call out as
/// eased.
#[must_use]
pub fn linear(t: f32) -> f32 {
    clamp01(t)
}

/// Cubic ease-out: fast start, slow finish. Used for the countdown digit's fade/scale-in.
#[must_use]
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

/// Cubic ease-in-out: slow start, fast middle, slow finish. Used for the hint's fade-outs.
#[must_use]
pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = clamp01(t);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let f = 2.0 - 2.0 * t;
        1.0 - f.powi(3) / 2.0
    }
}

/// Linearly interpolates between `from` and `to` at fraction `t` (not clamped: callers pass an
/// already-eased `t`).
#[must_use]
pub fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_boundaries() {
        assert!((linear(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((linear(1.0) - 1.0).abs() < f32::EPSILON);
        assert!((linear(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn linear_clamps_out_of_range() {
        assert!((linear(-1.0) - 0.0).abs() < f32::EPSILON);
        assert!((linear(2.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn ease_out_cubic_boundaries() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < 1e-6);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ease_out_cubic_front_loaded() {
        // Ease-out rises faster than linear at the start of the transition.
        assert!(ease_out_cubic(0.25) > 0.25);
    }

    #[test]
    fn ease_in_out_cubic_boundaries() {
        assert!((ease_in_out_cubic(0.0) - 0.0).abs() < 1e-6);
        assert!((ease_in_out_cubic(1.0) - 1.0).abs() < 1e-6);
        assert!((ease_in_out_cubic(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn ease_in_out_cubic_symmetric() {
        let a = ease_in_out_cubic(0.25);
        let b = ease_in_out_cubic(0.75);
        assert!((a + b - 1.0).abs() < 1e-6);
    }

    #[test]
    fn lerp_interpolates() {
        assert!((lerp(0.0, 10.0, 0.5) - 5.0).abs() < 1e-6);
        assert!((lerp(10.0, 20.0, 0.0) - 10.0).abs() < 1e-6);
        assert!((lerp(10.0, 20.0, 1.0) - 20.0).abs() < 1e-6);
    }
}
