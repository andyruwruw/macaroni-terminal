//! The window itself: the custom title bar's controls, size, scale, focus and closing.
//!
//! Also where keyboard focus moves between macaroni and a browser page's native window, which
//! iced knows nothing about (see `update/browser.rs`).

use std::collections::BTreeMap;

use iced::{Size, Task, window};

use crate::message::{Message, WindowOp};
#[cfg(windows)]
use crate::move_dpi;
use crate::{App, native_handle};

/// The most of a terminal's text kept across a restart, from its end.
const SCROLLBACK_KEPT_BYTES: usize = 1 << 20;

impl App {
    /// Remembers the main window and asks for its native handle and scale factor. On Windows,
    /// also hooks the window to hold DPI changes back during a move (`move_dpi`); that has to
    /// happen in `window::run`, on the event loop's thread, which owns the window.
    pub(crate) fn window_opened(&mut self, id: window::Id) -> Task<Message> {
        self.window = Some(id);
        Task::batch([
            window::run(id, |w| {
                let handle = native_handle(w);
                #[cfg(windows)]
                if let Some(handle) = handle {
                    move_dpi::install(handle);
                }
                handle
            })
            .map(Message::WindowHandle),
            window::scale_factor(id).map(Message::Rescaled),
        ])
    }

    /// Runs a title bar control, or a resize from the window's edge.
    pub(crate) fn window_op(&self, op: WindowOp) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        match op {
            WindowOp::Drag => window::drag(id),
            WindowOp::ToggleMaximize => window::toggle_maximize(id),
            WindowOp::Minimize => window::minimize(id, true),
            WindowOp::Resize(direction) => window::drag_resize(id, direction),
        }
    }

    /// Remembers the new size, then asks whether the window is now maximized.
    pub(crate) fn window_resized(&mut self, size: Size) -> Task<Message> {
        self.window_size = size;
        match self.window {
            Some(id) => window::is_maximized(id).map(Message::Maximized),
            None => Task::none(),
        }
    }

    /// Notes the window gaining or losing focus.
    pub(crate) fn set_window_focused(&mut self, focused: bool) -> Task<Message> {
        // Focus moving into a page looks like the window losing it, since the page is another
        // native window; the user is still looking at macaroni, so notifications stay quiet.
        self.window_focused = focused || self.page_focus.is_some();
        Task::none()
    }

    /// A click in macaroni's own UI takes keyboard focus back from the page that had it, by
    /// focusing the main window natively.
    pub(crate) fn reclaim_focus(&mut self) -> Task<Message> {
        self.blur_page();
        Task::none()
    }

    /// Saves the session now, then quits (after the browsers close; see [`App::exit`]).
    pub(crate) fn close_requested(&mut self) -> Task<Message> {
        self.write_scrollback();
        self.save_session(true);
        self.exit()
    }

    /// With `[session] keep_scrollback`, writes each running terminal's text to its own file
    /// for the final session save to name, then deletes the files no tab refers to any more
    /// (tabs not yet started keep the file they were restored with). Files are named by time
    /// and tab id, since tab ids start over in each run. Failures just leave a tab without its
    /// text.
    fn write_scrollback(&mut self) {
        let dir = crate::session::scrollback_dir(&self.config_dir);
        if !self.config.session.restore || !self.config.session.keep_scrollback || self.ran_program
        {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let mut files = BTreeMap::new();
        for (&session, running) in &self.running {
            let Some(tab) = self.model.tab_for_session(session) else {
                continue;
            };
            let mut text = running.session.all_text();
            // Long histories keep their end: the most recent output is what matters.
            if text.len() > SCROLLBACK_KEPT_BYTES {
                let mut cut = text.len() - SCROLLBACK_KEPT_BYTES;
                while !text.is_char_boundary(cut) {
                    cut += 1;
                }
                text = text.split_off(cut);
            }
            if text.trim().is_empty() {
                continue;
            }
            let name = format!("{stamp}-{}.txt", tab.0);
            if std::fs::write(dir.join(&name), text).is_ok() {
                files.insert(tab, name);
            }
        }
        let keep: std::collections::BTreeSet<String> = files
            .values()
            .cloned()
            .chain(self.restored.values().filter_map(|r| r.scrollback.clone()))
            .collect();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if !keep.contains(&entry.file_name().to_string_lossy().into_owned()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        self.scrollback_files = files;
        self.session_dirty = true;
    }
}
