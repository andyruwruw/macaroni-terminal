//! Messages exchanged over the control socket, one JSON object per line.
//!
//! Client to app: `{"id": 1, "method": "split", "params": {"direction": "right"}}`.
//! App to client: `{"id": 1, "result": {...}}` or `{"id": 1, "error": "..."}`.
//! After `subscribe`, the app also sends events: `{"event": "tab_opened", ...}`.
//!
//! Every `params` field named `tab` is optional and defaults to the focused tab; the CLI fills it
//! from `MACARONI_TAB_ID` when run inside a macaroni terminal, so commands act on their own tab.
//!
//! These types are the wire contract, shared by the server (`crates/app/src/ipc.rs`) and
//! `macaroni-cli`. A [`Request`] carries a [`Call`]; the app answers with a [`Response`] whose
//! `result` is one of the structs below ([`Tree`], [`Screen`], [`Page`]), the focused ids
//! `{"workspace", "pane", "tab"}`, or `{}`. Field names are the JSON keys as written; enums are
//! `snake_case` strings. Optional fields (`#[serde(default)]`) may be left out or sent as `null`.

use serde::{Deserialize, Serialize};

/// A side of a pane, for splits: `"right"`, `"down"`, `"left"` or `"up"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// `"right"`: the new pane goes to the right.
    Right,
    /// `"down"`: the new pane goes below.
    Down,
    /// `"left"`: the new pane goes to the left.
    Left,
    /// `"up"`: the new pane goes above.
    Up,
}

/// What a coding agent in a tab is doing: `"working"`, `"needs_input"`, `"idle"` or `"ended"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// `"working"`: busy on a prompt or a tool call.
    Working,
    /// `"needs_input"`: waiting for the user (a permission prompt or a question).
    NeedsInput,
    /// `"idle"`: finished, or waiting for the next prompt.
    Idle,
    /// The agent exited; the tab goes back to being a plain terminal.
    Ended,
}

