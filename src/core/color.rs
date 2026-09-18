//! Colors used by the cleaning overlay (DESIGN.md §4).
//!
//! The overlay's ink (text, dots, bar) is always the inverse of the cleaning color, expressed as
//! opacity rather than a fixed gray, and does not follow the system's light/dark appearance: the
//! overlay replaces the screen, so it sets its own contrast (DESIGN.md §4, note).

use std::fmt;

/// A 24-bit RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    /// Red channel, 0-255.
    pub r: u8,
    /// Green channel, 0-255.
    pub g: u8,
    /// Blue channel, 0-255.
    pub b: u8,
}

impl Rgb {
    /// Pure black, `#000000`.
    pub const BLACK: Self = Self::new(0, 0, 0);
    /// Pure white, `#FFFFFF`.
    pub const WHITE: Self = Self::new(255, 255, 255);

    /// Creates a color from its 8-bit channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Relative luminance in `[0.0, 1.0]`, using the sRGB (Rec. 601-ish, perceptual) weights
    /// `0.299 R + 0.587 G + 0.114 B`. Good enough to decide "is this background light or dark"
    /// without pulling in a full color-management stack.
    #[must_use]
    pub fn luminance(self) -> f32 {
        let r = f32::from(self.r) / 255.0;
        let g = f32::from(self.g) / 255.0;
        let b = f32::from(self.b) / 255.0;
        0.299f32.mul_add(r, 0.587f32.mul_add(g, 0.114 * b))
    }

    /// Whether this color reads as visually "light" (luminance above the midpoint).
    #[must_use]
    pub fn is_light(self) -> bool {
        self.luminance() > 0.5
    }

    /// Packs this color into the 0x00RRGGBB word format `softbuffer` expects.
    #[must_use]
    pub fn to_u32(self) -> u32 {
        (u32::from(self.r) << 16) | (u32::from(self.g) << 8) | u32::from(self.b)
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// The two cleaning screen colors offered by the app (DESIGN.md §4, §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CleaningColor {
    /// Pure black cleaning screen.
    #[default]
    Black,
    /// Pure white cleaning screen.
    White,
}

impl CleaningColor {
    /// The screen's fill color.
    #[must_use]
    pub fn rgb(self) -> Rgb {
        match self {
            Self::Black => Rgb::BLACK,
            Self::White => Rgb::WHITE,
        }
    }

    /// The setting file's string form (`"black"` / `"white"`).
    #[must_use]
    pub fn as_setting_str(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::White => "white",
        }
    }

    /// Parses a setting string, returning `None` for anything unrecognized (callers fall back to
    /// [`CleaningColor::default`]).
    #[must_use]
    pub fn from_setting_str(s: &str) -> Option<Self> {
        match s {
            "black" => Some(Self::Black),
            "white" => Some(Self::White),
            _ => None,
        }
    }
}

/// Picks the overlay ink color for legibility against `background`, by luminance
/// (DESIGN.md §4, §8): white ink on dark backgrounds, black ink on light ones — used both for
/// the normal black/white cleaning screen and for the dead-pixel test's red/green/blue/white/
/// black steps.
#[must_use]
pub fn ink_for(background: Rgb) -> Rgb {
    if background.is_light() {
        Rgb::BLACK
    } else {
        Rgb::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luminance_of_black_and_white() {
        assert!(Rgb::BLACK.luminance() < 0.01);
        assert!(Rgb::WHITE.luminance() > 0.99);
    }

    #[test]
    fn ink_is_white_on_black() {
        assert_eq!(ink_for(Rgb::BLACK), Rgb::WHITE);
    }

    #[test]
    fn ink_is_black_on_white() {
        assert_eq!(ink_for(Rgb::WHITE), Rgb::BLACK);
    }

    #[test]
    fn ink_is_white_on_red_blue_black() {
        assert_eq!(ink_for(Rgb::new(255, 0, 0)), Rgb::WHITE);
        assert_eq!(ink_for(Rgb::new(0, 0, 255)), Rgb::WHITE);
    }

    #[test]
    fn ink_is_black_on_green_and_white() {
        assert_eq!(ink_for(Rgb::new(0, 255, 0)), Rgb::BLACK);
        assert_eq!(ink_for(Rgb::WHITE), Rgb::BLACK);
    }

    #[test]
    fn cleaning_color_round_trips_through_settings_string() {
        for color in [CleaningColor::Black, CleaningColor::White] {
            let s = color.as_setting_str();
            assert_eq!(CleaningColor::from_setting_str(s), Some(color));
        }
    }

    #[test]
    fn cleaning_color_rejects_unknown_strings() {
        assert_eq!(CleaningColor::from_setting_str("blue"), None);
        assert_eq!(CleaningColor::from_setting_str(""), None);
    }

    #[test]
    fn default_cleaning_color_is_black() {
        assert_eq!(CleaningColor::default(), CleaningColor::Black);
    }

    #[test]
    fn to_u32_packs_00rrggbb() {
        assert_eq!(Rgb::new(0x11, 0x22, 0x33).to_u32(), 0x0011_2233);
    }

    #[test]
    fn display_formats_as_hex() {
        assert_eq!(Rgb::new(0, 128, 255).to_string(), "#0080FF");
    }
}
