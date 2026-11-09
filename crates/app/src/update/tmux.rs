//! tmux control mode in the app: each tmux window a tab beside the terminal that runs
//! `tmux -CC` (the gateway), each of its panes a session of the app's own.
//!
//! The term crate's client ([`TmuxClient`]) reports [`TmuxEvent`]s on the gateway session;
//! [`App::tmux_event`] keeps `App.tmux` in step: a [`Tmux`] per gateway with its windows (each
//! a `TabKind::Tmux` tab, opened with `Command::OpenTmux`) and its panes. Pane sessions aren't in
//! the model, so their ids come from their own range ([`PANE_IDS`]) and they live in
//! `App.running` like any terminal, which gives them input, selection, scrollback, find and
//! copy for free. tmux decides the panes' sizes from the client's: the tab body reports its
//! bounds (`Message::TmuxBounds`), which become `refresh-client -C`.
//!
//! What the user does goes back to tmux: clicking a pane selects it, splitting in a tmux tab
//! splits the pane, a new tab there is a new window, and closing the tab kills the window
//! (`App::tmux_prune`, after any command that closes tabs). Esc in the gateway detaches.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use iced::{Rectangle, Task};
use macaroni_core::{Command, Direction, SessionId, TabId, TabKind};
use macaroni_term::{
    GridSize, Layout, Session, SessionConfig, Source, TmuxClient, TmuxEvent, TmuxPane, TmuxWindow,
};

use super::config::cursor_shape;
use crate::message::Message;
use crate::{App, Running};

/// Where tmux pane sessions' ids start: far above anything the model's counter reaches, so the
/// two never meet.
pub(crate) const PANE_IDS: u64 = 1 << 62;

/// One gateway's control mode session.
pub(crate) struct Tmux {
    /// The client, for commands.
    pub client: Arc<TmuxClient>,
    /// Its windows, by tmux number.
    pub windows: BTreeMap<u32, TmuxTab>,
    /// Its panes' sessions, by tmux number.
    pub panes: HashMap<u32, SessionId>,
    /// The first window list hasn't come yet: when it does, its current window is shown.
    pub fresh: bool,
}

/// A tmux window's tab.
pub(crate) struct TmuxTab {
    /// The tab showing it.
    pub tab: TabId,
    /// Its panes, in cells.
    pub layout: Layout,
    /// Its focused pane.
    pub active_pane: Option<u32>,
}

impl App {
    /// Acts on what a gateway's tmux client reported.
    pub(crate) fn tmux_event(&mut self, gateway: SessionId, event: TmuxEvent) -> Task<Message> {
        match event {
            TmuxEvent::Attached => {
                if let Some(client) = self.running.get(&gateway).and_then(|r| r.session.tmux()) {
                    self.tmux.insert(
                        gateway,
                        Tmux {
                            client,
                            windows: BTreeMap::new(),
                            panes: HashMap::new(),
                            fresh: true,
                        },
                    );
                }
                Task::none()
            }
            TmuxEvent::Windows(windows) => self.tmux_windows(gateway, windows),
            TmuxEvent::Layout { window, layout } => {
                if let Some(t) = self.tmux.get_mut(&gateway)
                    && let Some(w) = t.windows.get_mut(&window)
                {
                    w.layout = layout;
                }
                self.tmux_sync_panes(gateway);
                Task::none()
            }
            TmuxEvent::WindowClosed(window) => {
                let tab = self
                    .tmux
                    .get_mut(&gateway)
                    .and_then(|t| t.windows.remove(&window))
                    .map(|w| w.tab);
                self.tmux_sync_panes(gateway);
                match tab {
                    Some(tab) => self.apply(Command::CloseTab(tab)),
                    None => Task::none(),
                }
            }
            TmuxEvent::Renamed { window, name } => {
                let tab = self
                    .tmux
                    .get(&gateway)
                    .and_then(|t| t.windows.get(&window))
                    .map(|w| w.tab);
                match tab {
                    Some(tab) => self.apply(Command::SetTmuxTitle(tab, Some(name))),
                    None => Task::none(),
                }
            }
            TmuxEvent::ActivePane { window, pane } => {
                if let Some(t) = self.tmux.get_mut(&gateway)
                    && let Some(w) = t.windows.get_mut(&window)
                {
                    w.active_pane = Some(pane);
                }
                Task::none()
            }
            TmuxEvent::Detached(_) => self.tmux_teardown(gateway),
        }
    }

