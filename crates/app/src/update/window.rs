//! The windows: opening and closing them, the custom title bar's controls, size, scale, focus.
//!
//! Each OS window shows one of the model's windows (`App.wins` maps them). The model decides
//! which is focused; OS focus events tell it ([`Command::FocusWindow`]), and its
//! [`Effect::OpenWindow`](macaroni_core::Effect), `CloseWindow` and `FocusWindow` open, close
//! and raise OS windows (`update/effects.rs`). Outside the view, "the window" means the focused
//! one; while `App::view` draws a window, `App.viewing` makes it that one.
//!
//! Also where keyboard focus moves between macaroni and a browser page's native window, which
//! iced knows nothing about (see `update/browser.rs`).

use std::collections::BTreeMap;

use iced::{Size, Task, window};
use macaroni_core::{Command, WindowId, WindowView, Workspace};

use crate::message::{Message, WindowOp};
#[cfg(windows)]
use crate::move_loop;
use crate::session::SavedWindow;
use crate::{App, DEFAULT_WINDOW_SIZE, Win, native_handle, window_settings};

/// Where the cursor ends up inside a window opened by dropping a tab or workspace outside
/// every window: over its title bar, near the left.
const DROP_GRAB: iced::Point = iced::Point::new(120.0, 16.0);

/// The most of a terminal's text kept across a restart, from its end.
const SCROLLBACK_KEPT_BYTES: usize = 1 << 20;

impl App {
    /// Opens an OS window for the model's window `w`, where `saved` says it was, or where a
    /// drag dropped something (`App.next_window_at`, the cursor a little inside its corner).
    /// It's in `App.wins` from now on, before it has actually opened.
    pub(crate) fn open_window(
        &mut self,
        w: WindowId,
        saved: Option<&SavedWindow>,
    ) -> Task<Message> {
        let ui = self.ui_scale();
        let mut settings = window_settings(saved, ui);
        if saved.is_none()
            && let Some(at) = self.next_window_at.take()
        {
            // A window's position is in the system's logical pixels, without `ui_scale`.
            settings.position = window::Position::Specific(iced::Point::new(
                (at.x - DROP_GRAB.x).max(0.0) * ui,
                (at.y - DROP_GRAB.y).max(0.0) * ui,
            ));
        }
        let (id, opened) = window::open(settings.clone());
        self.wins.insert(
            id,
            Win {
                model: w,
                size: settings.size,
                position: saved
                    .and_then(|s| s.position)
                    .map(|(x, y)| iced::Point::new(x / ui, y / ui)),
                maximized: false,
                parent: None,
                scale: 1.0,
                focused: false,
            },
        );
        let maximize = if saved.is_some_and(|s| s.maximized) {
            window::maximize(id, true)
        } else {
            Task::none()
        };
        Task::batch([opened.discard(), maximize])
    }

    /// Closes the OS window showing the model's window `w`. Its browsers were already closed.
    pub(crate) fn close_window(&mut self, w: WindowId) -> Task<Message> {
        let Some(id) = self.os_window(w) else {
            return Task::none();
        };
        self.wins.remove(&id);
        self.reader.forget(id);
        window::run(id, move |_| crate::accessibility::adapter::detach(id))
            .discard()
            .chain(window::close(id))
    }

    /// Brings the OS window showing the model's window `w` to the front.
    pub(crate) fn raise_window(&self, w: WindowId) -> Task<Message> {
        match self.os_window(w) {
            Some(id) => window::gain_focus(id),
            None => Task::none(),
        }
    }

    /// The OS window showing the model's window `w`.
    pub(crate) fn os_window(&self, w: WindowId) -> Option<window::Id> {
        self.wins
            .iter()
            .find(|(_, win)| win.model == w)
            .map(|(id, _)| *id)
    }

