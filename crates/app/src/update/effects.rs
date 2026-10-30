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
        let effects = self.model.apply(command);
        self.broadcast_workspace_changes(&before);
        let task = self.run_effects(effects);
        let changed: Vec<_> = open_tabs(&self.model)
            .into_iter()
            .filter(|now| !open.contains(now))
            .map(|(pane, _)| pane)
            .collect();
        let task = Task::batch(
            std::iter::once(task).chain(changed.into_iter().map(|pane| self.reveal_tab(pane))),
        );
        // Every command counts as a change (most do change something worth saving). After a
        // quit `restore` is off, so the save below does nothing and the forgotten file stays
        // gone.
        self.session_dirty = true;
        self.save_session(false);
        task
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
        let saved = session::Saved {
            sidebar: Some(self.sidebar),
            ..session::capture(&self.model)
        };
        if let Err(err) = session::save(&path, &saved) {
            self.config_errors.push(format!(
                "couldn't save the session to {}: {err}",
                path.display()
            ));
        }
        self.session_saved = Some(Instant::now());
        self.session_dirty = false;
    }

    /// Carries out what the model asked for. Only quitting returns a task.
    pub(crate) fn run_effects(&mut self, effects: Vec<Effect>) -> Task<Message> {
        for effect in effects {
            match effect {
                Effect::SpawnSession(id) => self.session_requested(id),
                Effect::KillSession(id) => self.kill_session(id),
                Effect::OpenBrowser(id) => self.browser_requested(id),
                Effect::CloseBrowser(id) => self.close_browser(id),
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
        Task::none()
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
        self.tab_shells.remove(&id);
        self.running.remove(&id);
        if let Some(tab) = self.ipc.session_tabs.remove(&id) {
            self.broadcast(macaroni_ipc::Event::TabClosed { tab: tab.0 });
        }
        if self.search.as_ref().is_some_and(|s| s.session == id) {
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
        self.browsers.remove(&id);
        if let Some(tab) = self.ipc.browser_tabs.remove(&id) {
            self.broadcast(macaroni_ipc::Event::TabClosed { tab: tab.0 });
        }
        if self.page_focus == Some(id) {
            self.page_focus = None;
            if let Some(parent) = self.parent {
                macaroni_browser::focus_window(parent);
            }
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
    /// lets CEF finish closing them before `browser::shutdown` runs `cef::shutdown` after the
    /// event loop; it gives up after [`BROWSER_CLOSE_TIMEOUT`].
    pub(crate) fn exit(&mut self) -> Task<Message> {
        self.browsers.clear();
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

/// Every workspace's id and name, to compare before and after a command.
fn workspace_names(model: &Model) -> Vec<(WorkspaceId, String)> {
    model
        .workspaces()
        .iter()
        .map(|w| (w.id, w.name.clone()))
        .collect()
}
