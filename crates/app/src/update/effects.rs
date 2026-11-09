//! Applying model commands: running their effects, telling subscribers, and saving the session.
//!
//! [`App::apply`] is the one way handlers change the model. The model answers each command
//! with [`Effect`]s, carried out here: sessions are queued to start (`update/terminals.rs`
//! starts them once their views have a size) or dropped, browser panels are added or removed
//! (`update/browser.rs` opens their windows), and quitting closes the browsers first.

use std::time::{Duration, Instant};

use iced::Task;
use macaroni_core::{BrowserId, Command, Effect, Model, PaneId, SessionId, TabId, WorkspaceId};

use crate::browser::Panel;
use crate::message::Message;
use crate::{App, session};

/// The least time between two saves of the session while it keeps changing.
const SESSION_SAVE_INTERVAL: Duration = Duration::from_secs(1);
/// How long quitting waits for the browsers to close.
const BROWSER_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

impl App {
    /// Applies `command` to the model, runs its effects, tells subscribers what happened to
    /// workspaces, and saves the session if a save is due. A pane whose open tab changed (a new
    /// tab, say) scrolls its strip to show it. The task is none except after the last workspace
    /// closed, when it's the one that quits, and those scrolls: callers must return it.
    pub(crate) fn apply(&mut self, command: Command) -> Task<Message> {
        let before = workspace_names(&self.model);
        let open = open_tabs(&self.model);
        // Tabs the user closes can be reopened; a program that ended (`exit`, a page's
        // `window.close()`) or quitting isn't a close to undo. Only these commands close tabs,
        // so others skip the bookkeeping (titles and folders change on every prompt).
        let closable = matches!(
            command,
            Command::CloseTab(_)
                | Command::CloseActiveTab
                | Command::ClosePane
                | Command::CloseWorkspace(_)
                | Command::CloseActiveWorkspace
                | Command::CloseWindow(_)
        );
        let closing = if closable {
            self.closing_tabs()
        } else {
            Vec::new()
        };
        let effects = self.model.apply(command);
        if !effects.iter().any(|e| matches!(e, Effect::Quit)) {
            self.remember_closed(closing);
        }
        let mut tmux = Task::none();
        if closable {
            self.git_tabs
                .retain(|tab, _| self.model.tab(*tab).is_some());
            self.agent_tabs
                .retain(|tab, _| self.model.tab(*tab).is_some());
            self.file_tabs
                .retain(|tab, _| self.model.tab(*tab).is_some());
            for page in self.orphaned_pages() {
                if self.page_focus == Some(page) {
                    self.blur_page();
                }
                self.browsers.remove(&page);
                if let Some(server) = crate::preview::server::running() {
                    server.forget(page);
                }
            }
            tmux = self.tmux_prune();
        }
        self.broadcast_workspace_changes(&before);
        let task = self.run_effects(effects);
        let changed: Vec<_> = open_tabs(&self.model)
            .into_iter()
            .filter(|now| !open.contains(now))
            .map(|(pane, _)| pane)
            .collect();
        let task = Task::batch(
            [task, tmux]
                .into_iter()
                .chain(changed.into_iter().map(|pane| self.reveal_tab(pane))),
        );
        // Every command counts as a change (most do change something worth saving). After a
        // quit `restore` is off, so the save below does nothing and the forgotten file stays
        // gone.
        self.session_dirty = true;
        self.save_session(false);
        task
    }

    /// What every tab would be reopened from, before a command that may close some of them;
    /// [`App::remember_closed`] keeps those that are gone afterwards.
    fn closing_tabs(&self) -> Vec<(TabId, crate::ClosedTab)> {
        let mut tabs = Vec::new();
        for ws in self.model.all_workspaces() {
            for pane in ws.layout.leaves() {
                for &tab in self
                    .model
                    .pane(pane)
                    .map(|p| p.tabs.as_slice())
                    .unwrap_or_default()
                {
                    if self
                        .model
                        .tab(tab)
                        .is_some_and(|t| t.kind == macaroni_core::TabKind::Tmux)
                    {
                        continue;
                    }
                    tabs.push((
                        tab,
                        crate::ClosedTab {
                            saved: crate::session::saved_tab(
                                &self.model,
                                &self.restored,
                                &self.ssh_tabs,
                                tab,
                            ),
                            pane,
                            workspace: ws.id,
                        },
                    ));
                }
            }
        }
        tabs
    }

