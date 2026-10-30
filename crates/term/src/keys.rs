//! Keyboard and paste encoding: turns a key press or pasted text into the bytes an
//! xterm-compatible terminal would send to the program.
//!
//! Pure and unit tested. The app maps iced keys to [`Key`] and [`Mods`]
//! (`crates/app/src/input.rs`), calls [`encode_key`] / [`encode_paste`] with the session's current
//! [`Modes`], and writes the result to the PTY with [`Session::write`](crate::Session::write).

use crate::grid::Modes;

/// A key, independent of the UI toolkit. Printable keys are [`Key::Char`] with the unshifted
/// character the key produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A character key. Ctrl combinations are encoded from this, not from the platform text.
    Char(char),
    /// Enter or Return.
    Enter,
    /// Backspace.
    Backspace,
    /// Tab.
    Tab,
    /// Escape.
    Escape,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Insert.
    Insert,
    /// Delete (forward delete).
    Delete,
    /// A function key. Only F1-F12 have an encoding; others produce nothing.
    F(u8),
}

/// Modifiers held during a key press. The platform key (Cmd/Win) isn't here: it's never sent
/// to the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// Shift.
    pub shift: bool,
    /// Alt (Option on macOS).
    pub alt: bool,
    /// Control.
    pub ctrl: bool,
}

impl Mods {
    /// Whether any modifier is held, which switches keys to their `CSI 1;<mod>` forms.
    fn any(self) -> bool {
        self.shift || self.alt || self.ctrl
    }

    /// The xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

/// Encodes a key press as the bytes an xterm-compatible terminal sends.
///
/// `text` is what the platform layout produced for the press (it carries AltGr and dead-key
/// results), and is preferred over `key` for printable input without Ctrl. `modes` decides the
/// arrow-key form (`CSI` or `SS3` in application cursor mode).
///
/// Returns `None` when the key has no encoding: F13 and up, or Ctrl with a character that has
/// no control code.
pub fn encode_key(key: Key, text: Option<&str>, mods: Mods, modes: Modes) -> Option<Vec<u8>> {
    // Alt is sent as an ESC prefix ("meta sends escape"), as xterm and most terminals do.
    let esc_prefixed = |mut bytes: Vec<u8>| {
        if mods.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };

    // Cursor keys: modifiers win over application cursor mode, because the `SS3` form has no
    // room for a modifier parameter.
    let csi = |letter: char| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{letter}", mods.param()).into_bytes()
        } else if modes.app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };

    // Editing keys and F5-F12: `CSI <code> ~`, with the modifier as a second parameter.
    let tilde = |code: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[{code};{}~", mods.param()).into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };

    let bytes = match key {
        // Shift+Enter sends ESC CR, which line editors such as Claude Code's prompt read as
        // "insert a newline" (the same bytes Alt+Enter sends).
        Key::Enter if mods.shift && !mods.ctrl => b"\x1b\r".to_vec(),
        Key::Enter => esc_prefixed(b"\r".to_vec()),
        // DEL is what every modern terminal sends for Backspace; Ctrl+Backspace sends BS so
        // shells can bind it to delete a word.
        Key::Backspace => esc_prefixed(vec![if mods.ctrl { 0x08 } else { 0x7f }]),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => esc_prefixed(b"\t".to_vec()),
        Key::Escape => esc_prefixed(b"\x1b".to_vec()),
        Key::Up => csi('A'),
        Key::Down => csi('B'),
        Key::Right => csi('C'),
        Key::Left => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        // F1-F4 are `SS3 P..S` unmodified, but `CSI 1;<mod> P..S` with modifiers.
        Key::F(n @ 1..=4) => {
            let letter = char::from(b'P' + n - 1);
            if mods.any() {
                format!("\x1b[1;{}{letter}", mods.param()).into_bytes()
            } else {
                format!("\x1bO{letter}").into_bytes()
            }
        }
        // xterm's codes skip 16 and 22, a VT220 legacy.
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        Key::F(_) => return None,
        Key::Char(c) => {
            let printable = text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control));
            // Ctrl+Alt is AltGr on Windows layouts; trust the produced text then. Otherwise Ctrl
            // uses the key itself, since the platform text for Ctrl+letter is unreliable (often
            // already a control character, or nothing).
            if mods.ctrl && !(mods.alt && printable.is_some()) {
                esc_prefixed(vec![ctrl_byte(c)?])
            } else if let Some(text) = printable {
                if mods.alt && !mods.ctrl {
                    esc_prefixed(text.as_bytes().to_vec())
                } else {
                    text.as_bytes().to_vec()
                }
            } else {
                esc_prefixed(c.to_string().into_bytes())
            }
        }
    };
    Some(bytes)
}

