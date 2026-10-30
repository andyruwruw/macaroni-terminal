//! iced keyboard input as `macaroni_term`'s key types, for `macaroni_term::encode_key` and
//! `encode_mouse`. Used by [`crate::terminal`]; the encoders turn these into the bytes a
//! terminal program expects.

use iced::keyboard::{self, key::Named};
use macaroni_term::{Key, Mods};

/// The terminal key for an iced logical key: the first character of a character key, or a named
/// key the encoder knows. `None` for keys a terminal has no encoding for (media keys, lone
/// modifiers, F13 and up).
pub fn key(key: &keyboard::Key) -> Option<Key> {
    Some(match key {
        keyboard::Key::Character(s) => Key::Char(s.chars().next()?),
        keyboard::Key::Named(named) => match named {
            Named::Enter => Key::Enter,
            Named::Backspace => Key::Backspace,
            Named::Tab => Key::Tab,
            Named::Escape => Key::Escape,
            Named::Space => Key::Char(' '),
            Named::ArrowUp => Key::Up,
            Named::ArrowDown => Key::Down,
            Named::ArrowLeft => Key::Left,
            Named::ArrowRight => Key::Right,
            Named::Home => Key::Home,
            Named::End => Key::End,
            Named::PageUp => Key::PageUp,
            Named::PageDown => Key::PageDown,
            Named::Insert => Key::Insert,
            Named::Delete => Key::Delete,
            Named::F1 => Key::F(1),
            Named::F2 => Key::F(2),
            Named::F3 => Key::F(3),
            Named::F4 => Key::F(4),
            Named::F5 => Key::F(5),
            Named::F6 => Key::F(6),
            Named::F7 => Key::F(7),
            Named::F8 => Key::F(8),
            Named::F9 => Key::F(9),
            Named::F10 => Key::F(10),
            Named::F11 => Key::F(11),
            Named::F12 => Key::F(12),
            _ => return None,
        },
        keyboard::Key::Unidentified => return None,
    })
}

/// The modifiers the terminal encodings use. The logo key (Cmd/Win) has no terminal encoding
/// and is dropped.
pub fn mods(modifiers: keyboard::Modifiers) -> Mods {
    Mods {
        shift: modifiers.shift(),
        alt: modifiers.alt(),
        ctrl: modifiers.control(),
    }
}
