//! The window itself: the custom title bar's controls, size, scale, focus and closing.
//!
//! Also where keyboard focus moves between macaroni and a browser page's native window, which
//! iced knows nothing about (see `update/browser.rs`).

use iced::{Size, Task, window};

use crate::message::{Message, WindowOp};
#[cfg(windows)]
use crate::move_dpi;
use crate::{App, native_handle};

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
        if self.page_focus.take().is_some()
            && let Some(parent) = self.parent
        {
            macaroni_browser::focus_window(parent);
        }
        Task::none()
    }

    /// Saves the session now, then quits (after the browsers close; see [`App::exit`]).
    pub(crate) fn close_requested(&mut self) -> Task<Message> {
        self.save_session(true);
        self.exit()
    }
}
