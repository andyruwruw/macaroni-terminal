//! Browser panels: pages shown in tabs, rendered by the system's web view or by Chromium.
//!
//! Each [`Browser`] is a native child view of macaroni's window that the app moves over a pane
//! with [`Browser::place`]. Two engines sit behind it ([`Engine`]):
//!
//! - **System** (`system.rs`, Windows and macOS): the OS's own web view through `wry`, Edge's
//!   WebView2 on Windows and Safari's WebKit on macOS. Nothing to ship or start; it runs on the
//!   app's main thread.
//! - **Chromium** (`chromium.rs`, Windows only): CEF, with its own UI thread. Its helper
//!   processes (renderer, GPU, ...) are macaroni.exe itself, started with `--type=...`:
//!   [`helper_process`] must run first thing in `main`. Its runtime isn't shipped: the app
//!   downloads it ([`CHROMIUM_ARCHIVE`]) and [`chromium_available`] says whether it's there.
//!   Elsewhere it falls back to the system engine.
//!
//! This is the only crate that touches either engine. The app's `browser.rs` and
//! `update/browser.rs` own the panels and decide when to open, place and drop them.

#[cfg(windows)]
mod chromium;
#[cfg(any(windows, target_os = "macos"))]
mod system;

use std::path::Path;
use std::time::Duration;

/// What renders a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// The system's web view (WebView2, WebKit).
    System,
    /// Chromium via CEF; the system engine where it isn't available.
    Chromium,
}

/// The file in the Chromium download's folder that names the CEF archive it came from, so a
/// build linked against another CEF doesn't load it.
pub const CHROMIUM_VERSION_FILE: &str = "archive.txt";
#[cfg(windows)]
pub use chromium::{ARCHIVE as CHROMIUM_ARCHIVE, ARCHIVE_SHA1 as CHROMIUM_SHA1};

/// If this process is one of Chromium's helpers, runs it to completion and returns its exit
/// code; `None` in the app itself, which carries on. `chromium` is where the app downloads
/// Chromium's runtime.
pub fn helper_process(chromium: &Path) -> Option<i32> {
    #[cfg(windows)]
    return chromium::helper_process(chromium);
    #[cfg(not(windows))]
    {
        let _ = chromium;
        None
    }
}

/// Whether Chromium can run: its runtime is downloaded to `chromium` (for this build), or sits
/// next to the executable (development builds). Always false off Windows.
pub fn chromium_available(chromium: &Path) -> bool {
    #[cfg(windows)]
    return chromium::runtime_dir(chromium).is_some();
    #[cfg(not(windows))]
    {
        let _ = chromium;
        false
    }
}

/// Whether this process has loaded Chromium, so its downloaded files are in use until the app
/// quits.
pub fn chromium_loaded() -> bool {
    #[cfg(windows)]
    return chromium::loaded();
    #[cfg(not(windows))]
    false
}

/// Blocks until every dropped Chromium [`Browser`] has finished closing, or `timeout` passes.
/// Close the app's window only after this. System pages close as they're dropped.
pub fn wait_closed(timeout: Duration) {
    #[cfg(windows)]
    chromium::wait_closed(timeout);
    #[cfg(not(windows))]
    let _ = timeout;
}

/// Stops Chromium if it ran. Call on the main thread once the app's event loop has exited.
pub fn shutdown() {
    #[cfg(windows)]
    chromium::shutdown();
}

/// One browser panel. Dropping it closes the page.
pub struct Browser(Inner);

/// The engine-specific panel.
enum Inner {
    /// A system web view.
    #[cfg(any(windows, target_os = "macos"))]
    System(system::Browser),
    /// A CEF browser.
    #[cfg(windows)]
    Chromium(chromium::Browser),
}

/// Runs the same call on whichever engine's panel this is.
macro_rules! each {
    ($self:ident, $b:ident => $call:expr) => {
        match &$self.0 {
            #[cfg(any(windows, target_os = "macos"))]
            Inner::System($b) => $call,
            #[cfg(windows)]
            Inner::Chromium($b) => $call,
        }
    };
}

impl Browser {
    /// Opens `url` with `engine` in a new child view of `parent` (an `HWND` on Windows, an
    /// `NSView` on macOS), at `bounds` (or hidden), with cookies and cache under `data_dir` and
    /// Chromium's downloaded runtime in `chromium`.
    ///
    /// Every [`Event`] goes to `on_event`, and keys pressed in the page are offered to
    /// `shortcut` first. Call on the main thread. Fails, with a message for the tab, when the
    /// engine can't start: Chromium already running on the same profile in another process, or
    /// no web view on the system.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        engine: Engine,
        data_dir: &Path,
        chromium: &Path,
        parent: isize,
        url: &str,
        bounds: Option<Bounds>,
        on_event: impl Fn(Event) + Send + Sync + 'static,
        shortcut: impl Fn(KeyPress) -> bool + Send + Sync + 'static,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        if engine == Engine::Chromium {
            chromium::start(data_dir, chromium)?;
            let browser = chromium::Browser::open(parent, url, bounds, on_event, shortcut);
            return Ok(Self(Inner::Chromium(browser)));
        }
        let _ = (engine, chromium);
        #[cfg(any(windows, target_os = "macos"))]
        return system::Browser::open(data_dir, parent, url, bounds, on_event, shortcut)
            .map(|b| Self(Inner::System(b)));
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (data_dir, parent, url, bounds, on_event, shortcut);
            Err("browser tabs aren't supported on this platform".into())
        }
    }

    /// Moves the page over `bounds`, or hides it. Cheap to call repeatedly with the same value.
    pub fn place(&self, bounds: Option<Bounds>) {
        each!(self, b => b.place(bounds))
    }

    /// Loads `url`.
    pub fn navigate(&self, url: &str) {
        each!(self, b => b.navigate(url))
    }

    /// Goes back in history.
    pub fn back(&self) {
        each!(self, b => b.back())
    }

    /// Goes forward in history.
    pub fn forward(&self) {
        each!(self, b => b.forward())
    }

    /// Reloads the page.
    pub fn reload(&self) {
        each!(self, b => b.reload())
    }

    /// Stops loading.
    pub fn stop(&self) {
        each!(self, b => b.stop())
    }

    /// Gives the page keyboard focus.
    pub fn focus(&self) {
        each!(self, b => b.focus())
    }

    /// Gives keyboard focus back to the app's window, away from the page.
    pub fn blur(&self) {
        each!(self, b => b.blur())
    }

    /// Opens the developer tools for this page in their own window.
    pub fn devtools(&self) {
        each!(self, b => b.devtools())
    }

    /// The page's visible text, delivered to `done`, possibly on another thread. `done` gets an
    /// empty string when the page can't be read.
    pub fn read_text(&self, done: impl FnOnce(String) + Send + 'static) {
        each!(self, b => b.read_text(done))
    }
}

/// What a browser reports: on the main thread from the system engine, on CEF's UI thread from
/// Chromium.
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
    /// The page closed itself (`window.close()`), or Chromium finished closing it after the
    /// [`Browser`] was dropped.
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

/// A key pressed while a page has focus, offered to the app before (Windows) or as (macOS) the
/// page sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyPress {
    /// The Windows virtual-key code (`VK_*`; letters and digits are their ASCII uppercase),
    /// which is also what JavaScript's `keyCode` reports for the keys shortcuts use.
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
