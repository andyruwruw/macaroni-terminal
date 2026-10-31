//! Browser tabs: placing their windows over their panes, their events, and the address bar.
//!
//! A browser page is a native CEF child window laid over its pane, not something iced draws.
//! So after every update [`App::sync_browsers`] moves each window to its pane's last reported
//! bounds ([`Message::BrowserBounds`]) or hides it, starts the ones shown for the first time,
//! and moves keyboard focus between macaroni and the page as the focused tab changes. Events
//! from CEF arrive as [`Message::Browser`]. See `references/browser.md`.

use iced::{Rectangle, Task};
use macaroni_config::OpenLinks;
use macaroni_core::{BrowserId, Command, Direction, TabId, TabKind};

use crate::browser::{self, BrowserMessage};
use crate::message::{Message, Nav};
use crate::{App, open_url};

impl App {
    /// Moves every browser over its pane or hides it, starts browsers shown for the first time,
    /// and moves keyboard focus into a page when its tab becomes the focused one. Does nothing
    /// until the main window's native handle is known.
    ///
    /// Side effects: moves, shows, hides and focuses native windows, and may start CEF.
    pub(crate) fn sync_browsers(&mut self) {
        let Some(parent) = self.parent else {
            return;
        };
        self.place_browsers(parent);
        self.follow_focused_tab(parent);
    }

    /// Puts each browser over its pane, or hides it while its tab is off screen or covered.
    /// A browser whose start failed isn't retried.
    fn place_browsers(&mut self, parent: isize) {
        let ids: Vec<BrowserId> = self.browsers.keys().copied().collect();
        let menu = self.menu_bounds();
        let dragging = self
            .item_drag
            .is_some_and(|d| d.dragging && d.tab().is_some());
        for id in ids {
            let visible = self
                .model
                .tab_for_browser(id)
                .is_some_and(|tab| self.model.tab_visible(tab));
            let panel = &self.browsers[&id];
            // Nothing iced draws can cover a page, so pages under the menu or the settings
            // dialog step aside.
            // A dragged tab's drop markers would be hidden under pages too.
            let covered = |b: &Rectangle| {
                dragging
                    || self.settings.is_some()
                    || self.icon_picker.is_some()
                    || self.folder_dialog.is_some()
                    || menu.iter().any(|m| m.intersects(b))
            };
            let want = panel
                .bounds
                .filter(|b| visible && !covered(b))
                .map(|b| browser::physical(b, self.scale));
            if panel.browser.is_none() && panel.error.is_none() && want.is_some() {
                self.start_browser(id, parent, want);
            } else if let Some(b) = &panel.browser {
                b.place(want);
            }
        }
    }

    /// When the focused tab changes, gives the keyboard to its page, or back to macaroni. Only
    /// on a change: clicking into macaroni's UI while a browser tab stays focused is handled by
    /// [`App::reclaim_focus`], and a page not yet started takes focus in `start_browser`.
    fn follow_focused_tab(&mut self, parent: isize) {
        let focused = self.model.focused_tab();
        let (focused_id, focused_kind) = (focused.id, focused.kind.clone());
        if self.last_focused_tab == Some(focused_id) {
            return;
        }
        self.last_focused_tab = Some(focused_id);
        match focused_kind {
            TabKind::Browser(id) => {
                if let Some(b) = self.browsers.get(&id).and_then(|p| p.browser.as_ref()) {
                    b.focus();
                    self.page_focus = Some(id);
                }
            }
            TabKind::Terminal(_) => {
                if self.page_focus.take().is_some() {
                    macaroni_browser::focus_window(parent);
                }
            }
        }
    }