    /// Matches the tabs to tmux's windows: opens tabs for new ones (after the gateway's tabs
    /// for this session), renames, closes tabs of windows that are gone. On attach, shows the
    /// session's current window.
    fn tmux_windows(&mut self, gateway: SessionId, windows: Vec<TmuxWindow>) -> Task<Message> {
        let Some(gateway_tab) = self.model.tab_for_session(gateway) else {
            return Task::none();
        };
        let Some(t) = self.tmux.get_mut(&gateway) else {
            return Task::none();
        };
        let fresh = std::mem::replace(&mut t.fresh, false);
        let listed: Vec<u32> = windows.iter().map(|w| w.id).collect();
        let gone: Vec<TabId> = t
            .windows
            .iter()
            .filter(|(id, _)| !listed.contains(id))
            .map(|(_, w)| w.tab)
            .collect();
        t.windows.retain(|id, _| listed.contains(id));
        let mut tasks = Vec::new();
        let mut current = None;
        for window in windows {
            let known = self
                .tmux
                .get(&gateway)
                .and_then(|t| t.windows.get(&window.id))
                .map(|w| w.tab);
            let tab = match known {
                Some(tab) => tab,
                None => {
                    // After the last of this session's tabs, else after the gateway.
                    let beside = self
                        .tmux
                        .get(&gateway)
                        .and_then(|t| t.windows.values().map(|w| w.tab).next_back())
                        .filter(|tab| self.model.tab(*tab).is_some())
                        .unwrap_or(gateway_tab);
                    tasks.push(self.apply(Command::OpenTmux { beside }));
                    let Some(tab) = self.tab_after(beside) else {
                        continue;
                    };
                    tab
                }
            };
            tasks.push(self.apply(Command::SetTmuxTitle(tab, Some(window.name.clone()))));
            if window.active {
                current = Some(tab);
            }
            if let Some(t) = self.tmux.get_mut(&gateway) {
                t.windows.insert(
                    window.id,
                    TmuxTab {
                        tab,
                        layout: window.layout,
                        active_pane: window.active_pane,
                    },
                );
            }
        }
        for tab in gone {
            tasks.push(self.apply(Command::CloseTab(tab)));
        }
        self.tmux_sync_panes(gateway);
        if fresh && let Some(tab) = current {
            tasks.push(self.apply(Command::SelectTab(tab)));
        }
        Task::batch(tasks)
    }

    /// The tab right after `tab` in its pane.
    fn tab_after(&self, tab: TabId) -> Option<TabId> {
        let pane = self.model.pane(self.model.pane_of_tab(tab)?)?;
        let at = pane.tabs.iter().position(|t| *t == tab)?;
        pane.tabs.get(at + 1).copied()
    }

    /// Opens a session for each pane tmux's layouts list and drops those of panes gone.
    fn tmux_sync_panes(&mut self, gateway: SessionId) {
        let Some(t) = self.tmux.get(&gateway) else {
            return;
        };
        let wanted: Vec<(u32, u16, u16)> = t
            .windows
            .values()
            .flat_map(|w| w.layout.panes())
            .map(|(pane, _, _, width, height)| (pane, width, height))
            .collect();
        let gone: Vec<(u32, SessionId)> = t
            .panes
            .iter()
            .filter(|(pane, _)| !wanted.iter().any(|w| w.0 == **pane))
            .map(|(pane, id)| (*pane, *id))
            .collect();
        let client = t.client.clone();
        for (pane, id) in gone {
            self.running.remove(&id);
            if let Some(t) = self.tmux.get_mut(&gateway) {
                t.panes.remove(&pane);
            }
        }
        for (pane, cols, rows) in wanted {
            let known = self
                .tmux
                .get(&gateway)
                .and_then(|t| t.panes.get(&pane))
                .copied();
            if let Some(id) = known {
                let size = self.pane_grid(cols, rows);
                if let Some(r) = self.running.get_mut(&id)
                    && (r.size.cols, r.size.rows) != (size.cols, size.rows)
                {
                    r.session.resize(size);
                    r.size = size;
                    r.snapshot = r.session.snapshot();
                }
                continue;
            }
            if let Some(id) = self.spawn_tmux_pane(&client, pane, cols, rows)
                && let Some(t) = self.tmux.get_mut(&gateway)
            {
                t.panes.insert(pane, id);
            }
        }
    }

