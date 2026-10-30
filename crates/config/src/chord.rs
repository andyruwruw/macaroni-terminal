//! Key chords as written in `[keybindings]`: `ctrl+shift+t`, `cmd+alt+left`, `f12`.
//!
//! [`Chord`] parses with [`str::parse`] (case-insensitive, spaces around `+` allowed) into
//! [`Modifiers`] plus one [`ChordKey`]. [`crate::Config::parse`] parses the user's keys with it,
//! and the app's keymap parses its built-in defaults with the same parser, then matches chords
//! against iced key events.

use std::fmt;
use std::str::FromStr;

/// The modifier keys a chord holds down. Matching in the app is exact: a chord with only `ctrl`
/// doesn't fire for Ctrl+Shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    /// `ctrl` or `control`.
    pub ctrl: bool,
    /// `shift`.
    pub shift: bool,
    /// `alt`, `option` or `opt`.
    pub alt: bool,
    /// Cmd on macOS, the Windows key elsewhere: `cmd`, `command`, `super`, `win`, `logo` or
    /// `meta`.
    pub logo: bool,
}

/// A key with a name rather than a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    /// `tab`.
    Tab,
    /// `enter` or `return`.
    Enter,
    /// `escape` or `esc`.
    Escape,
    /// `space`.
    Space,
    /// `backspace`.
    Backspace,
    /// `delete` or `del`.
    Delete,
    /// `insert` or `ins`.
    Insert,
    /// `home`.
    Home,
    /// `end`.
    End,
    /// `pageup` or `pgup`.
    PageUp,
    /// `pagedown` or `pgdn`.
    PageDown,
    /// `left`.
    Left,
    /// `right`.
    Right,
    /// `up`.
    Up,
    /// `down`.
    Down,
    /// `f1` to `f12`; the number is 1 to 12.
    F(u8),
}

/// The non-modifier key of a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChordKey {
    /// A letter (always lowercase) or a punctuation key, named by its unshifted US-layout
    /// character.
    Char(char),
    /// `0` to `9` on the main row. The app matches digits by physical key, so they work on
    /// layouts where the digit row is shifted.
    Digit(u8),
    /// A [`NamedKey`].
    Named(NamedKey),
}

/// A key combination such as `ctrl+shift+t` or `cmd+alt+left`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    /// The modifiers held down.
    pub mods: Modifiers,
    /// The key pressed with them.
    pub key: ChordKey,
}

/// Why a chord didn't parse: a readable message naming the bad part and the whole chord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChordError(pub String);

impl fmt::Display for ChordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.trim().to_ascii_lowercase();
        let parts: Vec<&str> = lower.split('+').map(str::trim).collect();
        // The last part is the key, everything before it a modifier. `split` always yields at
        // least one part, so the `else` only guards the type; an empty string fails as a key.
        let Some((key, modifiers)) = parts.split_last() else {
            return Err(ChordError(format!("empty key chord {s:?}")));
        };

        let mut mods = Modifiers::default();
        for m in modifiers {
            let flag = match *m {
                "ctrl" | "control" => &mut mods.ctrl,
                "shift" => &mut mods.shift,
                "alt" | "option" | "opt" => &mut mods.alt,
                "cmd" | "command" | "super" | "win" | "logo" | "meta" => &mut mods.logo,
                other => return Err(ChordError(format!("unknown modifier {other:?} in {s:?}"))),
            };
            *flag = true;
        }

        let key =
            parse_key(key).ok_or_else(|| ChordError(format!("unknown key {key:?} in {s:?}")))?;
        Ok(Self { mods, key })
    }
}

/// The key part of a chord, already lowercased: one letter, digit or punctuation character, or
/// a key name. `None` if it's none of those (including `f0` and `f13` and up).
fn parse_key(key: &str) -> Option<ChordKey> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return match c {
            'a'..='z' => Some(ChordKey::Char(c)),
            '0'..='9' => Some(ChordKey::Digit(c as u8 - b'0')),
            ',' | '.' | '/' | ';' | '\'' | '[' | ']' | '\\' | '-' | '=' | '`' => {
                Some(ChordKey::Char(c))
            }
            _ => None,
        };
    }
    let named = match key {
        "tab" => NamedKey::Tab,
        "enter" | "return" => NamedKey::Enter,
        "escape" | "esc" => NamedKey::Escape,
        "space" => NamedKey::Space,
        "backspace" => NamedKey::Backspace,
        "delete" | "del" => NamedKey::Delete,
        "insert" | "ins" => NamedKey::Insert,
        "home" => NamedKey::Home,
        "end" => NamedKey::End,
        "pageup" | "pgup" => NamedKey::PageUp,
        "pagedown" | "pgdn" => NamedKey::PageDown,
        "left" => NamedKey::Left,
        "right" => NamedKey::Right,
        "up" => NamedKey::Up,
        "down" => NamedKey::Down,
        f if f.starts_with('f') => match f[1..].parse::<u8>() {
            Ok(n @ 1..=12) => NamedKey::F(n),
            _ => return None,
        },
        _ => return None,
    };
    Some(ChordKey::Named(named))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> Chord {
        s.parse().unwrap()
    }

    #[test]
    fn modifiers_and_keys() {
        let c = chord("Ctrl+Shift+T");
        assert!(c.mods.ctrl && c.mods.shift && !c.mods.alt && !c.mods.logo);
        assert_eq!(c.key, ChordKey::Char('t'));
        assert_eq!(chord("cmd+1").key, ChordKey::Digit(1));
        assert!(chord("cmd+1").mods.logo);
        assert_eq!(chord("ctrl+alt+left").key, ChordKey::Named(NamedKey::Left));
        assert_eq!(chord("cmd+shift+]").key, ChordKey::Char(']'));
        assert_eq!(chord("ctrl+,").key, ChordKey::Char(','));
        assert_eq!(chord("f12").key, ChordKey::Named(NamedKey::F(12)));
        assert_eq!(
            chord("ctrl + pgdn").key,
            ChordKey::Named(NamedKey::PageDown)
        );
    }

    #[test]
    fn errors_name_the_problem() {
        assert!(
            "hyper+t"
                .parse::<Chord>()
                .unwrap_err()
                .0
                .contains("modifier")
        );
        assert!(
            "ctrl+banana"
                .parse::<Chord>()
                .unwrap_err()
                .0
                .contains("banana")
        );
        assert!("f13".parse::<Chord>().is_err());
        assert!("".parse::<Chord>().is_err());
    }
}
