//! Terminal color themes and the crate's color types.
//!
//! A [`Theme`] is a small TOML file (`foreground`, `background`, optional `cursor` and
//! `selection`, 16 `ansi` colors as `"#rrggbb"`). `theme = "<name>"` in `config.toml` picks
//! `<config dir>/themes/<name>.toml` if it exists, else one of [`BUILTIN_THEMES`], embedded from
//! `src/themes/`. Themes color the terminal only; the app's own chrome comes from `ui_theme` and
//! `[colors.*]` ([`crate::style`]). [`Rgb`] and [`Rgba`] are the color values both use.

use std::fmt;
use std::path::Path;

use serde::Deserialize;
use serde::de::{self, Deserializer};

/// An opaque color, written `#rrggbb` (case-insensitive) in theme files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        parse_hex(&s).ok_or_else(|| {
            de::Error::custom(format!("expected a color like \"#1e222a\", got {s:?}"))
        })
    }
}

/// Parses exactly `#rrggbb`; `None` for anything else (no shorthand, no alpha, no surrounding
/// spaces).
fn parse_hex(s: &str) -> Option<Rgb> {
    let hex = s.strip_prefix('#')?;
    // `is_ascii` makes the byte slicing below safe: every index is a char boundary.
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
}

impl Rgb {
    /// `#rrggbb`; `None` if `s` isn't exactly that.
    pub fn parse(s: &str) -> Option<Self> {
        parse_hex(s)
    }
}

/// A color with alpha: `#rrggbb` (opaque) or `#rrggbbaa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba(pub u8, pub u8, pub u8, pub u8);

impl Rgba {
    /// `#rrggbb` (alpha 255) or `#rrggbbaa`, ignoring surrounding spaces; `None` otherwise.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.trim().strip_prefix('#')?;
        if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let alpha = if hex.len() == 8 { byte(6)? } else { 255 };
        Some(Self(byte(0)?, byte(2)?, byte(4)?, alpha))
    }
}

impl<'de> Deserialize<'de> for Rgba {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).ok_or_else(|| {
            de::Error::custom(format!(
                "expected a color like \"#1e222a\" or \"#1e222a80\", got {s:?}"
            ))
        })
    }
}

/// `#rrggbb` when opaque, else `#rrggbbaa`, so writing a color back gives the shortest form
/// [`Rgba::parse`] reads.
impl fmt::Display for Rgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)?;
        if self.3 != 255 {
            write!(f, "{:02x}", self.3)?;
        }
        Ok(())
    }
}

/// `#rrggbb`, lowercase.
impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// Terminal colors: the palette programs draw with, plus the cursor and selection. The app's
/// own UI takes only the background from here (for pane cards); the rest of its chrome comes
/// from `ui_theme`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    /// Default text color.
    pub foreground: Rgb,
    /// Default background color.
    pub background: Rgb,
    /// Defaults to the foreground color.
    pub cursor: Option<Rgb>,
    /// Background of selected text and search matches. Defaults to a blend of the background
    /// and the theme's blue.
    pub selection: Option<Rgb>,
    /// Black, red, green, yellow, blue, magenta, cyan, white, then their bright variants.
    pub ansi: [Rgb; 16],
}

/// The themes compiled in, as `(name, TOML source)`.
pub const BUILTIN_THEMES: [(&str, &str); 2] = [
    ("macaroni-dark", include_str!("themes/macaroni-dark.toml")),
    ("macaroni-light", include_str!("themes/macaroni-light.toml")),
];

/// The theme used when `config.toml` doesn't name one.
pub const DEFAULT_THEME: &str = "macaroni-dark";

impl Theme {
    /// The built-in theme called `name`; `None` if there isn't one.
    ///
    /// # Panics
    ///
    /// If that built-in theme's TOML doesn't parse, which a test rules out.
    pub fn builtin(name: &str) -> Option<Self> {
        BUILTIN_THEMES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, source)| toml::from_str(source).expect("built-in themes parse"))
    }

    /// `<dir>/themes/<name>.toml` if it exists, otherwise a built-in theme of that name.
    ///
    /// A user file takes precedence even over a built-in of the same name. Errors name the file
    /// when it doesn't parse, or list the built-in names when neither exists.
    pub fn resolve(name: &str, dir: &Path) -> Result<Self, String> {
        let path = dir.join("themes").join(format!("{name}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(source) => toml::from_str(&source).map_err(|e| format!("{}: {e}", path.display())),
            Err(_) => Self::builtin(name).ok_or_else(|| {
                let builtins: Vec<_> = BUILTIN_THEMES.iter().map(|(n, _)| *n).collect();
                format!(
                    "unknown theme {name:?}: no {} and no built-in theme by that name (built-in: {})",
                    path.display(),
                    builtins.join(", ")
                )
            }),
        }
    }

    /// Every theme `theme =` can name: the built-in ones, then `<dir>/themes/*.toml` by name.
    ///
    /// User themes are sorted, and one that shares a built-in's name is listed once (as the
    /// built-in's entry, though [`Theme::resolve`] would load the user's file). Reads the themes
    /// directory; a missing one just means no user themes.
    pub fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = BUILTIN_THEMES.iter().map(|(n, _)| n.to_string()).collect();
        let mut user: Vec<String> = std::fs::read_dir(dir.join("themes"))
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                (path.extension()? == "toml")
                    .then(|| path.file_stem()?.to_str().map(str::to_owned))?
            })
            .filter(|name| !names.contains(name))
            .collect();
        user.sort();
        names.extend(user);
        names
    }

    /// The cursor color: the `cursor` field if set, else the foreground.
    pub fn cursor(&self) -> Rgb {
        self.cursor.unwrap_or(self.foreground)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse() {
        for (name, _) in BUILTIN_THEMES {
            assert!(Theme::builtin(name).is_some(), "{name}");
        }
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex("#1e222A"), Some(Rgb(0x1e, 0x22, 0x2a)));
        assert_eq!(parse_hex("1e222a"), None);
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(Rgb(1, 2, 255).to_string(), "#0102ff");
        assert_eq!(Rgba::parse("#0102ff"), Some(Rgba(1, 2, 255, 255)));
        assert_eq!(Rgba::parse("#0102ff80"), Some(Rgba(1, 2, 255, 128)));
        assert_eq!(Rgba::parse("#0102f"), None);
        assert_eq!(Rgba(1, 2, 255, 128).to_string(), "#0102ff80");
        assert_eq!(Rgba(1, 2, 255, 255).to_string(), "#0102ff");
    }

    #[test]
    fn user_theme_overrides_and_errors() {
        let dir = std::env::temp_dir().join(format!("macaroni-theme-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        let source = BUILTIN_THEMES[0]
            .1
            .replace("background = \"#121314\"", "background = \"#000000\"");
        std::fs::write(dir.join("themes/mine.toml"), source).unwrap();
        assert_eq!(
            Theme::resolve("mine", &dir).unwrap().background,
            Rgb(0, 0, 0)
        );

        std::fs::write(dir.join("themes/bad.toml"), "foreground = \"red\"").unwrap();
        assert!(
            Theme::resolve("bad", &dir)
                .unwrap_err()
                .contains("bad.toml")
        );
        assert!(
            Theme::resolve("nope", &dir)
                .unwrap_err()
                .contains("unknown theme")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