    /// A grid of `cols` by `rows` in the terminal font.
    fn pane_grid(&self, cols: u16, rows: u16) -> GridSize {
        GridSize {
            cols: cols.max(1),
            rows: rows.max(1),
            cell_width: self.metrics.cell_width.round() as u16,
            cell_height: self.metrics.cell_height.round() as u16,
        }
    }

    /// Starts a tmux pane's session under a new id from [`PANE_IDS`]; `None` if it couldn't.
    fn spawn_tmux_pane(
        &mut self,
        client: &Arc<TmuxClient>,
        pane: u32,
        cols: u16,
        rows: u16,
    ) -> Option<SessionId> {
        let events = self.events.clone()?;
        self.tmux_ids += 1;
        let id = SessionId(PANE_IDS + self.tmux_ids);
        let size = self.pane_grid(cols, rows);
        let config = SessionConfig {
            source: Source::Tmux(TmuxPane {
                client: client.clone(),
                pane,
            }),
            program: None,
            args: Vec::new(),
            cwd: None,
            size,
            scrollback: self.config.terminal.scrollback,
            env: Vec::new(),
            colors: Some(self.palette.default_colors()),
            cursor: cursor_shape(self.config.terminal.cursor_style),
            triggers: self.triggers.watched(),
        };
        let session = Session::spawn(config, move |event| {
            let _ = events.unbounded_send((id, event));
        })
        .ok()?;
        let snapshot = session.snapshot();
        self.running.insert(
            id,
            Running {
                session,
                snapshot,
                size,
            },
        );
        Some(id)
    }

    /// Control mode ended (or its gateway went away): its tabs close and its panes' sessions
    /// end.
    pub(crate) fn tmux_teardown(&mut self, gateway: SessionId) -> Task<Message> {
        let Some(t) = self.tmux.remove(&gateway) else {
            return Task::none();
        };
        for id in t.panes.values() {
            self.running.remove(id);
        }
        let tasks: Vec<_> = t
            .windows
            .values()
            .map(|w| w.tab)
            .filter(|tab| self.model.tab(*tab).is_some())
            .collect::<Vec<_>>()
            .into_iter()
            .map(|tab| self.apply(Command::CloseTab(tab)))
            .collect();
        Task::batch(tasks)
    }

    /// After a command that may close tabs: a gateway that's gone ends its control mode, and a
    /// tmux tab the user closed kills its window.
    pub(crate) fn tmux_prune(&mut self) -> Task<Message> {
        let ended: Vec<SessionId> = self
            .tmux
            .keys()
            .copied()
            .filter(|g| self.model.tab_for_session(*g).is_none())
            .collect();
        let mut tasks = Vec::new();
        for gateway in ended {
            tasks.push(self.tmux_teardown(gateway));
        }
        let gateways: Vec<SessionId> = self.tmux.keys().copied().collect();
        for gateway in gateways {
            let Some(t) = self.tmux.get_mut(&gateway) else {
                continue;
            };
            let closed: Vec<u32> = t
                .windows
                .iter()
                .filter(|(_, w)| self.model.tab(w.tab).is_none())
                .map(|(id, _)| *id)
                .collect();
            for window in closed {
                t.windows.remove(&window);
                t.client.kill_window(window);
            }
            self.tmux_sync_panes(gateway);
        }
        Task::batch(tasks)
    }

    /// The gateway and window a tmux tab shows.
    pub(crate) fn tmux_window_of(&self, tab: TabId) -> Option<(SessionId, u32)> {
        self.tmux.iter().find_map(|(gateway, t)| {
            t.windows
                .iter()
                .find(|(_, w)| w.tab == tab)
                .map(|(window, _)| (*gateway, *window))
        })
    }

