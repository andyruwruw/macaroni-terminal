//! Terminal sessions for macaroni: a program running in a PTY, turned into a grid the app can
//! copy and draw.
//!
//! [`Session`] owns the PTY, the child process and the reader, waiter and sync threads, and
//! reports [`SessionEvent`]s through a callback. Output goes through [`OscSniffer`] (agent
//! notifications and OSC 7 directories) and then the [`Emulator`] ([`AlacrittyEmulator`], the
//! only implementation).
//! The app reads the screen with [`Session::snapshot`], which copies it into a [`Snapshot`] made of
//! this crate's own types in `grid`, so no `alacritty_terminal` type leaks out.
//!
//! The pure helpers encode input for the program: [`encode_key`] and [`encode_paste`] (`keys`),
//! [`encode_mouse`] (`mouse`), and [`url_at`] / [`is_openable`] find and gate links (`links`).
//! [`shell`] picks the default shell and lists the installed ones. The crate knows nothing about
//! workspaces, tabs or iced.

mod emulator;
mod grid;
mod keys;
mod links;
mod mouse;
mod osc;
mod session;
pub mod shell;

pub use emulator::{AlacrittyEmulator, Emulator, EmulatorEvent};
pub use grid::{
    Cell, CellFlags, CellSide, Color, Cursor, CursorShape, DefaultColors, GridPoint, GridSize,
    Modes, Rgb, Scroll, SearchDirection, SelectionKind, Snapshot,
};
pub use keys::{Key, Mods, encode_key, encode_paste};
pub use links::{Link, is_openable, url_at};
pub use mouse::{MouseButton, MouseEvent, encode_mouse};
pub use osc::{Notification, OscEvent, OscSniffer};
pub use session::{Session, SessionConfig, SessionEvent};