    /// Keeps the tabs of `before` that no longer exist, newest last, at most
    /// [`crate::CLOSED_TABS_KEPT`].
    fn remember_closed(&mut self, before: Vec<(TabId, crate::ClosedTab)>) {
        for (tab, closed) in before {
            if self.model.tab(tab).is_none() {
                self.closed_tabs.push(closed);
            }
        }
        let extra = self
            .closed_tabs
            .len()
            .saturating_sub(crate::CLOSED_TABS_KEPT);
        self.closed_tabs.drain(..extra);
    }

    /// Reopens the most recently closed tab: in its old pane if that's still there, else the
    /// focused one. A terminal starts in its old directory and resumes its agent session
    /// (as a restored tab does); a browser tab reopens its page, and a Git, agent setup or file
    /// tab its folder or file. Its name and look come back.
    pub(crate) fn reopen_tab(&mut self) -> Task<Message> {
        let Some(closed) = self.closed_tabs.pop() else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        if self.model.pane(closed.pane).is_some() {
            tasks.push(self.apply(Command::FocusPane(closed.pane)));
        } else if self.model.workspace(closed.workspace).is_some() {
            tasks.push(self.apply(Command::SelectWorkspace(closed.workspace)));
        }
        let saved = closed.saved;
        if let Some(url) = saved.url.clone() {
            tasks.push(self.apply(Command::OpenBrowser { url, split: None }));
        } else if let Some(folder) = saved.git.clone() {
            tasks.push(self.apply(Command::OpenGit {
                folder,
                split: None,
            }));
        } else if let Some(folder) = saved.agent_setup.clone() {
            tasks.push(self.apply(Command::OpenAgentSetup {
                folder,
                split: None,
            }));
            let tab = self.model.focused_tab().id;
            tasks.push(self.agent_update(tab, crate::view::agent_tab::AgentMessage::Refresh));
        } else if let Some(path) = saved.file.clone() {
            tasks.push(self.apply(Command::OpenFile { path, split: None }));
            let tab = self.model.focused_tab().id;
            tasks.push(self.file_update(
                tab,
                crate::view::file_tab::FileMessage::Refresh { force: true },
            ));
        } else {
            // The terminal spawns later, at its real size; `spawn` reads this then, as for a
            // restored tab.
            tasks.push(self.apply(Command::NewTab));
            let tab = self.model.focused_tab().id;
            self.restored.insert(tab, saved.clone());
        }
        let tab = self.model.focused_tab().id;
        tasks.push(self.apply(Command::RenameTab(tab, saved.name.clone())));
        tasks.push(self.apply(Command::SetTabIcon(tab, saved.icon.clone())));
        tasks.push(self.apply(Command::SetTabColor(tab, saved.color.clone())));
        if saved.muted {
            tasks.push(self.apply(Command::SetTabMuted(tab, true)));
        }
        Task::batch(tasks)
    }

    /// Applies each command in turn, batching their tasks.
    pub(crate) fn apply_all(&mut self, commands: Vec<Command>) -> Task<Message> {
        Task::batch(
            commands
                .into_iter()
                .map(|c| self.apply(c))
                .collect::<Vec<_>>(),
        )
    }

