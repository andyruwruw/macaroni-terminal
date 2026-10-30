//! Encodes mouse events for programs that enabled mouse reporting (vim, htop, tmux, less).
//!
//! Which events are sent depends on the mode the program set: 1000 clicks only, 1002 clicks plus
//! drags, 1003 all motion. The encoding is SGR (1006) when enabled, which has no coordinate
//! limit; otherwise the X10-style `ESC [ M` form, with UTF-8 coordinates under 1005.
//!
//! Pure and unit tested. The terminal widget calls [`encode_mouse`] when
//! [`Modes::mouse_reporting`] is on and writes the bytes to the session instead of selecting or
//! scrolling locally.

use crate::grid::{GridPoint, Modes};
use crate::keys::Mods;

/// A mouse button, with the wheel reported as buttons 4 and 5 as xterm does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The primary button.
    Left,
    /// The middle button (wheel click).
    Middle,
    /// The secondary button.
    Right,
    /// One wheel step away from the user.
    WheelUp,
    /// One wheel step towards the user.
    WheelDown,
}

/// What the mouse did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    /// A button went down (or the wheel turned one step).
    Press(MouseButton),
    /// A button went up. Wheel releases aren't reported.
    Release(MouseButton),
    /// The pointer moved to a new cell, with the button held if any.
    Motion(Option<MouseButton>),
}

/// Bytes for `event` at `at`, or `None` when the program's mouse mode doesn't ask for it (or,
/// in the X10 encoding, the position can't be represented).
///
/// `at` is the cell under the pointer in visible-screen coordinates; reports are 1-based.
pub fn encode_mouse(event: MouseEvent, at: GridPoint, mods: Mods, modes: Modes) -> Option<Vec<u8>> {
    if !modes.mouse_reporting() {
        return None;
    }
    let base = |button: MouseButton| match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };
    // The button code: low bits are the button, +32 marks motion, then 4/8/16 for
    // Shift/Alt(Meta)/Ctrl. Wheel buttons start at 64.
    let (code, release) = match event {
        MouseEvent::Press(button) => (base(button), false),
        MouseEvent::Release(MouseButton::WheelUp | MouseButton::WheelDown) => return None,
        // Only SGR says which button was released (with a final `m`); the older encodings
        // send button 3 for any release.
        MouseEvent::Release(button) => (if modes.sgr_mouse { base(button) } else { 3 }, true),
        MouseEvent::Motion(held) => {
            let wanted = modes.mouse_motion || (modes.mouse_drag && held.is_some());
            if !wanted {
                return None;
            }
            // Motion with no button held reuses the "released" button 3.
            (32 + held.map_or(3, base), false)
        }
    };
    let code =
        code + 4 * u32::from(mods.shift) + 8 * u32::from(mods.alt) + 16 * u32::from(mods.ctrl);
    let (x, y) = (at.col as u32 + 1, at.row as u32 + 1);

    if modes.sgr_mouse {
        let end = if release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }

    // X10: each value is one byte offset by 32, so columns and rows past 223 can't be sent and
    // the event is dropped. UTF-8 mode (1005) stretches that by encoding each value as a
    // character of up to two bytes.
    let mut out = b"\x1b[M".to_vec();
    for value in [code, x, y] {
        let value = value + 32;
        if modes.utf8_mouse {
            let c = char::from_u32(value).filter(|_| value < 2048)?;
            let mut buf = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
        } else {
            out.push(u8::try_from(value).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: GridPoint = GridPoint { row: 4, col: 9 };

    fn modes(f: impl FnOnce(&mut Modes)) -> Modes {
        let mut m = Modes::default();
        f(&mut m);
        m
    }

    fn sgr() -> Modes {
        modes(|m| {
            m.mouse_click = true;
            m.sgr_mouse = true;
        })
    }

    #[test]
    fn nothing_without_a_mouse_mode() {
        let m = modes(|m| m.sgr_mouse = true);
        assert_eq!(
            encode_mouse(MouseEvent::Press(MouseButton::Left), AT, Mods::default(), m),
            None
        );
    }

    #[test]
    fn sgr_press_release_and_wheel() {
        let enc = |e| encode_mouse(e, AT, Mods::default(), sgr()).unwrap();
        assert_eq!(enc(MouseEvent::Press(MouseButton::Left)), b"\x1b[<0;10;5M");
        assert_eq!(
            enc(MouseEvent::Release(MouseButton::Left)),
            b"\x1b[<0;10;5m"
        );
        assert_eq!(enc(MouseEvent::Press(MouseButton::Right)), b"\x1b[<2;10;5M");
        assert_eq!(
            enc(MouseEvent::Press(MouseButton::WheelDown)),
            b"\x1b[<65;10;5M"
        );
        assert_eq!(
            encode_mouse(
                MouseEvent::Release(MouseButton::WheelUp),
                AT,
                Mods::default(),
                sgr()
            ),
            None
        );
    }

    #[test]
    fn modifiers_add_to_the_code() {
        let mods = Mods {
            shift: false,
            alt: true,
            ctrl: true,
        };
        assert_eq!(
            encode_mouse(MouseEvent::Press(MouseButton::Left), AT, mods, sgr()).unwrap(),
            b"\x1b[<24;10;5M"
        );
    }

    #[test]
    fn motion_follows_the_mode() {
        let held = MouseEvent::Motion(Some(MouseButton::Left));
        let free = MouseEvent::Motion(None);
        assert_eq!(encode_mouse(held, AT, Mods::default(), sgr()), None);

        let drag = modes(|m| {
            m.mouse_drag = true;
            m.sgr_mouse = true;
        });
        assert_eq!(
            encode_mouse(held, AT, Mods::default(), drag).unwrap(),
            b"\x1b[<32;10;5M"
        );
        assert_eq!(encode_mouse(free, AT, Mods::default(), drag), None);

        let any = modes(|m| {
            m.mouse_motion = true;
            m.sgr_mouse = true;
        });
        assert_eq!(
            encode_mouse(free, AT, Mods::default(), any).unwrap(),
            b"\x1b[<35;10;5M"
        );
    }

    #[test]
    fn x10_and_utf8_encodings() {
        let x10 = modes(|m| m.mouse_click = true);
        assert_eq!(
            encode_mouse(
                MouseEvent::Press(MouseButton::Left),
                AT,
                Mods::default(),
                x10
            )
            .unwrap(),
            [0x1b, b'[', b'M', 32, 32 + 10, 32 + 5]
        );
        assert_eq!(
            encode_mouse(
                MouseEvent::Release(MouseButton::Left),
                AT,
                Mods::default(),
                x10
            )
            .unwrap(),
            [0x1b, b'[', b'M', 32 + 3, 32 + 10, 32 + 5]
        );
        let far = GridPoint { row: 0, col: 300 };
        assert_eq!(
            encode_mouse(
                MouseEvent::Press(MouseButton::Left),
                far,
                Mods::default(),
                x10
            ),
            None
        );
        let utf8 = modes(|m| {
            m.mouse_click = true;
            m.utf8_mouse = true;
        });
        let bytes = encode_mouse(
            MouseEvent::Press(MouseButton::Left),
            far,
            Mods::default(),
            utf8,
        )
        .unwrap();
        assert_eq!(&bytes[..4], [0x1b, b'[', b'M', 32]);
        assert_eq!(std::str::from_utf8(&bytes[4..]).unwrap(), "\u{14d}!");
    }
}
