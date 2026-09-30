//! RGBA color with parsing for the two formats we meet: TOML hex strings and
//! Hyprland's `AARRGGBB` hex from `getoption`.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Straight (non-premultiplied) RGBA, each channel in `0.0..=1.0`.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ColorError {
    #[error("expected #RRGGBB or #RRGGBBAA, got {0:?}")]
    BadHex(String),
    #[error("Hyprland gradient string is empty")]
    EmptyGradient,
}

impl Rgba {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        )
    }

    /// Parse `#RRGGBB`, `#RRGGBBAA`, `RRGGBB` or `RRGGBBAA` (case-insensitive).
    pub fn parse_hex(s: &str) -> Result<Self, ColorError> {
        let hex = s.trim().trim_start_matches('#');
        let bad = || ColorError::BadHex(s.to_string());
        if !hex.is_ascii() {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        match hex.len() {
            6 => Ok(Self::from_rgba8(byte(0)?, byte(2)?, byte(4)?, 255)),
            8 => Ok(Self::from_rgba8(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => Err(bad()),
        }
    }

    /// Parse the first color of a Hyprland gradient as printed by
    /// `hyprctl -j getoption general:col.active_border`, e.g. `"77919191 0deg"`
    /// or `"ff33ccff ff00ff99 45deg"`. Each color is `AARRGGBB` hex.
    pub fn parse_hyprland_gradient(s: &str) -> Result<Self, ColorError> {
        let first = s
            .split_whitespace()
            .find(|tok| !tok.ends_with("deg"))
            .ok_or(ColorError::EmptyGradient)?;
        let first = first.trim_start_matches("0x");
        let bad = || ColorError::BadHex(s.to_string());
        // Hyprland prints `{:x}` without zero padding, so an alpha below 0x10
        // yields fewer than eight digits.
        if first.is_empty() || first.len() > 8 {
            return Err(bad());
        }
        let argb = u32::from_str_radix(first, 16).map_err(|_| bad())?;
        Ok(Self::from_rgba8(
            ((argb >> 16) & 0xff) as u8,
            ((argb >> 8) & 0xff) as u8,
            (argb & 0xff) as u8,
            ((argb >> 24) & 0xff) as u8,
        ))
    }

    /// Premultiplied `[r, g, b, a]`, what a blending shader wants.
    pub fn premultiplied(self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        write!(f, "#{:02x}{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b), c(self.a))
    }
}

impl Serialize for Rgba {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgba::parse_hex(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_hex() {
        assert_eq!(Rgba::parse_hex("#ff0000"), Ok(Rgba::new(1.0, 0.0, 0.0, 1.0)));
        assert_eq!(Rgba::parse_hex("00FF0080").unwrap().g, 1.0);
        assert!((Rgba::parse_hex("#00ff0080").unwrap().a - 128.0 / 255.0).abs() < 1e-6);
        assert!(Rgba::parse_hex("#fff").is_err());
        assert!(Rgba::parse_hex("zzzzzz").is_err());
        assert!(Rgba::parse_hex("#ff00é0").is_err(), "multi-byte input must not panic");
        assert!(Rgba::parse_hex("#ffffffé").is_err());
    }

    #[test]
    fn parses_hyprland_gradient_first_color_as_aarrggbb() {
        // rgba(91919177) in hyprland.conf prints as "77919191 0deg".
        let c = Rgba::parse_hyprland_gradient("77919191 0deg").unwrap();
        assert_eq!(c, Rgba::from_rgba8(0x91, 0x91, 0x91, 0x77));
        let c = Rgba::parse_hyprland_gradient("ff33ccff ff00ff99 45deg").unwrap();
        assert_eq!(c, Rgba::from_rgba8(0x33, 0xcc, 0xff, 0xff));
        // Alpha 0x0a prints unpadded as seven digits.
        let c = Rgba::parse_hyprland_gradient("a112233 90deg").unwrap();
        assert_eq!(c, Rgba::from_rgba8(0x11, 0x22, 0x33, 0x0a));
        assert!(Rgba::parse_hyprland_gradient("0deg").is_err());
        assert!(Rgba::parse_hyprland_gradient("123456789 0deg").is_err());
        assert!(Rgba::parse_hyprland_gradient("").is_err());
    }

    #[test]
    fn display_roundtrips() {
        let c = Rgba::from_rgba8(0x0d, 0xb7, 0xd4, 0x55);
        assert_eq!(c.to_string(), "#0db7d455");
        assert_eq!(Rgba::parse_hex(&c.to_string()).unwrap(), c);
    }
}