/// A method and its params: `"method": "<snake_case name>"` with the variant's fields as the
/// `"params"` object. Unit variants may omit `params`. An unknown method, a missing required
/// field or a field of the wrong type fails to deserialize, and the app answers with an error.
///
/// Results, unless a variant says otherwise: `{}`. "Focused ids" means
/// `{"workspace": <id>, "pane": <id>, "tab": <id>}`, what has focus after the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Call {
    /// `list`: the workspace / pane / tab tree. Result: [`Tree`].
    List,
    /// `new_workspace`: opens a workspace and switches to it. Result: focused ids (in the new
    /// workspace).
    NewWorkspace {
        /// The workspace's name; omitted, it gets the default name.
        #[serde(default)]
        name: Option<String>,
    },
    /// `select_workspace`: switches to a workspace.
    SelectWorkspace {
        /// The workspace's id, as in [`WorkspaceInfo::id`].
        workspace: u64,
    },
    /// `rename_workspace`: renames a workspace.
    RenameWorkspace {
        /// The workspace's id.
        workspace: u64,
        /// The new name.
        name: String,
    },
    /// `close_workspace`: closes a workspace and everything in it.
    CloseWorkspace {
        /// The workspace's id; omitted, the active workspace.
        #[serde(default)]
        workspace: Option<u64>,
    },
    /// `split`: splits the pane holding `tab`; the new pane gets a new terminal. Focuses `tab`
    /// first, like doing it by hand. Result: focused ids (the new pane and its tab).
    Split {
        /// The tab whose pane to split.
        #[serde(default)]
        tab: Option<u64>,
        /// Which side of that pane the new one goes.
        direction: Direction,
    },
    /// `new_tab`: a new terminal tab in the pane holding `tab`. Result: focused ids (the new
    /// tab).
    NewTab {
        /// The tab to open next to.
        #[serde(default)]
        tab: Option<u64>,
    },
    /// `close_tab`: closes a tab (and its terminal or browser).
    CloseTab {
        /// The tab to close.
        #[serde(default)]
        tab: Option<u64>,
    },
    /// `focus`: shows the tab: selects its workspace, pane and the tab itself. Result: focused
    /// ids.
    Focus {
        /// The tab to show. Required, unlike elsewhere.
        tab: u64,
    },
    /// `send_text`: writes `text` to the tab's shell as if typed. Include `\r` to press Enter.
    /// Fails if the tab is a browser or its terminal hasn't started yet (terminals start when
    /// first shown).
    SendText {
        /// The terminal tab to type into.
        #[serde(default)]
        tab: Option<u64>,
        /// Raw text, sent unchanged (no bracketed paste, no newline translation).
        text: String,
    },
    /// `read_screen`: the visible text of the tab. Result: [`Screen`]. Fails like
    /// [`Call::SendText`] for a browser or a terminal that hasn't started.
    ReadScreen {
        /// The terminal tab to read.
        #[serde(default)]
        tab: Option<u64>,
    },
    /// `notify`: marks the tab for attention, exactly like an OSC 777 notification from its
    /// program: ignored while the user is looking at that tab in the focused window, and an OS
    /// notification when it's out of view (if enabled). The tab must be a terminal.
    Notify {
        /// The terminal tab asking for attention.
        #[serde(default)]
        tab: Option<u64>,
        /// The OS notification's heading; omitted, the tab's title.
        #[serde(default)]
        title: Option<String>,
        /// The message, also shown in the sidebar.
        body: String,
    },
    /// `clear_attention`: clears the tab's attention mark.
    ClearAttention {
        /// The tab to clear.
        #[serde(default)]
        tab: Option<u64>,
    },
    /// `agent`: a coding agent in the tab reports what it's doing (sent by `macaroni-cli hook`).
    /// [`AgentState::Ended`] clears the agent. Also sends [`Event::AgentChanged`] to subscribers.
    Agent {
        /// The agent's tab.
        #[serde(default)]
        tab: Option<u64>,
        /// The new state.
        state: AgentState,
        /// A short line of what it's doing, e.g. "Running cargo test"; omitted, none.
        #[serde(default)]
        detail: Option<String>,
        /// The agent's session id, saved with the session so a restart can resume it; omitted,
        /// the previous one is kept.
        #[serde(default)]
        session_id: Option<String>,
        /// The agent's working directory; omitted, the previous one is kept.
        #[serde(default)]
        cwd: Option<String>,
    },
    /// `agent_summary`: a one-line summary from the agent's status line (sent by
    /// `macaroni-cli statusline`), shown in the pane's tab strip. Ignored if the tab has no
    /// agent.
    AgentSummary {
        /// The agent's tab.
        #[serde(default)]
        tab: Option<u64>,
        /// The summary; omitted, `null` or blank clears it.
        #[serde(default)]
        summary: Option<String>,
    },
    /// `subscribe`: keeps the connection open and streams [`Event`]s, one JSON line each, after
    /// the result `{"subscribed": true}`.
    Subscribe,
    /// `open_browser`: opens a browser tab. With `split`, in a new pane on that side of `tab`'s
    /// pane; without, like a link clicked in a terminal: in a browser pane of that workspace,
    /// else a new split to the right. Result: focused ids (the browser tab).
    OpenBrowser {
        /// The tab to open from (it's focused first).
        #[serde(default)]
        tab: Option<u64>,
        /// An address or search words, as typed in the address bar; omitted, `[browser] home`.
        #[serde(default)]
        url: Option<String>,
        /// Open in a new pane on this side instead.
        #[serde(default)]
        split: Option<Direction>,
    },
    /// `navigate`: loads `url`. For every browser method, `tab` may be a browser tab or a
    /// terminal tab; a terminal means the browser on screen in its workspace.
    Navigate {
        /// A browser tab, or a terminal tab meaning its workspace's browser.
        #[serde(default)]
        tab: Option<u64>,
        /// An address or search words, as typed in the address bar.
        url: String,
    },
    /// `browse`: goes back or forward, reloads, stops, or opens DevTools. Needs the browser
    /// started (shown at least once).
    Browse {
        /// A browser tab, or a terminal tab meaning its workspace's browser.
        #[serde(default)]
        tab: Option<u64>,
        /// What to do.
        action: BrowseAction,
    },
    /// `read_page`: the page's address, title and visible text. Result: [`Page`], sent once
    /// Chromium delivers the text, so other responses on the connection may arrive first. Needs
    /// the browser started.
    ReadPage {
        /// A browser tab, or a terminal tab meaning its workspace's browser.
        #[serde(default)]
        tab: Option<u64>,
    },
}

