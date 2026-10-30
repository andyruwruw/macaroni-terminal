//! Stand-ins where browser panels aren't supported: CEF never starts.
//!
//! Same API as `windows.rs`, so the app compiles unchanged. [`Runtime::start`] always fails, which
//! the app shows in the tab, so no [`Browser`] is ever opened and its methods never run.

use std::path::Path;
use std::time::Duration;

use crate::{Bounds, Event, KeyPress};

/// Never a helper process: CEF doesn't run here.
pub fn helper_process() -> Option<i32> {
    None
}

/// Never constructed: [`Runtime::start`] always fails.
pub struct Runtime(());

impl Runtime {
    /// Fails with the message the tab shows.
    pub fn start(_data_dir: &Path) -> Result<Self, String> {
        Err("browser panels aren't supported on this platform".into())
    }
}

/// Returns at once: there are no browsers to wait for.
pub fn wait_closed(_timeout: Duration) {}

/// Does nothing: no page can hold the keyboard.
pub fn focus_window(_window: isize) {}

/// Never constructed: [`Runtime::start`] always fails.
pub struct Browser(());

impl Browser {
    /// Unreachable in practice; returns an inert panel.
    pub fn open(
        _parent: isize,
        _url: &str,
        _bounds: Option<Bounds>,
        _on_event: impl Fn(Event) + Send + Sync + 'static,
        _shortcut: impl Fn(KeyPress) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self(())
    }

    /// Does nothing.
    pub fn place(&self, _bounds: Option<Bounds>) {}
    /// Does nothing.
    pub fn navigate(&self, _url: &str) {}
    /// Does nothing.
    pub fn back(&self) {}
    /// Does nothing.
    pub fn forward(&self) {}
    /// Does nothing.
    pub fn reload(&self) {}
    /// Does nothing.
    pub fn stop(&self) {}
    /// Does nothing.
    pub fn focus(&self) {}
    /// Does nothing.
    pub fn devtools(&self) {}

    /// Calls `done` at once with an empty string.
    pub fn read_text(&self, done: impl FnOnce(String) + Send + 'static) {
        done(String::new());
    }
}