/// The C0 control code Ctrl+`c` sends, following xterm: letters map to 1-26, and the digit row
/// doubles for the punctuation that's awkward to type (Ctrl+2 is NUL, Ctrl+3 is ESC, up to
/// Ctrl+8 as DEL). `None` for characters with no control code.
fn ctrl_byte(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Encodes pasted text, wrapping it in bracketed-paste markers when the program asked for them.
///
/// With bracketed paste, any end marker inside the text is removed so a paste can't end the
/// bracket early and have the rest run as typed input. Without it, line endings become CR, which
/// is what Enter sends.
pub fn encode_paste(text: &str, modes: Modes) -> Vec<u8> {
    if modes.bracketed_paste {
        let body = text.replace("\x1b[201~", "");
        format!("\x1b[200~{body}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(key: Key, text: Option<&str>, mods: Mods) -> Vec<u8> {
        encode_key(key, text, mods, Modes::default()).unwrap()
    }

    const CTRL: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: true,
    };
    const ALT: Mods = Mods {
        shift: false,
        alt: true,
        ctrl: false,
    };

    #[test]
    fn printable_text_is_sent_as_is() {
        assert_eq!(enc(Key::Char('a'), Some("a"), Mods::default()), b"a");
        assert_eq!(
            enc(Key::Char('e'), Some("é"), Mods::default()),
            "é".as_bytes()
        );
    }

    #[test]
    fn ctrl_letters_become_control_bytes() {
        assert_eq!(enc(Key::Char('c'), Some("\u{3}"), CTRL), [3]);
        assert_eq!(enc(Key::Char('['), None, CTRL), [0x1b]);
        assert_eq!(enc(Key::Char(' '), Some(" "), CTRL), [0]);
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(enc(Key::Char('b'), Some("b"), ALT), b"\x1bb");
    }

    #[test]
    fn altgr_text_wins_over_ctrl() {
        let altgr = Mods {
            shift: false,
            alt: true,
            ctrl: true,
        };
        assert_eq!(enc(Key::Char('q'), Some("@"), altgr), b"@");
    }

    #[test]
    fn arrows_follow_cursor_mode_and_modifiers() {
        assert_eq!(enc(Key::Up, None, Mods::default()), b"\x1b[A");
        let app = Modes {
            app_cursor: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_key(Key::Up, None, Mods::default(), app).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(enc(Key::Right, None, CTRL), b"\x1b[1;5C");
    }

    #[test]
    fn named_keys() {
        assert_eq!(enc(Key::Backspace, None, Mods::default()), [0x7f]);
        assert_eq!(enc(Key::Enter, None, Mods::default()), b"\r");
        let shift = Mods {
            shift: true,
            ..Mods::default()
        };
        assert_eq!(enc(Key::Enter, None, shift), b"\x1b\r");
        assert_eq!(
            enc(
                Key::Tab,
                None,
                Mods {
                    shift: true,
                    ..Mods::default()
                }
            ),
            b"\x1b[Z"
        );
        assert_eq!(enc(Key::Delete, None, Mods::default()), b"\x1b[3~");
        assert_eq!(enc(Key::F(1), None, Mods::default()), b"\x1bOP");
        assert_eq!(enc(Key::F(12), None, Mods::default()), b"\x1b[24~");
    }

    #[test]
    fn paste_is_bracketed_only_when_requested() {
        assert_eq!(encode_paste("a\nb", Modes::default()), b"a\rb");
        let bracketed = Modes {
            bracketed_paste: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_paste("x\x1b[201~y", bracketed),
            b"\x1b[200~xy\x1b[201~"
        );
    }
}