/// What [`Call::Browse`] does: `"back"`, `"forward"`, `"reload"`, `"stop"` or `"devtools"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowseAction {
    /// `"back"`: the previous page in history.
    Back,
    /// `"forward"`: the next page in history.
    Forward,
    /// `"reload"`: loads the page again.
    Reload,
    /// `"stop"`: stops loading.
    Stop,
    /// `"devtools"`: opens Chromium DevTools for the page.
    Devtools,
}

/// What a tab holds: `"terminal"` or `"browser"`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    /// `"terminal"`: a shell. The default, for servers that predate browser tabs.
    #[default]
    Terminal,
    /// `"browser"`: a Chromium page.
    Browser,
}

/// The result of [`Call::ReadPage`]: `{"url": ..., "title": ..., "text": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    /// The page's address; `null` if Chromium didn't report one.
    pub url: Option<String>,
    /// The page's title; `null` if it has none.
    pub title: Option<String>,
    /// The page's visible text.
    pub text: String,
}

/// One request line: `{"id": <u64>, "method": "...", "params": {...}}`. The call's `method` and
/// `params` sit beside `id` at the top level (flattened).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    /// Chosen by the client and echoed in the [`Response`], so answers can be matched to
    /// requests.
    pub id: u64,
    /// The method and its params.
    #[serde(flatten)]
    pub call: Call,
}

/// One response line: `{"id": <u64>, "result": ...}` on success or `{"id": <u64>, "error":
/// "..."}` on failure. Whichever field is `None` is left out of the JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The [`Request::id`] this answers.
    pub id: u64,
    /// The call's result (see [`Call`] for each method's); absent on error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// A readable message saying why the call failed; absent on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Something that happened in the app, streamed to connections that called
/// [`Call::Subscribe`]: `{"event": "<snake_case name>", ...fields}`, with the variant's fields at
/// the top level beside `event`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// `workspace_created`: a workspace opened.
    WorkspaceCreated {
        /// Its id.
        workspace: u64,
        /// Its name.
        name: String,
    },
    /// `workspace_closed`: a workspace closed.
    WorkspaceClosed {
        /// Its id.
        workspace: u64,
    },
    /// `workspace_renamed`: a workspace's name changed.
    WorkspaceRenamed {
        /// Its id.
        workspace: u64,
        /// The new name.
        name: String,
    },
    /// `tab_opened`: a terminal or browser tab opened.
    TabOpened {
        /// Its id.
        tab: u64,
        /// The workspace it opened in; `0` if the app couldn't tell.
        workspace: u64,
    },
    /// `tab_closed`: a tab closed (its terminal stopped, or its browser closed).
    TabClosed {
        /// Its id.
        tab: u64,
    },
    /// `title_changed`: a tab's title changed (set by its program or its page).
    TitleChanged {
        /// Its id.
        tab: u64,
        /// The tab's label now: the user's name for it, else the program's or page's title;
        /// `null` if it has neither.
        title: Option<String>,
    },
    /// `notification`: a tab asked for attention (OSC 9/777/99, the bell, or [`Call::Notify`]).
    Notification {
        /// The tab asking.
        tab: u64,
        /// The notification's heading, if it had one.
        title: Option<String>,
        /// The message.
        body: String,
    },
    /// `agent_changed`: a [`Call::Agent`] report arrived.
    AgentChanged {
        /// The agent's tab.
        tab: u64,
        /// The reported state; [`AgentState::Ended`] means the agent is gone.
        state: AgentState,
        /// The reported detail line, if any.
        detail: Option<String>,
    },
}

/// The result of [`Call::List`]: `{"workspaces": [...]}`, in sidebar order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    /// Every workspace.
    pub workspaces: Vec<WorkspaceInfo>,
}

/// One workspace in a [`Tree`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Its id, for `workspace` params.
    pub id: u64,
    /// Its name.
    pub name: String,
    /// Whether it's the one on screen.
    pub active: bool,
    /// Its panes, in layout order.
    pub panes: Vec<PaneInfo>,
}

