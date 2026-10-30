//! The control socket: lets `macaroni-cli` (and any program) drive the app.
//!
//! One thread accepts connections; each connection gets a reader thread (lines in, parsed into
//! [`Request`]s and forwarded to the UI) and a writer thread (lines out: responses, and events
//! for subscribers). All state changes happen on the UI thread through the same `Command`s the
//! keyboard uses.
//!
//! [`events`] is an iced subscription: it binds the socket and forwards everything the threads
//! report as [`Message::Ipc`]. `App::ipc_message` answers each [`Request`] with [`App::call`]
//! (or, for `read_page`, later, when the browser delivers the text). Events for `subscribe`
//! callers are sent by [`App::broadcast`], called from `App::apply`, the effect handlers, title
//! changes and `App::attention`. The protocol types live in `macaroni-ipc`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Write};
use std::sync::mpsc;
use std::thread;

use iced::Task;
use iced::futures::channel::mpsc::{UnboundedSender, unbounded};
use iced::futures::{SinkExt, Stream, StreamExt};
use macaroni_core::{
    Agent, AgentState, BrowserId, Command, Direction, Pane, PaneId, SessionId, Tab, TabId, TabKind,
    WorkspaceId,
};
use macaroni_ipc::{
    AgentInfo, BrowseAction, Call, Event, Listener, PaneInfo, Request, Response, Screen, TabInfo,
    Tree, WorkspaceInfo,
};
use macaroni_term::Snapshot;
use serde_json::{Value, json};

use crate::browser::BrowserMessage;
use crate::state::Running;
use crate::{App, Message};

/// A connection's number, unique for the app's lifetime.
pub type ConnId = u64;

/// How many messages from the socket threads the subscription's channel holds before they wait.
const EVENT_BUFFER: usize = 64;

/// What the socket threads tell the app.
#[derive(Debug, Clone)]
pub enum IpcMessage {
    /// The socket is bound and accepting.
    Listening,
    /// The socket couldn't be bound (another macaroni has it, say) or the accept thread
    /// couldn't start: the app runs without the API and says why.
    Failed(String),
    /// A client connected; lines sent on the channel go out to it.
    Opened(ConnId, mpsc::Sender<String>),
    /// A client sent a well-formed request.
    Request(ConnId, Request),
    /// A client sent a line that isn't a request; answered with an error.
    Invalid(ConnId, String),
    /// A client disconnected.
    Closed(ConnId),
}

/// The app's side of the control API.
pub struct IpcState {
    /// Where shells are told to connect (`MACARONI_SOCKET`); `None` if listening failed.
    pub endpoint: Option<String>,
    /// Each open connection's outgoing lines, written by its writer thread.
    connections: BTreeMap<ConnId, mpsc::Sender<String>>,
    /// Connections that called `subscribe` and get every [`Event`].
    subscribers: BTreeSet<ConnId>,
    /// Remembered so `tab_closed` can name the tab after the model has forgotten it.
    pub session_tabs: BTreeMap<SessionId, TabId>,
    /// The same for browsers, so a browser tab's `tab_closed` can name it too.
    pub browser_tabs: BTreeMap<BrowserId, TabId>,
    /// Tasks from commands run for the call being answered (the quit after the last workspace
    /// closes), handed back to iced once the call has been answered.
    tasks: Vec<Task<Message>>,
}

impl IpcState {
    /// No connections yet, at `endpoint` (cleared if listening fails).
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint: Some(endpoint),
            connections: BTreeMap::new(),
            subscribers: BTreeSet::new(),
            session_tabs: BTreeMap::new(),
            browser_tabs: BTreeMap::new(),
            tasks: Vec::new(),
        }
    }
}

/// The control API's subscription: binds the socket, starts the accept thread, and forwards
/// everything the socket threads report.
///
/// Side effects: binds the per-user socket or pipe (once; iced keeps the subscription alive)
/// and spawns the `ipc-accept` thread.
pub fn events() -> impl Stream<Item = Message> {
    iced::stream::channel(EVENT_BUFFER, async |mut output| {
        let (sender, mut receiver) = unbounded();
        match macaroni_ipc::listen(&macaroni_ipc::endpoint()) {
            Ok(listener) => {
                let _ = output.send(Message::Ipc(IpcMessage::Listening)).await;
                let spawned = thread::Builder::new()
                    .name("ipc-accept".into())
                    .spawn(move || accept(listener, sender));
                if let Err(err) = spawned {
                    let _ = output
                        .send(Message::Ipc(IpcMessage::Failed(err.to_string())))
                        .await;
                }
            }
            Err(err) => {
                let _ = output
                    .send(Message::Ipc(IpcMessage::Failed(err.to_string())))
                    .await;
            }
        }
        while let Some(message) = receiver.next().await {
            let _ = output.send(Message::Ipc(message)).await;
        }
    })
}

