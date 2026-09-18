//! Coordinate conversion between winit's window-local physical pixels and the session's
//! coordinate space (main-screen logical points, top-left origin —
//! `core::session::InputEvent`'s documented convention).
//!
//! The input-blocker's tap events need no conversion at all: `CGEventGetLocation` already reports
//! global points with the main display's top-left as its origin, which is exactly the space
//! `core::session::InputEvent`'s pointer variants use (see
//! `platform::ffi::event_tap::event_location`). Only winit's own `CursorMoved` (used for the
//! hover reveal, since plain pointer movement isn't blocked by the tap — `platform::input_blocker`'s
//! module docs) needs converting: it reports a window-local position in physical pixels, so it
//! needs the window's own origin added before it means the same thing as a tap event's location.

/// Converts a `WindowEvent::CursorMoved` position (window-local, physical pixels) into the
/// session's coordinate space, given the window's own outer/inner position (physical pixels, in
/// the same global, top-left-origin space the OS and `CGEventGetLocation` both use) and the
/// window's scale factor.
///
/// Pure, so it is unit-tested without a real window.
#[must_use]
pub fn cursor_to_session_point(
    window_origin_physical: (f64, f64),
    cursor_local_physical: (f64, f64),
    scale_factor: f64,
) -> (f32, f32) {
    let global_physical_x = window_origin_physical.0 + cursor_local_physical.0;
    let global_physical_y = window_origin_physical.1 + cursor_local_physical.1;
    // A scale factor of 0 or negative is not something any real display reports; guard it anyway
    // so a pathological value can't divide by zero and produce an "infinity" hover target.
    let scale = if scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a window position in points comfortably fits in f32's precision range"
    )]
    (
        (global_physical_x / scale) as f32,
        (global_physical_y / scale) as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_window_origin_to_local_position() {
        let (x, y) = cursor_to_session_point((100.0, 50.0), (20.0, 10.0), 1.0);
        assert!((x - 120.0).abs() < 1e-3);
        assert!((y - 60.0).abs() < 1e-3);
    }

    #[test]
    fn divides_by_scale_factor() {
        let (x, y) = cursor_to_session_point((200.0, 100.0), (40.0, 20.0), 2.0);
        // (200+40)/2 = 120, (100+20)/2 = 60.
        assert!((x - 120.0).abs() < 1e-3);
        assert!((y - 60.0).abs() < 1e-3);
    }

    #[test]
    fn zero_window_origin_is_a_no_op() {
        let (x, y) = cursor_to_session_point((0.0, 0.0), (33.0, 44.0), 1.0);
        assert!((x - 33.0).abs() < 1e-3);
        assert!((y - 44.0).abs() < 1e-3);
    }

    #[test]
    fn non_positive_scale_factor_falls_back_to_one_instead_of_dividing_by_zero() {
        let (x, y) = cursor_to_session_point((10.0, 10.0), (5.0, 5.0), 0.0);
        assert!((x - 15.0).abs() < 1e-3);
        assert!((y - 15.0).abs() < 1e-3);
    }
}