/// One pane in a [`WorkspaceInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneInfo {
    /// Its id.
    pub id: u64,
    /// Whether it's the workspace's focused pane.
    pub focused: bool,
    /// Its tabs, in strip order.
    pub tabs: Vec<TabInfo>,
}

/// One tab in a [`PaneInfo`]. `agent`, `kind` and `url` may be absent (older servers); they
/// default to `null`, `"terminal"` and `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabInfo {
    /// Its id, for `tab` params.
    pub id: u64,
    /// The tab's label (the user's name for it, else its title); `null` if it has neither.
    pub title: Option<String>,
    /// Whether it's the pane's selected tab.
    pub active: bool,
    /// The body of its pending attention request; `null` if none.
    pub attention: Option<String>,
    /// The coding agent running in it, if any.
    #[serde(default)]
    pub agent: Option<AgentInfo>,
    /// Terminal or browser.
    #[serde(default)]
    pub kind: TabKind,
    /// A browser tab's address.
    #[serde(default)]
    pub url: Option<String>,
}

/// A tab's coding agent in a [`TabInfo`], as last reported through [`Call::Agent`] and
/// [`Call::AgentSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Its state; never [`AgentState::Ended`], since an ended agent is removed.
    pub state: AgentState,
    /// What it's doing, e.g. "Editing main.rs".
    pub detail: Option<String>,
    /// The status line summary, e.g. model, context use and cost.
    pub summary: Option<String>,
    /// Its session id, for resuming it.
    pub session_id: Option<String>,
    /// Its working directory.
    pub cwd: Option<String>,
}

/// The result of [`Call::ReadScreen`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    /// Visible rows, trailing spaces trimmed, joined with `\n`.
    pub text: String,
    /// The terminal's height in rows.
    pub rows: usize,
    /// The terminal's width in columns.
    pub cols: usize,
    /// The cursor as `[row, column]`, both from 0 at the top left; `null` when it's hidden or
    /// scrolled out of view.
    pub cursor: Option<(usize, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_read_naturally() {
        let request = Request {
            id: 7,
            call: Call::Split {
                tab: None,
                direction: Direction::Right,
            },
        };
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(
            json,
            r#"{"id":7,"method":"split","params":{"tab":null,"direction":"right"}}"#
        );
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);

        let hand_written: Request = serde_json::from_str(
            r#"{"id":1,"method":"notify","params":{"body":"Waiting for input"}}"#,
        )
        .unwrap();
        assert_eq!(
            hand_written.call,
            Call::Notify {
                tab: None,
                title: None,
                body: "Waiting for input".into()
            }
        );
        let list: Request = serde_json::from_str(r#"{"id":2,"method":"list"}"#).unwrap();
        assert_eq!(list.call, Call::List);
    }

    #[test]
    fn browser_calls_read_naturally() {
        let open: Request = serde_json::from_str(
            r#"{"id":3,"method":"open_browser","params":{"url":"example.com","split":"down"}}"#,
        )
        .unwrap();
        assert_eq!(
            open.call,
            Call::OpenBrowser {
                tab: None,
                url: Some("example.com".into()),
                split: Some(Direction::Down),
            }
        );
        let browse: Request =
            serde_json::from_str(r#"{"id":4,"method":"browse","params":{"action":"devtools"}}"#)
                .unwrap();
        assert_eq!(
            browse.call,
            Call::Browse {
                tab: None,
                action: BrowseAction::Devtools,
            }
        );
        let read: Request =
            serde_json::from_str(r#"{"id":5,"method":"read_page","params":{"tab":9}}"#).unwrap();
        assert_eq!(read.call, Call::ReadPage { tab: Some(9) });
    }

    #[test]
    fn unknown_methods_are_errors() {
        assert!(serde_json::from_str::<Request>(r#"{"id":1,"method":"format_disk"}"#).is_err());
    }

    #[test]
    fn responses_and_events_are_compact() {
        let ok = Response {
            id: 1,
            result: Some(serde_json::json!({"tab": 3})),
            error: None,
        };
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"id":1,"result":{"tab":3}}"#
        );
        let event = Event::TabClosed { tab: 3 };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"event":"tab_closed","tab":3}"#
        );
    }
}
