//! The one TOML config file: `~/.config/hyprshell/hyprshell.toml`.
//!
//! Typed, validated, hand-editable. Fields that default to a Hyprland option
//! are `Option<T>`; they are resolved in the view-model, never here.

use crate::color::Rgba;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Environment variable that overrides the config path (useful while the
/// repo is not yet stowed as `~/.config/hyprshell`).
pub const CONFIG_ENV: &str = "HYPRSHELL_CONFIG";
pub const CONFIG_FILE: &str = "hyprshell.toml";

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub clock: Clock,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Clock {
    /// Side of the square surface in logical pixels.
    pub size: u32,
    /// Where on each monitor the surface is anchored.
    pub position: Anchor,
    /// Distance in logical pixels from the anchored edges.
    pub margin: u32,
    /// Draw the second hand (costs one redraw per second).
    pub show_seconds: bool,
    /// Ease hand movement on each tick instead of jumping. Costs a short burst
    /// of extra frames after every tick, so off by default.
    pub animation: bool,
    pub face: Face,
    pub hands: Hands,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Face {
    pub color: Rgba,
    /// Default: Hyprland `general:col.active_border` (first gradient stop).
    pub border_color: Option<Rgba>,
    /// Default: Hyprland `general:border_size`.
    pub border_width: Option<u32>,
    /// Corner radius in logical pixels. Default: Hyprland `decoration:rounding`.
    /// At `size / 2` or more the face is a circle.
    pub rounding: Option<u32>,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(from = "HandsPatch")]
pub struct Hands {
    pub hour: Hand,
    pub minute: Hand,
    pub second: Hand,
}

#[derive(Clone, PartialEq, Debug, Serialize)]
pub struct Hand {
    /// Fraction of the face radius, `0.0 < length <= 1.0`.
    pub length: f32,
    /// Thickness in logical pixels.
    pub width: f32,
    pub color: Rgba,
}

/// Each hand has its own defaults, so a partial `[clock.hands.second]` table
/// must fill in from the *second* hand's defaults, not from a generic `Hand`.
/// Serde's container default cannot express that; this patch type can.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HandsPatch {
    hour: HandPatch,
    minute: HandPatch,
    second: HandPatch,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct HandPatch {
    length: Option<f32>,
    width: Option<f32>,
    color: Option<Rgba>,
}

impl HandPatch {
    fn over(self, base: Hand) -> Hand {
        Hand {
            length: self.length.unwrap_or(base.length),
            width: self.width.unwrap_or(base.width),
            color: self.color.unwrap_or(base.color),
        }
    }
}

impl From<HandsPatch> for Hands {
    fn from(p: HandsPatch) -> Self {
        let d = Hands::default();
        Hands { hour: p.hour.over(d.hour), minute: p.minute.over(d.minute), second: p.second.over(d.second) }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            size: 220,
            position: Anchor::TopRight,
            margin: 24,
            show_seconds: true,
            animation: false,
            face: Face::default(),
            hands: Hands::default(),
        }
    }
}

impl Default for Face {
    fn default() -> Self {
        Self {
            color: Rgba::from_rgba8(0x1e, 0x1e, 0x2e, 0xcc),
            border_color: None,
            border_width: None,
            rounding: None,
        }
    }
}

impl Default for Hands {
    fn default() -> Self {
        let ink = Rgba::from_rgba8(0xcd, 0xd6, 0xf4, 0xff);
        Self {
            hour: Hand { length: 0.5, width: 6.0, color: ink },
            minute: Hand { length: 0.75, width: 4.0, color: ink },
            second: Hand { length: 0.85, width: 2.0, color: Rgba::from_rgba8(0xf3, 0x8b, 0xa8, 0xff) },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{path}: invalid config:\n  {}", .problems.join("\n  "))]
    Invalid { path: PathBuf, problems: Vec<String> },
}

impl Config {
    /// `$HYPRSHELL_CONFIG`, else `$XDG_CONFIG_HOME/hyprshell/hyprshell.toml`,
    /// else `~/.config/hyprshell/hyprshell.toml`.
    pub fn default_path() -> PathBuf {
        if let Some(p) = std::env::var_os(CONFIG_ENV) {
            return PathBuf::from(p);
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("hyprshell").join(CONFIG_FILE)
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text).map_err(|e| match e {
            ParseError::Toml(source) => ConfigError::Parse { path: path.to_path_buf(), source },
            ParseError::Invalid(problems) => ConfigError::Invalid { path: path.to_path_buf(), problems },
        })
    }

    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let config: Config = toml::from_str(text).map_err(ParseError::Toml)?;
        let problems = config.validate();
        if problems.is_empty() {
            Ok(config)
        } else {
            Err(ParseError::Invalid(problems))
        }
    }

    /// Every problem, not just the first, so one edit round fixes them all.
    pub fn validate(&self) -> Vec<String> {
        let c = &self.clock;
        let mut p = Vec::new();
        if !(16..=4096).contains(&c.size) {
            p.push(format!("clock.size must be between 16 and 4096, got {}", c.size));
        }
        if c.margin > 4096 {
            p.push(format!("clock.margin must be at most 4096, got {}", c.margin));
        }
        if let Some(w) = c.face.border_width
            && w > c.size / 2
        {
            p.push(format!("clock.face.border_width {w} is larger than half the size"));
        }
        for (name, hand) in [("hour", &c.hands.hour), ("minute", &c.hands.minute), ("second", &c.hands.second)] {
            if !(hand.length > 0.0 && hand.length <= 1.0) {
                p.push(format!("clock.hands.{name}.length must be in (0, 1], got {}", hand.length));
            }
            if !(hand.width > 0.0 && hand.width <= c.size as f32) {
                p.push(format!("clock.hands.{name}.width must be in (0, size], got {}", hand.width));
            }
        }
        p
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    Toml(toml::de::Error),
    #[error("invalid config:\n  {}", .0.join("\n  "))]
    Invalid(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_all_defaults() {
        let c = Config::parse("").unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(c.clock.position, Anchor::TopRight);
        assert_eq!(c.clock.face.border_width, None);
    }

    #[test]
    fn partial_override_keeps_other_defaults() {
        let c = Config::parse(
            r##"
            [clock]
            size = 300
            position = "bottom-left"
            show_seconds = false
            [clock.face]
            border_width = 3
            border_color = "#ff0000"
            [clock.hands.second]
            color = "#00ff00"
            "##,
        )
        .unwrap();
        assert_eq!(c.clock.size, 300);
        assert_eq!(c.clock.position, Anchor::BottomLeft);
        assert!(!c.clock.show_seconds);
        assert_eq!(c.clock.face.border_width, Some(3));
        assert_eq!(c.clock.face.border_color, Some(Rgba::from_rgba8(255, 0, 0, 255)));
        assert_eq!(c.clock.face.rounding, None);
        assert_eq!(c.clock.hands.second.color, Rgba::from_rgba8(0, 255, 0, 255));
        assert_eq!(c.clock.hands.second.length, Hands::default().second.length);
        assert_eq!(c.clock.hands.hour, Hands::default().hour);
    }

    #[test]
    fn unknown_field_is_an_error() {
        let err = Config::parse("[clock]\nsiez = 3\n").unwrap_err();
        assert!(matches!(err, ParseError::Toml(_)), "{err}");
        assert!(err.to_string().contains("siez"), "{err}");
    }

    #[test]
    fn bad_type_reports_span() {
        let err = Config::parse("[clock]\nsize = \"big\"\n").unwrap_err();
        let ParseError::Toml(e) = err else { panic!() };
        assert!(e.span().is_some());
    }

    #[test]
    fn validation_collects_every_problem() {
        let err = Config::parse("[clock]\nsize = 8\n[clock.hands.hour]\nlength = 2.0\nwidth = 0\n").unwrap_err();
        let ParseError::Invalid(p) = err else { panic!("{err}") };
        assert_eq!(p.len(), 3, "{p:?}");
    }

    #[test]
    fn default_serializes_and_parses_back() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), Config::default());
    }
}
