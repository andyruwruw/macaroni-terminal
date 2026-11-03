//! The find bar: in a terminal's scrollback, or in a browser tab's page.
//!
//! The bar (`App.search`) is bound to one terminal or page when it opens. A terminal search
//! runs in the emulator (`Session::search`), which selects the match and scrolls to it; each
//! step takes a new snapshot so the view shows it. A page search runs in the page
//! (`Browser::find`) and answers later, as `BrowserMessage::Found`, whether anything matched.

use iced::Task;
use macaroni_core::{BrowserId, TabKind};
use macaroni_term::SearchDirection;

use crate::browser::BrowserMessage;
use crate::message::Message;
use crate::state::{Search, SearchTarget};
use crate::{App, view};

impl App {
    /// Opens the find bar over the focused terminal or page and focuses its field, taking the
    /// keyboard back from the page first.
    pub(crate) fn open_search(&mut self) -> Task<Message> {
        let target = match self.model.focused_tab().kind {
            TabKind::Terminal(session) => SearchTarget::Terminal(session),
            TabKind::Browser(id) => {
                self.blur_page();
                SearchTarget::Page(id)
            }
        };
        // A bar open on another terminal or page closes first, clearing its match there.
        let closed = match &self.search {
            Some(open) if open.target != target => self.search_close(),
            _ => Task::none(),
        };
        let query = match &self.search {
            Some(open) => open.query.clone(),
            None => String::new(),
        };
        self.search = Some(Search {
            target,
            query,
            found: true,
        });
        Task::batch([closed, iced::widget::operation::focus(view::SEARCH_INPUT)])
    }

    /// Opens the find bar on the focused terminal with `query`, on the match `back` steps up
    /// from the newest: where a scrollback search result points.
    pub(crate) fn find_in_tab(&mut self, query: &str, back: usize) -> Task<Message> {
        let focus = self.open_search();
        let found = self.search_input(query.to_owned());
        let steps: Vec<_> = (0..back)
            .map(|_| self.search_step(SearchDirection::Older))
            .collect();
        Task::batch([focus, found].into_iter().chain(steps))
    }

    /// Searches again as the query changes: from the newest line in a terminal, from the top
    /// of a page.
    pub(crate) fn search_input(&mut self, query: String) -> Task<Message> {
        let Some(search) = &mut self.search else {
            return Task::none();
        };
        search.query = query.clone();
        match search.target {
            SearchTarget::Terminal(session) => {
                if let Some(r) = self.running.get_mut(&session) {
                    // From the bottom again, so the nearest match to the newest output comes
                    // first.
                    r.session.reset_search();
                    search.found =
                        r.session.search(&query, SearchDirection::Older) || query.is_empty();
                    r.snapshot = r.session.snapshot();
                }
            }
            SearchTarget::Page(id) => {
                search.found = true;
                self.find_in_page(id, &query, true, false);
            }
        }
        Task::none()
    }

    /// Moves to the next match in `direction`: `Older` is up and `Newer` down, in a page too.
    pub(crate) fn search_step(&mut self, direction: SearchDirection) -> Task<Message> {
        let Some(search) = &mut self.search else {
            return Task::none();
        };
        match search.target {
            SearchTarget::Terminal(session) => {
                if let Some(r) = self.running.get_mut(&session) {
                    search.found =
                        r.session.search(&search.query, direction) || search.query.is_empty();
                    r.snapshot = r.session.snapshot();
                }
            }
            SearchTarget::Page(id) => {
                let query = search.query.clone();
                self.find_in_page(id, &query, direction == SearchDirection::Newer, true);
            }
        }
        Task::none()
    }

    /// Asks page `id` to find `query` (see `Browser::find`); the answer comes back as
    /// [`BrowserMessage::Found`]. An empty query just clears the last match.
    fn find_in_page(&self, id: BrowserId, query: &str, forward: bool, again: bool) {
        let (Some(events), Some(b)) = (
            self.browser_events.clone(),
            self.browsers.get(&id).and_then(|p| p.browser.as_ref()),
        ) else {
            return;
        };
        if query.is_empty() {
            b.stop_find();
            return;
        }
        let asked = query.to_owned();
        b.find(query, forward, again, move |found| {
            let _ = events.unbounded_send((id, BrowserMessage::Found(asked, found)));
        });
    }

    /// A page answered whether `query` matched. Ignored once the bar has moved on to another
    /// query or target, since answers can arrive late.
    pub(crate) fn page_found(&mut self, id: BrowserId, query: &str, found: bool) {
        if let Some(search) = &mut self.search
            && search.target == SearchTarget::Page(id)
            && search.query == query
        {
            search.found = found;
        }
    }

    /// Closes the find bar, clearing the match it left selected.
    pub(crate) fn search_close(&mut self) -> Task<Message> {
        match self.search.take().map(|s| s.target) {
            Some(SearchTarget::Terminal(session)) => {
                if let Some(r) = self.running.get_mut(&session) {
                    r.session.reset_search();
                    r.session.clear_selection();
                    r.snapshot = r.session.snapshot();
                }
            }
            Some(SearchTarget::Page(id)) => {
                if let Some(b) = self.browsers.get(&id).and_then(|p| p.browser.as_ref()) {
                    b.stop_find();
                }
            }
            None => {}
        }
        Task::none()
    }
}
