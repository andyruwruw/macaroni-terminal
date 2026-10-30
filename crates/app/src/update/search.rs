//! Find in a terminal's scrollback.
//!
//! The bar (`App.search`) is bound to one session when it opens. The search itself runs in the
//! emulator (`Session::search`), which selects the match and scrolls to it; each step takes a
//! new snapshot so the view shows it.

use iced::Task;
use macaroni_term::SearchDirection;

use crate::message::Message;
use crate::state::Search;
use crate::{App, view};

impl App {
    /// Opens the search bar over the focused terminal and focuses its field. Does nothing when
    /// a browser tab is focused.
    pub(super) fn open_search(&mut self) -> Task<Message> {
        let Some(session) = self.focused_session() else {
            return Task::none();
        };
        self.search = Some(Search {
            session,
            query: String::new(),
            found: true,
        });
        iced::widget::operation::focus(view::SEARCH_INPUT)
    }

    /// Searches again from the newest line as the query changes.
    pub(crate) fn search_input(&mut self, query: String) -> Task<Message> {
        if let Some(search) = &mut self.search
            && let Some(r) = self.running.get_mut(&search.session)
        {
            // From the bottom again, so the nearest match to the newest output comes first.
            r.session.reset_search();
            search.found = r.session.search(&query, SearchDirection::Older) || query.is_empty();
            search.query = query;
            r.snapshot = r.session.snapshot();
        }
        Task::none()
    }

    /// Moves to the next match in `direction`.
    pub(crate) fn search_step(&mut self, direction: SearchDirection) -> Task<Message> {
        if let Some(search) = &mut self.search
            && let Some(r) = self.running.get_mut(&search.session)
        {
            search.found = r.session.search(&search.query, direction) || search.query.is_empty();
            r.snapshot = r.session.snapshot();
        }
        Task::none()
    }

    /// Closes the search bar, clearing the match it left selected.
    pub(crate) fn search_close(&mut self) -> Task<Message> {
        if let Some(search) = self.search.take()
            && let Some(r) = self.running.get_mut(&search.session)
        {
            r.session.reset_search();
            r.session.clear_selection();
            r.snapshot = r.session.snapshot();
        }
        Task::none()
    }
}