/// The accept thread: for each client, registers it with the app and starts its writer and
/// reader threads. Returns once the app has gone away.
fn accept(listener: Listener, to_app: UnboundedSender<IpcMessage>) {
    let mut next: ConnId = 0;
    loop {
        // A failed accept is one client's problem; keep serving the rest.
        let Ok(stream) = macaroni_ipc::accept(&listener) else {
            continue;
        };
        next += 1;
        let id = next;
        // Shared by the reader and writer threads; reads and writes on a socket or pipe don't
        // need `&mut`.
        let stream = std::sync::Arc::new(stream);
        let writer = stream.clone();
        let (out_tx, out_rx) = mpsc::channel::<String>();
        if to_app
            .unbounded_send(IpcMessage::Opened(id, out_tx))
            .is_err()
        {
            return;
        }
        let _ = thread::Builder::new()
            .name(format!("ipc-write-{id}"))
            .spawn(move || {
                let mut send = &*writer;
                for line in out_rx {
                    if send
                        .write_all(line.as_bytes())
                        .and_then(|()| send.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            });
        let to_app = to_app.clone();
        let _ = thread::Builder::new()
            .name(format!("ipc-read-{id}"))
            .spawn(move || {
                let mut reader = BufReader::new(&*stream);
                loop {
                    let message = match macaroni_ipc::read_line::<_, Request>(&mut reader) {
                        Ok(Some(request)) => IpcMessage::Request(id, request),
                        Ok(None) => break,
                        // A bad line is answered; the connection stays open for the next one.
                        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                            IpcMessage::Invalid(id, err.to_string())
                        }
                        Err(_) => break,
                    };
                    if to_app.unbounded_send(message).is_err() {
                        return;
                    }
                }
                let _ = to_app.unbounded_send(IpcMessage::Closed(id));
            });
    }
}

/// `value` as one JSON line, newline included.
fn line<T: serde::Serialize>(value: &T) -> String {
    let mut s = serde_json::to_string(value).unwrap_or_default();
    s.push('\n');
    s
}

/// A protocol direction as the model's.
fn direction(d: macaroni_ipc::Direction) -> Direction {
    match d {
        macaroni_ipc::Direction::Right => Direction::Right,
        macaroni_ipc::Direction::Down => Direction::Down,
        macaroni_ipc::Direction::Left => Direction::Left,
        macaroni_ipc::Direction::Up => Direction::Up,
    }
}

impl App {
    /// Handles what the socket threads report: tracks connections, answers requests, and shows
    /// a banner if the socket couldn't be bound. Never returns a task.
    pub(crate) fn ipc_message(&mut self, message: IpcMessage) -> Task<Message> {
        match message {
            IpcMessage::Listening => {}
            IpcMessage::Failed(err) => {
                // Terminals started from now on don't get a `MACARONI_SOCKET` that goes nowhere.
                self.ipc.endpoint = None;
                self.config_errors
                    .push(format!("control socket unavailable: {err}"));
            }
            IpcMessage::Opened(id, sender) => {
                self.ipc.connections.insert(id, sender);
            }
            IpcMessage::Closed(id) => {
                self.ipc.connections.remove(&id);
                self.ipc.subscribers.remove(&id);
            }
            // The request's id couldn't be read, so the answer goes out with id 0.
            IpcMessage::Invalid(id, err) => self.reply(
                id,
                Response {
                    id: 0,
                    result: None,
                    error: Some(format!("invalid request: {err}")),
                },
            ),
            IpcMessage::Request(conn, request) => {
                // The page's text arrives later, from the browser.
                if let Call::ReadPage { tab } = request.call {
                    if let Err(err) = self.read_page(conn, request.id, tab) {
                        self.respond(conn, request.id, Err(err));
                    }
                    return Task::none();
                }
                let result = self.call(conn, request.call);
                // Answered first, so a call that quits the app still gets its reply queued.
                self.respond(conn, request.id, result);
                return Task::batch(std::mem::take(&mut self.ipc.tasks));
            }
        }
        Task::none()
    }

    /// Answers request `id` on `conn` with a result or an error.
    pub(crate) fn respond(&mut self, conn: ConnId, id: u64, result: Result<Value, String>) {
        let response = match result {
            Ok(result) => Response {
                id,
                result: Some(result),
                error: None,
            },
            Err(error) => Response {
                id,
                result: None,
                error: Some(error),
            },
        };
        self.reply(conn, response);
    }

    /// Asks a browser for its page's text, to answer request `id` on `conn` once it arrives
    /// (as [`BrowserMessage::Text`], matched through `App.text_requests`). Errs if there's no
    /// such browser or it hasn't started.
    fn read_page(&mut self, conn: ConnId, id: u64, tab: Option<u64>) -> Result<(), String> {
        let browser = self.browser_target(tab)?;
        let (Some(events), Some(b)) = (
            self.browser_events.clone(),
            self.browsers.get(&browser).and_then(|p| p.browser.as_ref()),
        ) else {
            return Err("that browser hasn't started yet (its tab hasn't been shown)".into());
        };
        self.next_text_request += 1;
        let request = self.next_text_request;
        self.text_requests.insert(request, (conn, id));
        b.read_text(move |text| {
            let _ = events.unbounded_send((browser, BrowserMessage::Text(request, text)));
        });
        Ok(())
    }

    /// The browser a call means: `tab` if it's a browser tab; for a terminal tab (or none, the
    /// focused tab), the browser on screen in that tab's workspace, else any in that workspace.
    fn browser_target(&self, tab: Option<u64>) -> Result<BrowserId, String> {
        let tab = self.tab(tab)?;
        if let Some(TabKind::Browser(id)) = self.model.tab(tab).map(|t| t.kind.clone()) {
            return Ok(id);
        }
        let ws = self
            .workspace_of_tab(tab)
            .and_then(|w| self.model.workspace(w))
            .ok_or("no such tab")?;
        let panes: Vec<_> = ws
            .layout
            .leaves()
            .into_iter()
            .filter_map(|p| self.model.pane(p))
            .collect();
        let browser_of = |t: &TabId| match self.model.tab(*t)?.kind {
            TabKind::Browser(id) => Some(id),
            TabKind::Terminal(_) => None,
        };
        panes
            .iter()
            .find_map(|p| browser_of(&p.active))
            .or_else(|| panes.iter().flat_map(|p| &p.tabs).find_map(browser_of))
            .ok_or_else(|| {
                "no browser in this workspace; open one with `macaroni-cli browser open <url>`"
                    .to_owned()
            })
    }

    /// A browser that has started; errs if its tab was never shown.
    fn browser_ready(&self, id: BrowserId) -> Result<&macaroni_browser::Browser, String> {
        self.browsers
            .get(&id)
            .and_then(|p| p.browser.as_ref())
            .ok_or_else(|| "that browser hasn't started yet (its tab hasn't been shown)".to_owned())
    }

    /// Sends a response line to a connection, forgetting the connection if its writer is gone.
    fn reply(&mut self, conn: ConnId, response: Response) {
        if let Some(sender) = self.ipc.connections.get(&conn)
            && sender.send(line(&response)).is_err()
        {
            self.ipc.connections.remove(&conn);
        }
    }

    /// Sends `event` to every subscriber, dropping the ones whose connection has closed.
    pub(crate) fn broadcast(&mut self, event: Event) {
        if self.ipc.subscribers.is_empty() {
            return;
        }
        let text = line(&event);
        let connections = &self.ipc.connections;
        self.ipc.subscribers.retain(|id| {
            connections
                .get(id)
                .is_some_and(|s| s.send(text.clone()).is_ok())
        });
    }

    /// A call's tab: the one named, if it exists, else the focused tab.
    fn tab(&self, tab: Option<u64>) -> Result<TabId, String> {
        match tab {
            None => Ok(self.model.focused_tab().id),
            Some(id) => self
                .model
                .tab(TabId(id))
                .map(|t| t.id)
                .ok_or_else(|| format!("no tab {id}")),
        }
    }

    /// A terminal tab's session; errs for a browser tab.
    fn session(&self, tab: TabId) -> Result<SessionId, String> {
        match self.model.tab(tab).ok_or("no such tab")?.kind {
            TabKind::Terminal(session) => Ok(session),
            TabKind::Browser(_) => Err(format!("tab {} is a browser, not a terminal", tab.0)),
        }
    }

    /// A call's workspace: the one named, if it exists, else the active workspace.
    fn workspace_id(&self, id: Option<u64>) -> Result<WorkspaceId, String> {
        match id {
            None => Ok(self.model.active_workspace().id),
            Some(id) => self
                .model
                .workspace(WorkspaceId(id))
                .map(|w| w.id)
                .ok_or_else(|| format!("no workspace {id}")),
        }
    }

    /// The focused workspace, pane and tab, which calls that create or focus things answer with.
    fn focused_ids(&self) -> Value {
        let ws = self.model.active_workspace();
        json!({
            "workspace": ws.id.0,
            "pane": ws.focused.0,
            "tab": self.model.focused_tab().id.0,
        })
    }

    /// Every workspace, pane and tab, as `list` reports them.
    fn tree(&self) -> Tree {
        let active = self.model.active_workspace().id;
        let workspaces = self
            .model
            .workspaces()
            .iter()
            .map(|ws| WorkspaceInfo {
                id: ws.id.0,
                name: ws.name.clone(),
                active: ws.id == active,
                panes: ws
                    .layout
                    .leaves()
                    .into_iter()
                    .filter_map(|p| self.model.pane(p))
                    .map(|pane| PaneInfo {
                        id: pane.id.0,
                        focused: pane.id == ws.focused,
                        tabs: pane
                            .tabs
                            .iter()
                            .filter_map(|t| self.model.tab(*t))
                            .map(|tab| tab_info(tab, pane))
                            .collect(),
                    })
                    .collect(),
            })
            .collect();
        Tree { workspaces }
    }

    /// Carries out one request (all but `read_page`, which [`App::ipc_message`] handles) and
    /// returns its result, or an error message for the caller.
    fn call(&mut self, conn: ConnId, call: Call) -> Result<Value, String> {
        match call {
            Call::List => to_json(self.tree()),
            Call::Subscribe => {
                self.ipc.subscribers.insert(conn);
                Ok(json!({ "subscribed": true }))
            }
            Call::NewWorkspace { name } => self.new_workspace(name),
            Call::SelectWorkspace { workspace } => {
                self.workspace_command(Some(workspace), Command::SelectWorkspace)
            }
            Call::RenameWorkspace { workspace, name } => {
                self.workspace_command(Some(workspace), |id| Command::RenameWorkspace(id, name))
            }
            Call::CloseWorkspace { workspace } => {
                self.workspace_command(workspace, Command::CloseWorkspace)
            }
            Call::Split { tab, direction: d } => {
                self.focus_then(tab, Some(Command::SplitPane(direction(d))))
            }
            Call::NewTab { tab } => self.focus_then(tab, Some(Command::NewTab)),
            Call::Focus { tab } => self.focus_then(Some(tab), None),
            Call::CloseTab { tab } => self.tab_command(tab, Command::CloseTab),
            Call::SendText { tab, text } => self.send_text(tab, &text),
            Call::ReadScreen { tab } => self.read_screen(tab),
            Call::Notify { tab, title, body } => self.notify(tab, title, body),
            Call::ClearAttention { tab } => self.tab_command(tab, Command::ClearAttention),
            Call::Agent {
                tab,
                state,
                detail,
                session_id,
                cwd,
            } => self.agent(tab, state, detail, session_id, cwd),
            Call::AgentSummary { tab, summary } => {
                self.tab_command(tab, |tab| Command::SetAgentSummary { tab, summary })
            }
            Call::OpenBrowser { tab, url, split } => self.open_browser(tab, url, split),
            Call::Navigate { tab, url } => self.navigate(tab, &url),
            Call::Browse { tab, action } => self.browse(tab, action),
            Call::ReadPage { .. } => Err("read_page is answered asynchronously".into()),
        }
    }

    /// Applies a command for a call. Commands can't fail (calls check their ids first), so
    /// there's no error to answer with. The task `apply` returns (the quit, after the last
    /// workspace closes) is kept and returned once the call is answered.
    fn run(&mut self, command: Command) {
        let task = self.apply(command);
        self.ipc.tasks.push(task);
    }

    /// Opens a workspace, names it if asked, and answers with what's focused in it.
    fn new_workspace(&mut self, name: Option<String>) -> Result<Value, String> {
        self.run(Command::NewWorkspace);
        if let Some(name) = name {
            let id = self.model.active_workspace().id;
            self.run(Command::RenameWorkspace(id, name));
        }
        Ok(self.focused_ids())
    }

    /// Applies `command` to a workspace (`None`: the active one).
    fn workspace_command(
        &mut self,
        workspace: Option<u64>,
        command: impl FnOnce(WorkspaceId) -> Command,
    ) -> Result<Value, String> {
        let id = self.workspace_id(workspace)?;
        self.run(command(id));
        Ok(json!({}))
    }

    /// Applies `command` to a tab (`None`: the focused one).
    fn tab_command(
        &mut self,
        tab: Option<u64>,
        command: impl FnOnce(TabId) -> Command,
    ) -> Result<Value, String> {
        let tab = self.tab(tab)?;
        self.run(command(tab));
        Ok(json!({}))
    }

    /// Focuses a tab, then applies `then` there (a split or a new tab), and answers with what's
    /// focused after.
    fn focus_then(&mut self, tab: Option<u64>, then: Option<Command>) -> Result<Value, String> {
        let tab = self.tab(tab)?;
        self.run(Command::SelectTab(tab));
        if let Some(command) = then {
            self.run(command);
        }
        Ok(self.focused_ids())
    }

    /// The running terminal of a tab (`None`: the focused one).
    fn running_in(&self, tab: Option<u64>) -> Result<&Running, String> {
        let session = self.session(self.tab(tab)?)?;
        self.running.get(&session).ok_or_else(|| {
            "that terminal hasn't started yet (its workspace hasn't been shown)".to_owned()
        })
    }

    /// Types `text` into a tab's shell.
    fn send_text(&self, tab: Option<u64>, text: &str) -> Result<Value, String> {
        self.running_in(tab)?.session.write(text.as_bytes());
        Ok(json!({}))
    }

    /// A tab's visible text, size and cursor.
    fn read_screen(&self, tab: Option<u64>) -> Result<Value, String> {
        let snapshot = self.running_in(tab)?.session.snapshot();
        to_json(Screen {
            text: screen_text(&snapshot),
            rows: snapshot.rows.len(),
            cols: snapshot.cols,
            cursor: snapshot.cursor.map(|c| (c.row, c.col)),
        })
    }

    /// Marks a tab for attention, as a notification from its program would.
    fn notify(
        &mut self,
        tab: Option<u64>,
        title: Option<String>,
        body: String,
    ) -> Result<Value, String> {
        let tab = self.tab(tab)?;
        let session = self.session(tab)?;
        // An agent's hook reports its new state just before notifying, so the state says why:
        // idle after finishing, or waiting for input. `[agents]` can turn either off.
        let agents = &self.config.agents;
        let wanted = match self.model.tab(tab).and_then(|t| t.agent.as_ref()) {
            Some(agent) if agent.state == AgentState::Idle => agents.notify_finished,
            Some(agent) if agent.state == AgentState::NeedsInput => agents.notify_input,
            _ => true,
        };
        if !wanted {
            return Ok(json!({}));
        }
        let task = self.attention(session, title, body);
        self.ipc.tasks.push(task);
        Ok(json!({}))
    }

    /// Records what the tab's coding agent is doing, and tells subscribers.
    fn agent(
        &mut self,
        tab: Option<u64>,
        state: macaroni_ipc::AgentState,
        detail: Option<String>,
        session_id: Option<String>,
        cwd: Option<String>,
    ) -> Result<Value, String> {
        let tab = self.tab(tab)?;
        let command = match state {
            macaroni_ipc::AgentState::Ended => Command::ClearAgent(tab),
            other => Command::SetAgent {
                tab,
                state: match other {
                    macaroni_ipc::AgentState::Working => AgentState::Working,
                    macaroni_ipc::AgentState::NeedsInput => AgentState::NeedsInput,
                    _ => AgentState::Idle,
                },
                detail: detail.clone(),
                session_id,
                cwd,
            },
        };
        self.run(command);
        self.broadcast(Event::AgentChanged {
            tab: tab.0,
            state,
            detail,
        });
        Ok(json!({}))
    }

    /// Opens a browser tab at `url` (else the home page): in a new split if asked, else in the
    /// workspace's browser pane.
    fn open_browser(
        &mut self,
        tab: Option<u64>,
        url: Option<String>,
        split: Option<macaroni_ipc::Direction>,
    ) -> Result<Value, String> {
        let tab = self.tab(tab)?;
        let url = url
            .map(|u| crate::browser::resolve(&u))
            .unwrap_or_else(|| self.config.browser.home.clone());
        self.run(Command::SelectTab(tab));
        match split {
            Some(d) => self.run(Command::OpenBrowser {
                url,
                split: Some(direction(d)),
            }),
            None => {
                let task = self.open_in_browser_pane(url);
                self.ipc.tasks.push(task);
            }
        }
        Ok(self.focused_ids())
    }

    /// Sends a browser to `url`.
    fn navigate(&mut self, tab: Option<u64>, url: &str) -> Result<Value, String> {
        let id = self.browser_target(tab)?;
        let url = crate::browser::resolve(url);
        match self.browsers.get_mut(&id) {
            Some(panel) if panel.browser.is_some() => {
                panel.address = url.clone();
                if let Some(b) = &panel.browser {
                    b.navigate(&url);
                }
            }
            // Not started: it opens the new address when it does.
            _ => self.run(Command::SetBrowserUrl(id, url)),
        }
        Ok(json!({}))
    }

    /// Goes back or forward, reloads, stops, or opens devtools in a browser.
    fn browse(&self, tab: Option<u64>, action: BrowseAction) -> Result<Value, String> {
        let b = self.browser_ready(self.browser_target(tab)?)?;
        match action {
            BrowseAction::Back => b.back(),
            BrowseAction::Forward => b.forward(),
            BrowseAction::Reload => b.reload(),
            BrowseAction::Stop => b.stop(),
            BrowseAction::Devtools => b.devtools(),
        }
        Ok(json!({}))
    }

    /// The workspace holding `tab`, or `None` if there's no such tab.
    pub(crate) fn workspace_of_tab(&self, tab: TabId) -> Option<WorkspaceId> {
        let pane: PaneId = self.model.pane_of_tab(tab)?;
        self.model.workspace_of_pane(pane)
    }
}

/// A call's result as JSON.
fn to_json(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// The screen's rows as lines of text, without trailing blanks. Wide characters' spacer cells
/// are skipped and combining marks kept, so the text reads as it looks.
fn screen_text(snapshot: &Snapshot) -> String {
    let text = snapshot
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .filter(|c| !c.flags.contains(macaroni_term::CellFlags::WIDE_SPACER))
                .flat_map(|c| std::iter::once(c.ch).chain(c.zerowidth.iter().copied()))
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    text.trim_end_matches('\n').to_owned()
}

/// A tab as `list` reports it.
fn tab_info(tab: &Tab, pane: &Pane) -> TabInfo {
    TabInfo {
        id: tab.id.0,
        kind: match tab.kind {
            TabKind::Terminal(_) => macaroni_ipc::TabKind::Terminal,
            TabKind::Browser(_) => macaroni_ipc::TabKind::Browser,
        },
        url: tab.url.clone(),
        title: tab.label().map(str::to_owned),
        active: tab.id == pane.active,
        attention: tab.attention.as_ref().map(|n| n.body.clone()),
        agent: tab.agent.as_ref().map(agent_info),
    }
}

/// A tab's coding agent as `list` reports it.
fn agent_info(agent: &Agent) -> AgentInfo {
    AgentInfo {
        state: match agent.state {
            AgentState::Working => macaroni_ipc::AgentState::Working,
            AgentState::NeedsInput => macaroni_ipc::AgentState::NeedsInput,
            AgentState::Idle => macaroni_ipc::AgentState::Idle,
        },
        detail: agent.detail.clone(),
        summary: agent.summary.clone(),
        session_id: agent.session_id.clone(),
        cwd: agent.cwd.clone(),
    }
}