    /// Broadcasts the workspaces closed, created or renamed since `before`.
    fn broadcast_workspace_changes(&mut self, before: &[(WorkspaceId, String)]) {
        let after = workspace_names(&self.model);
        for (gone, _) in before
            .iter()
            .filter(|(id, _)| !after.iter().any(|(a, _)| a == id))
        {
            self.broadcast(macaroni_ipc::Event::WorkspaceClosed { workspace: gone.0 });
        }
        for (id, name) in &after {
            let event = match before.iter().find(|(b, _)| b == id) {
                None => macaroni_ipc::Event::WorkspaceCreated {
                    workspace: id.0,
                    name: name.clone(),
                },
                Some((_, old)) if old != name => macaroni_ipc::Event::WorkspaceRenamed {
                    workspace: id.0,
                    name: name.clone(),
                },
                Some(_) => continue,
            };
            self.broadcast(event);
        }
    }

    /// Saves the layout for the next launch; at most once a second unless `now`. Closing the
    /// last workspace (which quits) forgets the session instead, so the next launch starts fresh.
    ///
    /// Side effects: writes `session.json`. A failure is shown in the error banner.
    ///
    /// A change made within a second of the last save is saved by the next [`Message::SaveTick`]
    /// (see `save_ticks` in `main.rs`).
    pub(crate) fn save_session(&mut self, now: bool) {
        // A one-off `macaroni <program>` run doesn't replace the saved session.
        if !self.config.session.restore || self.ran_program {
            return;
        }
        let due = self
            .session_saved
            .is_none_or(|at| at.elapsed() >= SESSION_SAVE_INTERVAL);
        if !self.session_dirty || !(now || due) {
            return;
        }
        let path = session::path(&self.config_dir);
        let mut saved = session::Saved {
            sidebar: Some(self.sidebar),
            file_tree: self.file_tree,
            ..session::capture(
                &self.model,
                &self.restored,
                &self.ssh_tabs,
                &self.scrollback_files,
            )
        };
        // The commands run in each workspace are ours too.
        for (saved, ws) in saved.workspaces.iter_mut().zip(self.model.all_workspaces()) {
            saved.commands = self.history.saved(ws.id, &ws.layout.leaves());
        }
        // The model knows each window's active workspace; where it is on screen is ours.
        for (window, placed) in saved.windows.iter_mut().zip(self.saved_windows()) {
            *window = session::SavedWindow {
                active: window.active,
                ..placed
            };
        }
        if let Err(err) = session::save(&path, &saved) {
            self.config_errors.push(format!(
                "couldn't save the session to {}: {err}",
                path.display()
            ));
        }
        self.session_saved = Some(Instant::now());
        self.session_dirty = false;
    }

    /// Carries out what the model asked for. Opening, closing or raising a window, and
    /// quitting, return tasks.
    pub(crate) fn run_effects(&mut self, effects: Vec<Effect>) -> Task<Message> {
        let mut tasks = Vec::new();
        for effect in effects {
            match effect {
                Effect::SpawnSession(id) => self.session_requested(id),
                Effect::KillSession(id) => self.kill_session(id),
                Effect::OpenBrowser(id) => self.browser_requested(id),
                Effect::CloseBrowser(id) => self.close_browser(id),
                Effect::OpenWindow(w) => tasks.push(self.open_window(w, None)),
                Effect::CloseWindow(w) => tasks.push(self.close_window(w)),
                Effect::FocusWindow(w) => tasks.push(self.raise_window(w)),
                Effect::Quit => {
                    // The model lists `Quit` last (after killing every session and closing
                    // every browser), so returning here skips nothing. Turning `restore` off
                    // stops the saves `apply` and `close_requested` would still try.
                    session::forget(&session::path(&self.config_dir));
                    self.session_dirty = false;
                    self.config.session.restore = false;
                    return self.exit();
                }
            }
        }
        Task::batch(tasks)
    }

    /// Queues a terminal to start once its view has a size, and tells subscribers its tab
    /// opened.
    fn session_requested(&mut self, id: SessionId) {
        self.pending.insert(id, None);
        if let Some(tab) = self.model.tab_for_session(id) {
            self.ipc.session_tabs.insert(id, tab);
            self.broadcast_tab_opened(tab);
        }
    }

