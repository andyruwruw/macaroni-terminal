//! Terminal sessions: starting them, their events, and what their views report.
//!
//! A session the model asks for waits in `App.pending` until both its view has reported a
//! grid size ([`ViewEvent::Resize`]) and the event channel exists ([`App::events_ready`]),
//! whichever comes last; then [`App::spawn`] starts the shell at that size. The first one
//! shown also starts the rest, the hidden tabs', at its size ([`App::start_hidden`]). Events from the
//! session's threads arrive as [`App::session_event`], and input, selection, scrolling and
//! clicks from its view as [`App::view_event`]. Programs asking for attention go through
//! [`App::attention`], whatever the source (OSC 9/777/99, the bell, `macaroni-cli notify`).

use std::path::PathBuf;

use iced::Task;
use iced::futures::channel::mpsc::UnboundedSender;
use macaroni_core::{Command, SessionId};
use macaroni_term::{
    GridPoint, GridSize, Scroll, Session, SessionConfig, SessionEvent, encode_paste, is_openable,
    url_at,
};

use super::config::cursor_shape;
use crate::message::Message;
use crate::state::Running;
use crate::terminal::{SelectOp, ViewEvent};
use crate::{APP_NAME, App, bell, notifications, session};

impl App {
    /// The session event channel exists: start every terminal whose view already has a size.
    pub(crate) fn events_ready(
        &mut self,
        events: UnboundedSender<(SessionId, SessionEvent)>,
    ) -> Task<Message> {
        self.events = Some(events);
        let ready: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(id, size)| size.map(|s| (*id, s)))
            .collect();
        for (id, size) in ready {
            self.spawn(id, size);
        }
        Task::none()
    }

    /// Pastes clipboard text into a terminal, bracketed if the program asked for that.
    pub(crate) fn paste(&mut self, id: SessionId, text: String) -> Task<Message> {
        let Some(modes) = self.running.get(&id).map(|r| r.snapshot.modes) else {
            return Task::none();
        };
        self.view_event(id, ViewEvent::Write(encode_paste(&text, modes)))
    }

    /// Starts the terminal `id` at `size`, or remembers the size until the event channel exists.
    ///
    /// Side effects: spawns the shell (with its reader and waiter threads), types
    /// `claude --resume` into it for a restored agent, and on failure shows the error as the
    /// tab's title.
    fn spawn(&mut self, id: SessionId, size: GridSize) {
        let Some(events) = self.events.clone() else {
            self.pending.insert(id, Some(size));
            return;
        };
        self.pending.remove(&id);
        let restored = self
            .model
            .tab_for_session(id)
            .and_then(|tab| self.restored.remove(&tab))
            .unwrap_or_default();
        // A restored tab reopens where it was; a new one starts in its workspace's folder, or
        // `[terminal] default_dir` for a workspace without one. A directory that's gone
        // (deleted, an unplugged drive) would make the spawn fail, so each falls back to the
        // next, and then to the home directory.
        let folder = self
            .model
            .tab_for_session(id)
            .and_then(|tab| self.workspace_of_tab(tab))
            .and_then(|ws| self.model.workspace(ws)?.cwd.clone());
        let cwd = restored
            .cwd
            .as_ref()
            .map(PathBuf::from)
            .filter(|dir| dir.is_dir())
            .or_else(|| folder.map(PathBuf::from).filter(|dir| dir.is_dir()))
            .or_else(|| {
                self.default_folder()
                    .map(PathBuf::from)
                    .filter(|dir| dir.is_dir())
            });
        let (program, args) = self.program_for(id, &restored);
        let config = SessionConfig {
            // Blank means the platform's default shell.
            program: Some(program).filter(|p| !p.trim().is_empty()),
            args,
            cwd,
            size,
            scrollback: self.config.terminal.scrollback,
            env: self.session_env(id),
            colors: Some(self.palette.default_colors()),
            cursor: cursor_shape(self.config.terminal.cursor_style),
        };
        match Session::spawn(config, move |event| {
            let _ = events.unbounded_send((id, event));
        }) {
            Ok(session) => {
                self.resume_agent(&session, &restored);
                let snapshot = session.snapshot();
                self.running.insert(
                    id,
                    Running {
                        session,
                        snapshot,
                        size,
                    },
                );
            }
            Err(err) => {
                // Setting a title never quits, so there's no task to keep.
                let _ = self.apply(Command::SetTabTitle(
                    id,
                    Some(format!("failed to start: {err}")),
                ));
            }
        }
    }

    /// What a new terminal runs: the launch command for the first one, else the shell picked
    /// for its tab (or the SSH profile a restored tab ran, while it's still in `[[ssh]]`), else
    /// the configured shell (an empty program for the default shell).
    fn program_for(
        &mut self,
        id: SessionId,
        restored: &session::SavedTab,
    ) -> (String, Vec<String>) {
        let mut picked = self.tab_shells.remove(&id);
        if picked.is_none()
            && let Some(name) = &restored.ssh
            && let Some(profile) = self.config.ssh.iter().find(|p| p.label() == name)
        {
            self.ssh_tabs.insert(id, name.clone());
            picked = Some(crate::update::menus::ssh_shell(profile));
        }
        self.first_command.take().unwrap_or_else(|| match picked {
            Some(choice) => (choice.program, choice.args),
            None => {
                let shell = &self.config.shell;
                (
                    shell.program.clone().unwrap_or_default(),
                    shell.args.clone(),
                )
            }
        })
    }

    /// The control socket and tab id, so programs in the terminal can reach the app. Empty if
    /// the socket couldn't be bound.
    fn session_env(&self, id: SessionId) -> Vec<(String, String)> {
        let mut env = Vec::new();
        if let Some(endpoint) = &self.ipc.endpoint {
            env.push((macaroni_ipc::SOCKET_ENV.to_owned(), endpoint.clone()));
            if let Some(tab) = self.model.tab_for_session(id) {
                env.push((macaroni_ipc::TAB_ENV.to_owned(), tab.0.to_string()));
            }
        }
        env
    }

    /// Resumes the Claude Code session a restored tab had, if enabled and its id is safe to type.
    /// Typed straight away: the shell reads it once it's ready, like typeahead.
    fn resume_agent(&self, session: &Session, restored: &session::SavedTab) {
        if self.config.session.resume_agents
            && let Some(agent) = restored.agent_session.as_deref()
            && session::is_safe_session_id(agent)
        {
            session.write(format!("claude --resume {agent}\r").as_bytes());
        }
    }

    /// Handles an event from a terminal's threads: new output (re-snapshot), a title, a bell or
    /// notification, a directory change, a clipboard write, or the program exiting (which the
    /// model turns into closing the tab).
    pub(crate) fn session_event(&mut self, id: SessionId, event: SessionEvent) -> Task<Message> {
        match event {
            SessionEvent::Wakeup => {
                if let Some(r) = self.running.get_mut(&id) {
                    r.snapshot = r.session.snapshot();
                }
                Task::none()
            }
            SessionEvent::Title(title) => self.set_title(id, Some(title)),
            SessionEvent::ResetTitle => self.set_title(id, None),
            SessionEvent::Bell => {
                let muted = self
                    .model
                    .tab_for_session(id)
                    .is_some_and(|t| self.model.is_muted(t));
                if !muted {
                    bell::ring(&self.config.notifications.bell_sound);
                }
                if self.config.notifications.bell {
                    self.attention(id, None, "Bell".into())
                } else {
                    Task::none()
                }
            }
            SessionEvent::Notify(n) => self.attention(id, n.title, n.body),
            SessionEvent::Cwd(path) => self.apply(Command::SetTabCwd(id, Some(path))),
            SessionEvent::ClipboardStore(text) => iced::clipboard::write(text),
            SessionEvent::Exited(code) => self.session_exited(id, code),
        }
    }

    /// A terminal's program exited: its tab closes, except an SSH tab whose `ssh` failed (a
    /// connection that couldn't be made or was dropped), which stays to show why.
    fn session_exited(&mut self, id: SessionId, code: Option<u32>) -> Task<Message> {
        let Some(name) = self.ssh_tabs.get(&id).filter(|_| code != Some(0)).cloned() else {
            return self.apply(Command::SessionExited(id));
        };
        self.disconnected.insert(id);
        if let Some(r) = self.running.get_mut(&id) {
            let code = code.map_or_else(String::new, |c| format!(" (exit code {c})"));
            r.session.print(
                format!("\r\n\x1b[2m[{name} disconnected{code}. Press Enter to connect again.]\x1b[0m\r\n")
                    .as_bytes(),
            );
            r.snapshot = r.session.snapshot();
        }
        self.apply(Command::SetTabTitle(
            id,
            Some(format!("{name} (disconnected)")),
        ))
    }

    /// Enter in a disconnected SSH tab: runs its profile again in the same tab, at its size.
    /// The profile is looked up by name, so a removed one closes the tab instead.
    fn reconnect(&mut self, id: SessionId) -> Task<Message> {
        self.disconnected.remove(&id);
        let profile = self
            .ssh_tabs
            .get(&id)
            .and_then(|name| self.config.ssh.iter().find(|p| p.label() == name));
        let (Some(profile), Some(size)) = (profile, self.running.get(&id).map(|r| r.size)) else {
            return self.apply(Command::SessionExited(id));
        };
        self.tab_shells
            .insert(id, crate::update::menus::ssh_shell(profile));
        self.spawn(id, size);
        self.apply(Command::SetTabTitle(id, None))
    }

    /// Handles what a terminal's view reported.
    pub(crate) fn view_event(&mut self, id: SessionId, event: ViewEvent) -> Task<Message> {
        match event {
            ViewEvent::Resize(size) => self.resize_session(id, size),
            ViewEvent::Write(bytes) if self.disconnected.contains(&id) => {
                if bytes.contains(&b'\r') {
                    return self.reconnect(id);
                }
            }
            ViewEvent::Write(bytes) => self.write_input(id, &bytes),
            ViewEvent::Select(op) => self.select(id, op),
            ViewEvent::Scroll(scroll) => self.scroll(id, scroll),
            ViewEvent::Focus(focused) => self.report_focus(id, focused),
            ViewEvent::Link(at) => return self.open_link_at(id, at),
            ViewEvent::RenameItem => return self.rename_focused_item(),
            ViewEvent::Clicked => return self.view_clicked(id),
            ViewEvent::Action(action) => {
                // A shortcut lets go of the focused item, as any other key would.
                self.item_focus = None;
                return self.perform(action);
            }
        }
        Task::none()
    }

    /// Resizes a running terminal to its view, or starts one that was waiting for its size. A
    /// report of the size it already has is ignored, so the shell doesn't reflow for nothing.
    fn resize_session(&mut self, id: SessionId, size: GridSize) {
        if let Some(r) = self.running.get_mut(&id) {
            if r.size != size {
                r.session.resize(size);
                r.size = size;
                r.snapshot = r.session.snapshot();
            }
        } else if self.pending.contains_key(&id) {
            self.spawn(id, size);
            self.start_hidden(size);
        }
    }

    /// Starts every terminal that's waiting with no size, because its tab isn't shown (another
    /// workspace's, or behind its pane's open tab), at `size`: the size the terminal just shown
    /// got, the best guess there is. So after a restart every tab's shell runs and its agent
    /// resumes without the tab being opened first; showing one resizes it to its view.
    fn start_hidden(&mut self, size: GridSize) {
        let hidden: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, size)| size.is_none())
            .map(|(id, _)| *id)
            .collect();
        for id in hidden {
            self.spawn(id, size);
        }
    }

    /// Sends typed input to the shell, first dropping the selection and scrolling to the bottom.
    fn write_input(&mut self, id: SessionId, bytes: &[u8]) {
        self.item_focus = None;
        self.clear_attention(id);
        if let Some(r) = self.running.get_mut(&id) {
            if r.snapshot.has_selection {
                r.session.clear_selection();
            }
            if r.snapshot.display_offset > 0 {
                r.session.scroll(Scroll::Bottom);
            }
            if r.snapshot.has_selection || r.snapshot.display_offset > 0 {
                r.snapshot = r.session.snapshot();
            }
            // After the screen is back at the bottom: the program's echo lands where it's seen.
            r.session.write(bytes);
        }
    }

    /// Starts, extends or clears a terminal's selection.
    fn select(&mut self, id: SessionId, op: SelectOp) {
        if let Some(r) = self.running.get_mut(&id) {
            match op {
                SelectOp::Start { kind, at, side } => r.session.start_selection(kind, at, side),
                SelectOp::Update { at, side } => r.session.update_selection(at, side),
                SelectOp::Clear => r.session.clear_selection(),
            }
            r.snapshot = r.session.snapshot();
        }
    }

    /// Scrolls a terminal through its scrollback.
    fn scroll(&mut self, id: SessionId, scroll: Scroll) {
        if let Some(r) = self.running.get_mut(&id) {
            r.session.scroll(scroll);
            r.snapshot = r.session.snapshot();
        }
    }

    /// Tells the program the window gained or lost focus, if it asked to know.
    fn report_focus(&self, id: SessionId, focused: bool) {
        if let Some(r) = self.running.get(&id)
            && r.session.modes().focus_events
        {
            r.session.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    /// Opens the link at a cell: an OSC 8 hyperlink, else a URL in the text.
    fn open_link_at(&mut self, id: SessionId, at: GridPoint) -> Task<Message> {
        let url = self.running.get(&id).and_then(|r| {
            r.session
                .hyperlink_at(at)
                .filter(|u| is_openable(u))
                .or_else(|| url_at(r.snapshot.rows.get(at.row)?, at.col).map(|link| link.url))
        });
        match url {
            Some(url) => self.open_link(url),
            None => Task::none(),
        }
    }

    /// A click in an unfocused terminal selects its tab.
    fn view_clicked(&mut self, id: SessionId) -> Task<Message> {
        self.item_focus = None;
        let Some(tab) = self.model.tab_for_session(id) else {
            return Task::none();
        };
        self.clear_attention(id);
        self.apply(Command::SelectTab(tab))
    }

    /// A program asked for attention. Nothing happens if the tab is muted or the user is
    /// already looking at it; otherwise the tab is marked, subscribers are told, and an OS
    /// notification goes out (if enabled) when the tab is off screen or the window isn't
    /// focused.
    pub(crate) fn attention(
        &mut self,
        id: SessionId,
        title: Option<String>,
        body: String,
    ) -> Task<Message> {
        let Some(tab) = self.model.tab_for_session(id) else {
            return Task::none();
        };
        if self.model.is_muted(tab) || (self.window_focused && self.model.focused_tab().id == tab) {
            return Task::none();
        }
        let visible = self.model.tab_visible(tab);
        let heading = title
            .clone()
            .or_else(|| {
                self.model
                    .tab(tab)
                    .and_then(|t| t.label().map(str::to_owned))
            })
            .unwrap_or_else(|| APP_NAME.to_owned());
        if self.config.notifications.os && (!self.window_focused || !visible) {
            notifications::show(heading, body.clone());
        }
        self.broadcast(macaroni_ipc::Event::Notification {
            tab: tab.0,
            title: title.clone(),
            body: body.clone(),
        });
        self.apply(Command::Notify {
            session: id,
            title,
            body,
        })
    }

    /// Sets (or with `None`, resets) a terminal tab's title from its program and tells
    /// subscribers the label it now shows (a user-given name wins over the program's title).
    fn set_title(&mut self, id: SessionId, title: Option<String>) -> Task<Message> {
        let task = self.apply(Command::SetTabTitle(id, title));
        if let Some(tab) = self.model.tab_for_session(id) {
            let title = self
                .model
                .tab(tab)
                .and_then(|t| t.label().map(str::to_owned));
            self.broadcast(macaroni_ipc::Event::TitleChanged { tab: tab.0, title });
        }
        task
    }

    /// Clears the tab's attention marker, if it has one. Called on input and clicks, before
    /// anything else they do. Skips the command when there's nothing to clear, so typing
    /// doesn't apply (and mark the session dirty) on every key.
    fn clear_attention(&mut self, id: SessionId) {
        if let Some(tab) = self.model.tab_for_session(id)
            && self.model.tab(tab).is_some_and(|t| t.attention.is_some())
        {
            // Clearing attention never quits, so there's no task to keep.
            let _ = self.apply(Command::ClearAttention(tab));
        }
    }
}