    /// The gateway and pane of a tmux pane's session.
    fn tmux_pane_of(&self, id: SessionId) -> Option<(SessionId, u32)> {
        self.tmux.iter().find_map(|(gateway, t)| {
            t.panes
                .iter()
                .find(|(_, s)| **s == id)
                .map(|(pane, _)| (*gateway, *pane))
        })
    }

    /// The session of a tmux tab's focused pane.
    pub(crate) fn tmux_focused_pane(&self, tab: TabId) -> Option<SessionId> {
        let (gateway, window) = self.tmux_window_of(tab)?;
        let t = self.tmux.get(&gateway)?;
        let w = t.windows.get(&window)?;
        let pane = w
            .active_pane
            .or_else(|| w.layout.panes().first().map(|p| p.0))?;
        t.panes.get(&pane).copied()
    }

    /// Whether `id` is a tmux pane's session.
    pub(crate) fn is_tmux_pane(id: SessionId) -> bool {
        id.0 >= PANE_IDS
    }

    /// A tmux pane's view was clicked while it wasn't the focused one: tmux selects it, and
    /// its tab's pane takes the focus.
    pub(crate) fn tmux_pane_clicked(&mut self, id: SessionId) -> Task<Message> {
        let Some((gateway, pane)) = self.tmux_pane_of(id) else {
            return Task::none();
        };
        let Some(t) = self.tmux.get_mut(&gateway) else {
            return Task::none();
        };
        t.client.select_pane(pane);
        let mut tab = None;
        for w in t.windows.values_mut() {
            if w.layout.panes().iter().any(|p| p.0 == pane) {
                w.active_pane = Some(pane);
                tab = Some(w.tab);
            }
        }
        match tab {
            Some(tab) => self.apply(Command::SelectTab(tab)),
            None => Task::none(),
        }
    }

    /// The tmux tab body was laid out at `bounds`: tmux lays its windows out for that many
    /// cells (one size for the whole client).
    pub(crate) fn tmux_bounds(&mut self, tab: TabId, bounds: Rectangle) -> Task<Message> {
        let Some((gateway, _)) = self.tmux_window_of(tab) else {
            return Task::none();
        };
        let cols = (bounds.width / self.metrics.cell_width).floor().max(1.0) as u16;
        let rows = (bounds.height / self.metrics.cell_height).floor().max(1.0) as u16;
        if let Some(t) = self.tmux.get(&gateway) {
            t.client.set_size(cols, rows);
        }
        Task::none()
    }

    /// Typing in a gateway while tmux runs there: Esc detaches; the rest would be read as
    /// commands, so it's dropped. `false` when `id` isn't a gateway in control mode.
    pub(crate) fn tmux_gateway_input(&mut self, id: SessionId, bytes: &[u8]) -> bool {
        let Some(t) = self.tmux.get(&id) else {
            return false;
        };
        if bytes == b"\x1b" {
            t.client.detach();
        }
        true
    }

    /// A model command while a tmux tab is focused, done by tmux instead when it's about panes
    /// or tabs there: splits split the focused pane, a new tab is a new window. `None` for the
    /// app to apply it as usual.
    pub(crate) fn tmux_command(&mut self, command: &Command) -> Option<Task<Message>> {
        let tab = self.model.focused_tab();
        if tab.kind != TabKind::Tmux {
            return None;
        }
        let (gateway, window) = self.tmux_window_of(tab.id)?;
        let t = self.tmux.get(&gateway)?;
        let w = t.windows.get(&window)?;
        let pane = w
            .active_pane
            .or_else(|| w.layout.panes().first().map(|p| p.0))?;
        match command {
            Command::SplitPane(direction) => {
                t.client.split(
                    pane,
                    matches!(direction, Direction::Left | Direction::Right),
                );
                Some(Task::none())
            }
            Command::NewTab => {
                t.client.new_window();
                Some(Task::none())
            }
            _ => None,
        }
    }
}
