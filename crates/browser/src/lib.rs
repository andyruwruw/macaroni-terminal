//! Chromium browser panels, via CEF (the Chromium Embedded Framework).
//!
//! Each [`Browser`] is a native child window of macaroni's window that the app moves over a
//! pane with [`Browser::place`]. CEF runs its own UI thread (`multi_threaded_message_loop`), so
//! nothing here blocks iced, and every callback reaches the app through the `on_event` closure.
//! Helper processes (renderer, GPU, ...) are macaroni.exe itself, started with `--type=...`:
//! [`helper_process`] must run first thing in `main`.
//!
//! Windows only. Elsewhere CEF isn't built at all, and [`Runtime::start`] reports an
//! error, so no [`Browser`] is ever opened.
//!
//! This is the only crate that touches CEF. The platform module (`windows.rs`, or the
//! `unsupported.rs` stub) provides [`Runtime`], [`Browser`], [`helper_process`], [`wait_closed`]
//! and [`focus_window`] with the same signatures; this file holds the types both share. The app's
//! `browser.rs` and `update/browser.rs` own the panels and decide when to open, place and drop
//! them.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
pub use unsupported::*;

/// What a browser reports. Sent from CEF's UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The main frame's URL changed (navigation, redirect, in-page history).
    Address(String),
    /// The page title changed.
    Title(String),
    /// Loading started or finished, or history changed.
    Loading {
        /// Whether the page is loading.
        loading: bool,
        /// Whether there is history to go back to.
        back: bool,
        /// Whether there is history to go forward to.
        forward: bool,
    },
    /// The page took keyboard focus (the user clicked into it).
    Focused,
    /// The page asked for a new window (`target=_blank`, `window.open`); open it as a tab.
    NewWindow(String),
    /// CEF finished closing the browser after the [`Browser`] was dropped.
    Closed,
}

/// A rectangle in physical pixels, relative to the parent window's client area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

/// A key pressed while a page has focus, offered to the app before the page sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// The Windows virtual-key code (`VK_*`; letters and digits are their ASCII uppercase).
    pub code: u32,
    /// Control is held.
    pub ctrl: bool,
    /// Shift is held.
    pub shift: bool,
    /// Alt is held.
    pub alt: bool,
    /// The platform key (Windows key, Cmd) is held.
    pub meta: bool,
}