    /// Opens the browser window for a panel, at its tab's address (or the home page). Starts CEF
    /// on first use; if that fails, the panel keeps the error and shows it instead. Does nothing
    /// until the browser event channel exists.
    fn start_browser(
        &mut self,
        id: BrowserId,
        parent: isize,
        bounds: Option<macaroni_browser::Bounds>,
    ) {
        let Some(events) = self.browser_events.clone() else {
            return;
        };
        let url = self
            .model
            .tab_for_browser(id)
            .and_then(|t| self.model.tab(t)?.url.clone())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| self.config.browser.home.clone());
        // A VS Code tab (restored, say) waits for its server, which starts now if it isn't
        // running; `place_browsers` tries again on the next update. One that failed to start
        // loads anyway and shows the browser's error.
        let web = &mut self.vscode_web;
        if web.owns(&url) && !web.ready && web.error.is_none() {
            web.ensure_started();
            if web.server.is_some() {
                return;
            }
        }
        let Some(panel) = self.browsers.get_mut(&id) else {
            return;
        };
        if let Err(err) = browser::start() {
            panel.error = Some(err);
            return;
        }
        let keys = events.clone();
        panel.browser = Some(macaroni_browser::Browser::open(
            parent,
            &url,
            bounds,
            move |event| {
                let _ = events.unbounded_send((id, BrowserMessage::Event(event)));
            },
            browser::shortcuts(self.shared_keymap.clone(), move |action| {
                let _ = keys.unbounded_send((id, BrowserMessage::Action(action)));
            }),
        ));
        // A browser started by switching to its tab takes the keyboard, like an existing one.
        if self.model.focused_tab().kind == TabKind::Browser(id) {
            self.page_focus = Some(id);
        }
    }

    /// Handles what a browser reported: its address, title, loading state, a click into the
    /// page, a request for a new window, closing itself, a shortcut pressed in the page, or
    /// page text for the control API. Ignored once the browser's tab is gone.
    pub(crate) fn browser_message(
        &mut self,
        id: BrowserId,
        message: BrowserMessage,
    ) -> Task<Message> {
        use macaroni_browser::Event;
        let Some(tab) = self.model.tab_for_browser(id) else {
            return Task::none();
        };
        match message {
            BrowserMessage::Event(Event::Address(url)) => {
                if let Some(panel) = self.browsers.get_mut(&id) {
                    panel.address = url.clone();
                }
                self.apply(Command::SetBrowserUrl(id, url))
            }
            BrowserMessage::Event(Event::Title(title)) => self.page_title(id, tab, title),
            BrowserMessage::Event(Event::Loading {
                loading,
                back,
                forward,
            }) => {
                if let Some(panel) = self.browsers.get_mut(&id) {
                    panel.loading = loading;
                    panel.back = back;
                    panel.forward = forward;
                }
                Task::none()
            }
            BrowserMessage::Event(Event::Focused) => self.page_focused(id, tab),
            // Pages' popups and `target=_blank` links open as tabs beside this one.
            BrowserMessage::Event(Event::NewWindow(url)) => {
                self.apply(Command::OpenBrowserBeside { beside: tab, url })
            }
            // The page closed itself (window.close()).
            BrowserMessage::Event(Event::Closed) => self.apply(Command::BrowserClosed(id)),
            BrowserMessage::Action(action) => self.perform(action),
            BrowserMessage::Text(request, text) => {
                self.page_text(tab, request, text);
                Task::none()
            }
        }
    }

    /// Shows the page's new title on its tab and tells subscribers.
    fn page_title(&mut self, id: BrowserId, tab: TabId, title: String) -> Task<Message> {
        let task = self.apply(Command::SetBrowserTitle(id, Some(title)));
        let title = self
            .model
            .tab(tab)
            .and_then(|t| t.label().map(str::to_owned));
        self.broadcast(macaroni_ipc::Event::TitleChanged { tab: tab.0, title });
        task
    }

    /// A click in a page gave it the keyboard: its tab becomes the focused one.
    fn page_focused(&mut self, id: BrowserId, tab: TabId) -> Task<Message> {
        self.page_focus = Some(id);
        // The main window lost focus to the page, but the user is still in macaroni.
        self.window_focused = true;
        // Set first, so `follow_focused_tab` doesn't treat the `SelectTab` below as a change
        // and focus the page again.
        self.last_focused_tab = Some(tab);
        if self.model.focused_tab().id == tab {
            return Task::none();
        }
        self.apply(Command::SelectTab(tab))
    }

    /// Answers the control API caller that asked for this page's text (a request id already
    /// answered or unknown is ignored).
    fn page_text(&mut self, tab: TabId, request: u64, text: String) {
        if let Some((conn, id)) = self.text_requests.remove(&request) {
            let tab = self.model.tab(tab);
            let page = macaroni_ipc::Page {
                url: tab.and_then(|t| t.url.clone()),
                title: tab.and_then(|t| t.label().map(str::to_owned)),
                text,
            };
            self.respond(
                conn,
                id,
                serde_json::to_value(page).map_err(|e| e.to_string()),
            );
        }
    }

    /// Remembers where a browser's pane is, for placing its window.
    pub(crate) fn browser_bounds(&mut self, id: BrowserId, bounds: Rectangle) -> Task<Message> {
        if let Some(panel) = self.browsers.get_mut(&id) {
            panel.bounds = Some(bounds);
        }
        Task::none()
    }

    /// Typing in the address bar.
    pub(crate) fn address_input(&mut self, id: BrowserId, text: String) -> Task<Message> {
        if let Some(panel) = self.browsers.get_mut(&id) {
            panel.address = text;
        }
        Task::none()
    }

    /// Goes to what's in the address bar (an address, or search words), then hands the
    /// keyboard to the page. A blank bar does nothing.
    pub(crate) fn address_submit(&mut self, id: BrowserId) -> Task<Message> {
        if let Some(panel) = self.browsers.get_mut(&id)
            && !panel.address.trim().is_empty()
        {
            let url = browser::resolve(&panel.address);
            panel.address = url.clone();
            if let Some(b) = &panel.browser {
                b.navigate(&url);
                b.focus();
                self.page_focus = Some(id);
            }
        }
        Task::none()
    }

    /// The address bar's back, forward, reload, stop and devtools buttons.
    pub(crate) fn nav(&self, id: BrowserId, nav: Nav) -> Task<Message> {
        if let Some(b) = self.browsers.get(&id).and_then(|p| p.browser.as_ref()) {
            match nav {
                Nav::Back => b.back(),
                Nav::Forward => b.forward(),
                Nav::Reload => b.reload(),
                Nav::Stop => b.stop(),
                Nav::DevTools => b.devtools(),
            }
        }
        Task::none()
    }

    /// Opens a link from a terminal in a browser pane (see [`App::open_in_browser_pane`]), or in
    /// the system browser with `[browser] open_links = "system"` and on platforms without
    /// browser tabs (everything but Windows).
    pub(crate) fn open_link(&mut self, url: String) -> Task<Message> {
        if self.config.browser.open_links == OpenLinks::System || !cfg!(windows) {
            open_url(&url);
            return Task::none();
        }
        self.open_in_browser_pane(url)
    }

    /// Opens `url` in a new tab of the active workspace's browser pane (the first pane whose
    /// active tab is a browser), else in a new split to the right of the focused pane.
    pub(crate) fn open_in_browser_pane(&mut self, url: String) -> Task<Message> {
        let ws = self.model.active_workspace();
        let pane = ws.layout.leaves().into_iter().find(|p| {
            self.model
                .pane(*p)
                .and_then(|p| self.model.tab(p.active))
                .is_some_and(|t| matches!(t.kind, TabKind::Browser(_)))
        });
        let split = match pane {
            Some(pane) => {
                // Focusing a pane never quits, so there's no task to keep.
                let _ = self.apply(Command::FocusPane(pane));
                None
            }
            None => Some(Direction::Right),
        };
        self.apply(Command::OpenBrowser { url, split })
    }
}
