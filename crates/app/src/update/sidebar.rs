//! The sidebar's size: dragged by its right edge, folded away with Ctrl+B (Cmd+B) or the title
//! bar's button, and saved with the session.
//!
//! The width in effect is written into `App.chrome.s.window.sidebar_width` ([`App::sync_sidebar`]),
//! so everything that already lays out around the sidebar (the title bar's logo column, drop
//! targets, divider dragging, browser placement through `view::workspace_origin`) follows a drag
//! without knowing about it. The config's `style.window.sidebar_width` is the width until the
//! user drags; changing that setting drops the dragged width.

use iced::Task;
use macaroni_core::Command;

use crate::App;
use crate::message::Message;
use crate::view::sidebar_compact_width;

/// Dragged narrower than this, the sidebar folds away.
const COLLAPSE_BELOW: f32 = 40.0;

/// The least room a drag leaves the panes.
const MIN_PANE_ROOM: f32 = 240.0;

/// Where the system draws the title bar there's nowhere to put the button that brings the
/// sidebar back, so a folded sidebar keeps a rail this wide for it.
pub(crate) const RAIL_WIDTH: f32 = 40.0;

impl App {
    /// Enter in the search field: opens the first workspace that matches, then empties the
    /// field and gives the keyboard back to the terminal.
    pub(crate) fn sidebar_search_submit(&mut self) -> Task<Message> {
        let first = self
            .model
            .workspaces()
            .iter()
            .find(|w| self.workspace_matches(w, &self.sidebar_query))
            .map(|w| w.id);
        let end = self.sidebar_search_end();
        match first {
            Some(ws) => Task::batch([self.apply(Command::SelectWorkspace(ws)), end]),
            None => end,
        }
    }

    /// Empties the search field and takes the keyboard from it.
    pub(crate) fn sidebar_search_end(&mut self) -> Task<Message> {
        self.sidebar_query.clear();
        self.sidebar_search_focused = false;
        iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
    }

    /// The sidebar's width now: the dragged width, else the configured one; folded, nothing
    /// (or the rail, where there's no custom title bar).
    pub(crate) fn sidebar_width(&self) -> f32 {
        if self.sidebar.collapsed {
            return if crate::view::CUSTOM_FRAME {
                0.0
            } else {
                RAIL_WIDTH
            };
        }
        self.sidebar
            .width
            .unwrap_or(self.config.style.window.sidebar_width)
    }

    /// Puts the width in effect into the chrome, where the view reads it.
    pub(crate) fn sync_sidebar(&mut self) {
        self.chrome.s.window.sidebar_width = self.sidebar_width();
    }

    /// The sidebar's edge was pressed: follow the pointer until the button is released.
    pub(crate) fn sidebar_drag_start(&mut self) -> Task<Message> {
        self.sidebar_drag = true;
        Task::none()
    }

    /// Moves the sidebar's edge to the window's `x`: no narrower than icons only (folding away
    /// below [`COLLAPSE_BELOW`]) and leaving the panes [`MIN_PANE_ROOM`].
    pub(crate) fn sidebar_drag_move(&mut self, x: f32) -> Task<Message> {
        let collapsed = x < COLLAPSE_BELOW;
        let max = (self.window_size.width - MIN_PANE_ROOM).max(COLLAPSE_BELOW);
        self.sidebar.collapsed = collapsed;
        if !collapsed {
            let min = sidebar_compact_width(self.chrome);
            self.sidebar.width = Some(x.clamp(min.min(max), max));
        }
        self.sync_sidebar();
        self.session_dirty = true;
        Task::none()
    }

    /// Folds the sidebar away, or brings it back at the width it had.
    pub(crate) fn toggle_sidebar(&mut self) -> Task<Message> {
        self.sidebar.collapsed = !self.sidebar.collapsed;
        self.sync_sidebar();
        self.session_dirty = true;
        Task::none()
    }
}