    /// The OS window this is about: the one being drawn inside `App::view`, else the focused
    /// one.
    pub(crate) fn win(&self) -> Option<&Win> {
        match self.viewing.get() {
            Some(id) => self.wins.get(&id),
            None => self
                .os_window(self.model.focused_window())
                .and_then(|id| self.wins.get(&id)),
        }
    }

    /// That window's size in logical pixels.
    pub(crate) fn window_size(&self) -> Size {
        self.win().map_or(DEFAULT_WINDOW_SIZE, |w| w.size)
    }

    /// Whether that window is maximized.
    pub(crate) fn maximized(&self) -> bool {
        self.win().is_some_and(|w| w.maximized)
    }

    /// That window's scale factor: the display's times `[accessibility] ui_scale`, how many
    /// physical pixels a logical one covers.
    pub(crate) fn scale(&self) -> f32 {
        self.win().map_or(1.0, |w| w.scale) * self.ui_scale()
    }

    /// `[accessibility] ui_scale`: how much bigger than the display's own scaling macaroni
    /// draws. iced's logical pixels include it; the system's (window positions, saved sizes)
    /// don't.
    pub(crate) fn ui_scale(&self) -> f32 {
        self.config.accessibility.ui_scale
    }

    /// That window's sidebar.
    ///
    /// # Panics
    ///
    /// Never: the focused window always exists, and a window being drawn is only drawn while
    /// it's in `App.wins` (closing removes it first).
    pub(crate) fn shown(&self) -> WindowView<'_> {
        let w = self.win().map_or(self.model.focused_window(), |w| w.model);
        self.model
            .window(w)
            .or_else(|| self.model.window(self.model.focused_window()))
            .expect("the focused window exists")
    }

    /// That window's active workspace.
    ///
    /// # Panics
    ///
    /// Never while the model's invariants hold.
    pub(crate) fn shown_workspace(&self) -> &Workspace {
        self.model
            .workspace(self.shown().active)
            .expect("a window's active workspace exists")
    }

    /// Whether OS window `id` shows the model's focused window.
    pub(crate) fn is_focused_window(&self, id: window::Id) -> bool {
        self.wins
            .get(&id)
            .is_some_and(|w| w.model == self.model.focused_window())
    }

    /// Whether the window being drawn is the focused one, where menus and dialogs show.
    pub(crate) fn drawing_focused(&self) -> bool {
        self.win()
            .is_none_or(|w| w.model == self.model.focused_window())
    }

    /// Asks a newly opened window for its native handle and scale factor. On Windows, also
    /// hooks it to start drags safely and hold DPI changes back during a move (`move_loop`);
    /// that has to happen in `window::run`, on the event loop's thread, which owns the window.
    pub(crate) fn window_opened(&mut self, id: window::Id) -> Task<Message> {
        let ui = self.ui_scale();
        Task::batch([
            window::run(id, move |w| {
                let handle = native_handle(w);
                #[cfg(windows)]
                if let Some(handle) = handle {
                    move_loop::install(handle);
                }
                // As soon as it's open, so screen readers find it from the start.
                if let Some(handle) = handle {
                    crate::accessibility::adapter::attach(id, handle);
                }
                handle
            })
            .map(move |handle| Message::WindowHandle(id, handle)),
            window::scale_factor(id).map(move |scale| Message::Rescaled(id, scale)),
            // Where it is, for dragging between windows; `Moved` keeps it current. This answers
            // in the system's logical pixels, `Moved` in iced's.
            window::position(id).then(move |at| match at {
                Some(at) => Task::done(Message::WindowMoved(
                    id,
                    iced::Point::new(at.x / ui, at.y / ui),
                )),
                None => Task::none(),
            }),
        ])
    }

    /// Runs a title bar control, or a resize from the window's edge, on the focused window
    /// (pressing it focused it).
    pub(crate) fn window_op(&mut self, op: WindowOp) -> Task<Message> {
        let Some(id) = self.os_window(self.model.focused_window()) else {
            return Task::none();
        };
        match op {
            WindowOp::Close => self.close_requested(id),
            WindowOp::Drag => drag(id, None),
            WindowOp::ToggleMaximize => window::toggle_maximize(id),
            WindowOp::Minimize => window::minimize(id, true),
            WindowOp::Resize(direction) => drag(id, Some(direction)),
        }
    }

    /// Remembers a window's new size, then asks whether it's now maximized.
    pub(crate) fn window_resized(&mut self, id: window::Id, size: Size) -> Task<Message> {
        let Some(win) = self.wins.get_mut(&id) else {
            return Task::none();
        };
        win.size = size;
        self.session_dirty = true;
        window::is_maximized(id).map(move |maximized| Message::Maximized(id, maximized))
    }

    /// Notes a window gaining or losing focus. A window gaining it becomes the model's
    /// focused window, after closing any menu or dialog open in the one before.
    pub(crate) fn set_window_focused(&mut self, id: window::Id, focused: bool) -> Task<Message> {
        let Some(win) = self.wins.get_mut(&id) else {
            return Task::none();
        };
        win.focused = focused;
        let w = win.model;
        // Focus moving into a page looks like the window losing it, since the page is another
        // native window; the user is still looking at macaroni, so notifications stay quiet.
        self.window_focused = self.wins.values().any(|w| w.focused) || self.page_focus.is_some();
        // macOS's adapter needs telling; Windows' follows the window's own messages.
        let reader = if cfg!(target_os = "macos") && self.reader.active() {
            window::run(id, move |_| {
                crate::accessibility::adapter::focus(id, focused);
            })
            .discard()
        } else {
            Task::none()
        };
        if !focused || w == self.model.focused_window() {
            return reader;
        }
        let dismissed = self.dismiss();
        let applied = self.apply(Command::FocusWindow(w));
        Task::batch([reader, dismissed, applied])
    }

    /// A click in macaroni's own UI takes keyboard focus back from the page that had it, by
    /// focusing the main window natively.
    pub(crate) fn reclaim_focus(&mut self) -> Task<Message> {
        self.blur_page();
        Task::none()
    }

    /// A window's close button: with other windows open, it closes with everything in it.
    /// The last window saves the session now, then quits (after the browsers close; see
    /// [`App::exit`]).
    pub(crate) fn close_requested(&mut self, id: window::Id) -> Task<Message> {
        if self.wins.len() > 1
            && let Some(win) = self.wins.get(&id)
        {
            return self.apply(Command::CloseWindow(win.model));
        }
        self.write_scrollback();
        self.save_session(true);
        self.exit()
    }

    /// The saved state of every window, in the model's window order: its active workspace
    /// (filled in by `session::capture`), position, size and whether it's maximized.
    pub(crate) fn saved_windows(&self) -> Vec<SavedWindow> {
        self.model
            .windows()
            .into_iter()
            .map(|w| {
                let win = self.os_window(w).and_then(|id| self.wins.get(&id));
                // Saved in the system's logical pixels, so a new `ui_scale` keeps the window
                // where and how big it was.
                let ui = self.ui_scale();
                SavedWindow {
                    active: 0,
                    position: win.and_then(|w| w.position).map(|p| (p.x * ui, p.y * ui)),
                    size: win.map(|w| (w.size.width * ui, w.size.height * ui)),
                    maximized: win.is_some_and(|w| w.maximized),
                }
            })
            .collect()
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

/// Moves window `id` (with `edge` `None`) or resizes it from `edge`, for a press on the title
/// bar or an edge. On Windows `move_loop` starts it, only if the button is still down.
fn drag(id: window::Id, edge: Option<window::Direction>) -> Task<Message> {
    #[cfg(windows)]
    return window::run(id, move |w| {
        if let Some(handle) = native_handle(w) {
            move_loop::drag(handle, edge);
        }
    })
    .discard();
    #[cfg(not(windows))]
    match edge {
        None => window::drag(id),
        Some(direction) => window::drag_resize(id, direction),
    }
}