    /// Stops a terminal and forgets everything kept for it. Dropping its [`crate::state::Running`]
    /// kills the process. Tells subscribers its tab closed.
    fn kill_session(&mut self, id: SessionId) {
        self.pending.remove(&id);
        // A shell still starting is dropped (and killed) when `session_started` finds no entry.
        if let Some(starting) = self.starting.remove(&id)
            && let Some(tab) = starting.tab
        {
            self.restored.remove(&tab);
        }
        self.booting.remove(&id);
        self.typeahead.remove(&id);
        self.tab_shells.remove(&id);
        self.ssh_tabs.remove(&id);
        self.disconnected.remove(&id);
        self.running.remove(&id);
        if let Some(tab) = self.ipc.session_tabs.remove(&id) {
            self.broadcast(macaroni_ipc::Event::TabClosed { tab: tab.0 });
        }
        if self
            .search
            .as_ref()
            .is_some_and(|s| s.target == crate::state::SearchTarget::Terminal(id))
        {
            self.search = None;
        }
    }

    /// Adds a browser panel; its browser starts the first time it's shown. Tells subscribers
    /// its tab opened.
    fn browser_requested(&mut self, id: BrowserId) {
        let url = self
            .model
            .tab_for_browser(id)
            .and_then(|t| self.model.tab(t)?.url.clone())
            .unwrap_or_default();
        self.browsers.insert(id, Panel::new(&url));
        if let Some(tab) = self.model.tab_for_browser(id) {
            self.ipc.browser_tabs.insert(id, tab);
            self.broadcast_tab_opened(tab);
        }
    }

    /// Closes a browser, giving keyboard focus back to macaroni if its page had it. Tells
    /// subscribers its tab closed.
    fn close_browser(&mut self, id: BrowserId) {
        // While the page still exists: it hands the keyboard back to macaroni's window.
        if self.page_focus == Some(id) {
            self.blur_page();
        }
        self.browsers.remove(&id);
        if self
            .search
            .as_ref()
            .is_some_and(|s| s.target == crate::state::SearchTarget::Page(id))
        {
            self.search = None;
        }
        if let Some(tab) = self.ipc.browser_tabs.remove(&id) {
            self.broadcast(macaroni_ipc::Event::TabClosed { tab: tab.0 });
        }
    }

    /// Tells subscribers a tab opened, and in which workspace.
    fn broadcast_tab_opened(&mut self, tab: TabId) {
        let workspace = self.workspace_of_tab(tab).map_or(0, |w| w.0);
        self.broadcast(macaroni_ipc::Event::TabOpened {
            tab: tab.0,
            workspace,
        });
    }

    /// Closes the browsers, waits (off the UI thread) for them to finish, then quits. The wait
    /// lets Chromium finish closing them before `macaroni_browser::shutdown` runs `cef::shutdown`
    /// after the event loop; it gives up after [`BROWSER_CLOSE_TIMEOUT`].
    pub(crate) fn exit(&mut self) -> Task<Message> {
        // No saves past this point: while the browsers close, dying agents can still report
        // (Claude Code's SessionEnd clears the tab's agent), and a save would drop what the
        // close just saved.
        self.config.session.restore = false;
        self.browsers.clear();
        self.vscode_web.stop();
        Task::future(async {
            macaroni_browser::wait_closed(BROWSER_CLOSE_TIMEOUT);
        })
        .then(|()| iced::exit())
    }
}

/// Each pane of the open workspace with its open tab, to compare before and after a command.
fn open_tabs(model: &Model) -> Vec<(PaneId, TabId)> {
    model
        .active_workspace()
        .layout
        .leaves()
        .into_iter()
        .filter_map(|pane| Some((pane, model.pane(pane)?.active)))
        .collect()
}

/// Every workspace's id and name, in every window, to compare before and after a command.
fn workspace_names(model: &Model) -> Vec<(WorkspaceId, String)> {
    model
        .all_workspaces()
        .iter()
        .map(|w| (w.id, w.name.clone()))
        .collect()
}
