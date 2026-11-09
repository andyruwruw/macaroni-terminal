//! tmux control mode (`tmux -CC`): a tmux session's windows and panes shown as macaroni's own,
//! with tmux carried over an ordinary session (the gateway: a local terminal, or SSH).
//!
//! - **Detection.** `tmux -CC` starts by printing `ESC P 1000 p`. The gateway's reader passes its
//!   output through a `Gate`, which spots that, makes a [`TmuxClient`], and from then on hands
//!   whole lines to it instead of the emulator, until `%exit` (and the `ESC \` after it) gives
//!   the terminal back.
//! - **The protocol.** Lines starting `%` are notifications (`%output`, `%layout-change`,
//!   `%window-add`...), except `%begin` ... `%end` / `%error`, which wrap the reply to a command.
//!   Commands go to tmux as lines on the gateway's input, and replies come back in the same order,
//!   so the client's pending queue says what each reply is for. A reply that isn't to our command (tmux
//!   marks ours with flag 1; older versions don't, and then only the first one, for the command
//!   that started the client, isn't ours) is skipped.
//! - **Panes** are sessions too ([`crate::Source::Tmux`]): output tmux reports for a pane
//!   (`%output %3 ...`, with bytes below space and `\` as octal escapes) is sent to its session's
//!   reader, and its input goes back as `send-keys -H`. A pane first shows what tmux captured of
//!   it (its history and screen, and the cursor), then its live output; output before the
//!   capture is in the capture, so it's dropped.
//! - **The app** gets [`TmuxEvent`]s on the gateway session: the windows with their layouts
//!   (parsed into [`Layout`] trees in cells), layout changes, closes, renames, the active pane
//!   and the detach. It sizes the client with [`TmuxClient::set_size`] and drives it with the
//!   other methods (select, split, kill, new window, detach).

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, mpsc};

use crate::pipe::ChunkReader;
use crate::session::SessionEvent;
use crate::source::{Control, Opened};

/// What starts control mode in the gateway's output.
const START: &[u8] = b"\x1bP1000p";

/// What follows `%exit` and ends control mode.
const END: &[u8] = b"\x1b\\";

/// How many lines of a pane's history its first capture brings.
const HISTORY_LINES: usize = 2000;

/// The most bytes one `send-keys` carries.
const KEYS_PER_COMMAND: usize = 256;

/// What tmux says that the app acts on, raised on the gateway session as
/// [`SessionEvent::Tmux`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxEvent {
    /// Control mode started; the gateway session's [`crate::Session::tmux`] has the client.
    Attached,
    /// Every window of the session, in order (on attach, and after a window is added).
    Windows(Vec<TmuxWindow>),
    /// A window's panes were rearranged, added or removed.
    Layout {
        /// The window, by its number (`@3` is 3).
        window: u32,
        /// Its panes, in cells.
        layout: Layout,
    },
    /// A window closed.
    WindowClosed(u32),
    /// A window was renamed.
    Renamed {
        /// The window.
        window: u32,
        /// Its name.
        name: String,
    },
    /// The focused pane of a window changed.
    ActivePane {
        /// The window.
        window: u32,
        /// The pane, by its number (`%5` is 5).
        pane: u32,
    },
    /// Control mode ended (detached, or the session ended); the gateway is a terminal again.
    Detached(String),
}

/// One tmux window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxWindow {
    /// Its number (`@3` is 3).
    pub id: u32,
    /// Its name.
    pub name: String,
    /// Its panes, in cells.
    pub layout: Layout,
    /// Its focused pane.
    pub active_pane: Option<u32>,
    /// Whether it's the session's current window.
    pub active: bool,
}

/// A window's panes as tmux lays them out: a tree whose leaves are panes, every node with its
/// place in cells. Panes are a cell apart; the gaps are tmux's borders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Columns from the window's left.
    pub x: u16,
    /// Rows from the window's top.
    pub y: u16,
    /// Columns wide.
    pub width: u16,
    /// Rows tall.
    pub height: u16,
    /// A pane, or how it's split.
    pub kind: LayoutKind,
}

/// What a [`Layout`] node is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutKind {
    /// A pane, by number.
    Pane(u32),
    /// Children side by side, left to right.
    Row(Vec<Layout>),
    /// Children stacked, top to bottom.
    Column(Vec<Layout>),
}

impl Layout {
    /// Every pane with its place: `(pane, x, y, width, height)`, in cells.
    pub fn panes(&self) -> Vec<(u32, u16, u16, u16, u16)> {
        match &self.kind {
            LayoutKind::Pane(id) => vec![(*id, self.x, self.y, self.width, self.height)],
            LayoutKind::Row(children) | LayoutKind::Column(children) => {
                children.iter().flat_map(Layout::panes).collect()
            }
        }
    }

    /// Reads a tmux layout string: a checksum, then `WxH,X,Y` followed by `,PANE`, or children
    /// in `{...}` (side by side) or `[...]` (stacked).
    pub fn parse(text: &str) -> Option<Self> {
        let (_, tree) = text.split_once(',')?;
        let mut rest = tree;
        let layout = parse_node(&mut rest)?;
        rest.is_empty().then_some(layout)
    }
}

/// One node from the front of `rest`, which is advanced past it.
fn parse_node(rest: &mut &str) -> Option<Layout> {
    let number = |rest: &mut &str| -> Option<u16> {
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let n = rest[..end].parse().ok()?;
        *rest = &rest[end..];
        Some(n)
    };
    let width = number(rest)?;
    *rest = rest.strip_prefix('x')?;
    let height = number(rest)?;
    *rest = rest.strip_prefix(',')?;
    let x = number(rest)?;
    *rest = rest.strip_prefix(',')?;
    let y = number(rest)?;
    let kind = if let Some(after) = rest.strip_prefix(',') {
        *rest = after;
        LayoutKind::Pane(u32::from(number(rest)?))
    } else {
        let (close, row) = match rest.chars().next()? {
            '{' => ('}', true),
            '[' => (']', false),
            _ => return None,
        };
        *rest = &rest[1..];
        let mut children = vec![parse_node(rest)?];
        while let Some(after) = rest.strip_prefix(',') {
            *rest = after;
            children.push(parse_node(rest)?);
        }
        *rest = rest.strip_prefix(close)?;
        if row {
            LayoutKind::Row(children)
        } else {
            LayoutKind::Column(children)
        }
    };
    Some(Layout {
        x,
        y,
        width,
        height,
        kind,
    })
}

/// A tmux pane's session ([`crate::Source::Tmux`]): the client it's under and its number.
#[derive(Clone)]
pub struct TmuxPane {
    /// The control client carrying it.
    pub client: Arc<TmuxClient>,
    /// The pane's number.
    pub pane: u32,
}

impl fmt::Debug for TmuxPane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TmuxPane(%{})", self.pane)
    }
}

impl PartialEq for TmuxPane {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.client, &other.client) && self.pane == other.pane
    }
}

impl Eq for TmuxPane {}

/// One control mode connection: sends tmux commands on the gateway's input and routes what
/// tmux reports.
pub struct TmuxClient {
    /// The gateway's input, which tmux reads commands from.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Raises events on the gateway session.
    notify: Arc<dyn Fn(SessionEvent) + Send + Sync>,
    /// Everything that changes.
    inner: Mutex<Inner>,
}

impl fmt::Debug for TmuxClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TmuxClient")
    }
}

/// A [`TmuxClient`]'s state.
#[derive(Default)]
struct Inner {
    /// What each reply still to come is for, in the order the commands went out.
    pending: VecDeque<Pending>,
    /// The reply being read: what it's for (`None`: not ours) and its lines so far.
    reply: Option<(Option<Pending>, Vec<String>)>,
    /// A reply came that wasn't marked as ours or not (tmux before 3.2): the next unmarked one
    /// is ours.
    unmarked_seen: bool,
    /// Every pane tmux has mentioned, by number.
    panes: HashMap<u32, PaneState>,
    /// Which panes each window has, from its layout.
    windows: HashMap<u32, Vec<u32>>,
    /// The size last asked for.
    size: Option<(u16, u16)>,
    /// Control mode ended.
    ended: bool,
}

/// What a reply is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// `list-windows`: the windows to report.
    Windows,
    /// A pane's capture and then its cursor.
    Capture(u32),
    /// Nothing to read.
    Ignore,
}

/// A pane's output routing.
#[derive(Default)]
struct PaneState {
    /// Its session's reader, once the session is open.
    out: Option<mpsc::Sender<Vec<u8>>>,
    /// Its session's waiter, told when the pane closes.
    closed: Option<mpsc::Sender<()>>,
    /// What to show once the session opens: the capture, then output since.
    backlog: Vec<u8>,
    /// The capture has arrived; output from now on is shown.
    captured: bool,
}

impl TmuxClient {
    /// A client sending commands on `writer` and raising events with `notify`.
    fn new(
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        notify: Arc<dyn Fn(SessionEvent) + Send + Sync>,
    ) -> Self {
        Self {
            writer,
            notify,
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Sends one command line; its reply is read as `pending`.
    fn command(&self, line: &str, pending: Pending) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.ended {
            return;
        }
        inner.pending.push_back(pending);
        drop(inner);
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer
                .write_all(format!("{line}\n").as_bytes())
                .and_then(|()| writer.flush());
        }
    }

    /// Asks for every window, for [`TmuxEvent::Windows`].
    fn list_windows(&self) {
        self.command(
            "list-windows -F \"#{window_id}\t#{window_layout}\t#{pane_id}\t#{window_active}\t#{window_name}\"",
            Pending::Windows,
        );
    }

    /// Makes tmux lay its windows out for a client `cols` by `rows` cells. Repeats are skipped.
    pub fn set_size(&self, cols: u16, rows: u16) {
        let size = (cols.max(2), rows.max(2));
        if let Ok(mut inner) = self.inner.lock() {
            if inner.size == Some(size) {
                return;
            }
            inner.size = Some(size);
        }
        self.command(
            &format!("refresh-client -C {},{}", size.0, size.1),
            Pending::Ignore,
        );
    }

    /// Focuses a pane (and its window).
    pub fn select_pane(&self, pane: u32) {
        self.command(&format!("select-pane -t %{pane}"), Pending::Ignore);
    }

    /// Shows a window as the session's current one.
    pub fn select_window(&self, window: u32) {
        self.command(&format!("select-window -t @{window}"), Pending::Ignore);
    }

    /// Splits a pane: side by side when `beside`, else one above the other.
    pub fn split(&self, pane: u32, beside: bool) {
        let flag = if beside { "-h" } else { "-v" };
        self.command(&format!("split-window {flag} -t %{pane}"), Pending::Ignore);
    }

    /// Closes a pane and what runs in it.
    pub fn kill_pane(&self, pane: u32) {
        self.command(&format!("kill-pane -t %{pane}"), Pending::Ignore);
    }

    /// Opens a new window.
    pub fn new_window(&self) {
        self.command("new-window", Pending::Ignore);
    }

    /// Closes a window and what runs in it.
    pub fn kill_window(&self, window: u32) {
        self.command(&format!("kill-window -t @{window}"), Pending::Ignore);
    }

    /// Leaves tmux running and gives the gateway terminal back.
    pub fn detach(&self) {
        self.command("detach-client", Pending::Ignore);
    }

    /// Whether control mode has ended.
    pub fn ended(&self) -> bool {
        self.inner.lock().map_or(true, |inner| inner.ended)
    }

    /// Types `bytes` into a pane.
    fn send_keys(&self, pane: u32, bytes: &[u8]) {
        for chunk in bytes.chunks(KEYS_PER_COMMAND) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
            self.command(
                &format!("send-keys -t %{pane} -H {}", hex.join(" ")),
                Pending::Ignore,
            );
        }
    }

    /// Opens a pane's session ends: its output so far, then whatever comes.
    fn open_pane(self: &Arc<Self>, pane: u32) -> Opened {
        let (out_tx, out_rx) = mpsc::channel();
        let (closed_tx, closed_rx) = mpsc::channel();
        if let Ok(mut inner) = self.inner.lock() {
            let ended = inner.ended;
            let state = inner.panes.entry(pane).or_default();
            if !state.backlog.is_empty() {
                let _ = out_tx.send(std::mem::take(&mut state.backlog));
            }
            if ended {
                drop(closed_tx);
            } else {
                state.out = Some(out_tx);
                state.closed = Some(closed_tx);
            }
        }
        Opened {
            reader: Box::new(ChunkReader::new(out_rx)),
            writer: Box::new(PaneWriter {
                client: self.clone(),
                pane,
            }),
            control: Box::new(PaneControl),
            waiter: Box::new(move || {
                // Told, or the client gone: either way the pane has ended.
                let _ = closed_rx.recv();
                Some(0)
            }),
        }
    }

    /// Reads one line of control mode output (without its line ending).
    fn line(self: &Arc<Self>, line: &str) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if let Some((pending, lines)) = &mut inner.reply {
            if line.starts_with("%end ") || line.starts_with("%error ") {
                let pending = *pending;
                let lines = std::mem::take(lines);
                inner.reply = None;
                let failed = line.starts_with("%error ");
                drop(inner);
                if let Some(pending) = pending
                    && !failed
                {
                    self.reply(pending, lines);
                }
            } else {
                lines.push(line.to_owned());
            }
            return;
        }
        if let Some(rest) = line.strip_prefix("%begin ") {
            // `%begin time number flags`: flag 1 marks a reply to this client's command.
            let ours = match rest.split_whitespace().nth(2) {
                Some(flags) => flags.parse::<u32>().is_ok_and(|f| f & 1 == 1),
                None => std::mem::replace(&mut inner.unmarked_seen, true),
            };
            let pending = if ours {
                inner.pending.pop_front()
            } else {
                None
            };
            inner.reply = Some((pending, Vec::new()));
            return;
        }
        drop(inner);
        self.notification(line);
    }

    /// Acts on a reply to one of our commands.
    fn reply(self: &Arc<Self>, pending: Pending, lines: Vec<String>) {
        match pending {
            Pending::Ignore => {}
            Pending::Windows => {
                let windows: Vec<TmuxWindow> =
                    lines.iter().filter_map(|l| window_line(l)).collect();
                for window in &windows {
                    self.track_layout(window.id, &window.layout);
                }
                (self.notify)(SessionEvent::Tmux(TmuxEvent::Windows(windows)));
            }
            Pending::Capture(pane) => {
                // The last line is the cursor (`display-message`), the rest the capture.
                let mut lines = lines;
                let cursor = lines.pop().unwrap_or_default();
                let mut screen = lines.join("\r\n").into_bytes();
                let mut parts = cursor.split_whitespace().map(|n| n.parse::<u32>().ok());
                if let (Some(Some(x)), Some(Some(y))) = (parts.next(), parts.next()) {
                    screen.extend_from_slice(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
                }
                if let Ok(mut inner) = self.inner.lock() {
                    let state = inner.panes.entry(pane).or_default();
                    state.captured = true;
                    match &state.out {
                        Some(out) => {
                            let _ = out.send(screen);
                        }
                        // Output that came since goes after the capture.
                        None => {
                            screen.extend_from_slice(&state.backlog);
                            state.backlog = screen;
                        }
                    }
                }
            }
        }
    }

    /// Acts on a notification line.
    fn notification(self: &Arc<Self>, line: &str) {
        let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
        match name {
            "%output" => {
                let Some((pane, data)) = rest.split_once(' ') else {
                    return;
                };
                if let Some(pane) = number(pane, '%') {
                    self.output(pane, unescape(data));
                }
            }
            "%extended-output" => {
                // `%extended-output %pane age ... : data`
                let Some((head, data)) = rest.split_once(" : ") else {
                    return;
                };
                if let Some(pane) = head.split_whitespace().next().and_then(|p| number(p, '%')) {
                    self.output(pane, unescape(data));
                }
            }
            "%layout-change" => {
                let mut parts = rest.split_whitespace();
                let (Some(window), Some(layout)) = (
                    parts.next().and_then(|w| number(w, '@')),
                    parts.next().and_then(Layout::parse),
                ) else {
                    return;
                };
                self.track_layout(window, &layout);
                (self.notify)(SessionEvent::Tmux(TmuxEvent::Layout { window, layout }));
            }
            "%window-add" | "%sessions-changed" | "%session-changed" => self.list_windows(),
            "%window-close" | "%unlinked-window-close" => {
                if let Some(window) = number(rest.trim(), '@') {
                    self.track_layout_panes(window, Vec::new());
                    (self.notify)(SessionEvent::Tmux(TmuxEvent::WindowClosed(window)));
                }
            }
            "%window-renamed" => {
                let (window, name) = rest.split_once(' ').unwrap_or((rest, ""));
                if let Some(window) = number(window, '@') {
                    (self.notify)(SessionEvent::Tmux(TmuxEvent::Renamed {
                        window,
                        name: name.to_owned(),
                    }));
                }
            }
            "%window-pane-changed" => {
                let mut parts = rest.split_whitespace();
                if let (Some(window), Some(pane)) = (
                    parts.next().and_then(|w| number(w, '@')),
                    parts.next().and_then(|p| number(p, '%')),
                ) {
                    (self.notify)(SessionEvent::Tmux(TmuxEvent::ActivePane { window, pane }));
                }
            }
            "%exit" => self.end(rest.trim()),
            _ => {}
        }
    }

    /// Routes a pane's output: to its session, else kept for it once captured, else dropped
    /// (the capture will have it).
    fn output(&self, pane: u32, data: Vec<u8>) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let state = inner.panes.entry(pane).or_default();
        if !state.captured {
            return;
        }
        match &state.out {
            Some(out) => {
                let _ = out.send(data);
            }
            None => state.backlog.extend_from_slice(&data),
        }
    }

    /// Notes a window's panes from its layout: new ones are captured, gone ones closed.
    fn track_layout(self: &Arc<Self>, window: u32, layout: &Layout) {
        let panes = layout.panes().into_iter().map(|p| p.0).collect();
        self.track_layout_panes(window, panes);
    }

    /// [`TmuxClient::track_layout`] with the window's panes listed.
    fn track_layout_panes(self: &Arc<Self>, window: u32, panes: Vec<u32>) {
        let (new, gone) = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            let before = inner
                .windows
                .insert(window, panes.clone())
                .unwrap_or_default();
            if panes.is_empty() {
                inner.windows.remove(&window);
            }
            let new: Vec<u32> = panes
                .iter()
                .copied()
                .filter(|p| !inner.panes.contains_key(p))
                .collect();
            for pane in &new {
                inner.panes.insert(*pane, PaneState::default());
            }
            let still: Vec<u32> = inner.windows.values().flatten().copied().collect();
            let gone: Vec<PaneState> = before
                .into_iter()
                .filter(|p| !still.contains(p))
                .filter_map(|p| inner.panes.remove(&p))
                .collect();
            (new, gone)
        };
        // Dropping a pane's senders ends its session's reader and waiter.
        drop(gone);
        for pane in new {
            self.command(
                &format!(
                    "capture-pane -p -e -J -t %{pane} -S -{HISTORY_LINES} ; display-message -p -t %{pane} \"#{{cursor_x}} #{{cursor_y}}\""
                ),
                Pending::Capture(pane),
            );
        }
    }

    /// Ends control mode: every pane's session ends and the app is told.
    fn end(&self, reason: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.ended {
                return;
            }
            inner.ended = true;
            inner.panes.clear();
            inner.pending.clear();
        }
        (self.notify)(SessionEvent::Tmux(TmuxEvent::Detached(reason.to_owned())));
    }
}

/// Opens a tmux pane's session: see [`TmuxClient::open_pane`].
pub(crate) fn open(pane: &TmuxPane) -> Opened {
    pane.client.open_pane(pane.pane)
}

/// Types into a pane through its client.
struct PaneWriter {
    /// The client.
    client: Arc<TmuxClient>,
    /// The pane.
    pane: u32,
}

impl Write for PaneWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.client.send_keys(self.pane, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A pane's control: tmux sizes panes from the client's size, so a view's resize changes only
/// its grid.
struct PaneControl;

impl Control for PaneControl {
    fn resize(&self, _: crate::grid::GridSize) {}
}

/// Watches a gateway's output for control mode. Created by the gateway's reader thread.
pub(crate) struct Gate {
    /// The gateway's input, for the client's commands.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// The gateway's events.
    notify: Arc<dyn Fn(SessionEvent) + Send + Sync>,
    /// Where the gateway session finds its client.
    slot: Arc<Mutex<Option<Arc<TmuxClient>>>>,
    /// The client while control mode runs.
    client: Option<Arc<TmuxClient>>,
    /// An unfinished line of control mode output.
    line: Vec<u8>,
    /// Output ending in a prefix of [`START`], kept to see whether the rest follows.
    held: Vec<u8>,
}

impl Gate {
    /// A gate for a gateway with this input, events and client slot.
    pub(crate) fn new(
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        notify: Arc<dyn Fn(SessionEvent) + Send + Sync>,
        slot: Arc<Mutex<Option<Arc<TmuxClient>>>>,
    ) -> Self {
        Self {
            writer,
            notify,
            slot,
            client: None,
            line: Vec::new(),
            held: Vec::new(),
        }
    }

    /// Takes a chunk of the gateway's output; what the terminal should show of it.
    pub(crate) fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut input = std::mem::take(&mut self.held);
        input.extend_from_slice(bytes);
        let mut shown = Vec::new();
        let mut rest: &[u8] = &input;
        while !rest.is_empty() {
            match &self.client {
                None => match find(rest, START) {
                    Some(at) => {
                        shown.extend_from_slice(&rest[..at]);
                        rest = &rest[at + START.len()..];
                        self.start();
                        shown.extend_from_slice(
                            b"\r\n\x1b[2m[tmux control mode: the session's windows open as tabs beside this one. Esc detaches.]\x1b[0m\r\n",
                        );
                    }
                    None => {
                        // A possible start cut off at the chunk's end waits for the next chunk.
                        let keep = (1..START.len())
                            .rev()
                            .find(|&n| rest.ends_with(&START[..n]))
                            .unwrap_or(0);
                        shown.extend_from_slice(&rest[..rest.len() - keep]);
                        self.held = rest[rest.len() - keep..].to_vec();
                        rest = &[];
                    }
                },
                Some(client) => {
                    let client = client.clone();
                    match rest.iter().position(|&b| b == b'\n') {
                        Some(at) => {
                            self.line.extend_from_slice(&rest[..at]);
                            rest = &rest[at + 1..];
                            let line = String::from_utf8_lossy(&self.line)
                                .trim_end_matches('\r')
                                .to_owned();
                            self.line.clear();
                            // `%exit`'s `ESC \` may start the line after it.
                            let line = line.strip_prefix("\x1b\\").unwrap_or(&line).to_owned();
                            client.line(&line);
                            if client.ended() {
                                self.client = None;
                                if let Ok(mut slot) = self.slot.lock() {
                                    *slot = None;
                                }
                                rest = rest.strip_prefix(END).unwrap_or(rest);
                            }
                        }
                        None => {
                            self.line.extend_from_slice(rest);
                            rest = &[];
                        }
                    }
                }
            }
        }
        shown
    }

    /// Control mode began: a client, published to the session, asking for the windows.
    fn start(&mut self) {
        let client = Arc::new(TmuxClient::new(self.writer.clone(), self.notify.clone()));
        if let Ok(mut slot) = self.slot.lock() {
            *slot = Some(client.clone());
        }
        self.client = Some(client.clone());
        self.line.clear();
        (self.notify)(SessionEvent::Tmux(TmuxEvent::Attached));
        client.list_windows();
    }
}

/// Where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// `%3` as 3 (with `prefix` `%`), `@2` as 2 (with `@`).
fn number(text: &str, prefix: char) -> Option<u32> {
    text.strip_prefix(prefix)?.parse().ok()
}

/// A `list-windows` line: id, layout, active pane, whether current, then the name.
fn window_line(line: &str) -> Option<TmuxWindow> {
    let mut parts = line.splitn(5, '\t');
    let id = number(parts.next()?, '@')?;
    let layout = Layout::parse(parts.next()?)?;
    let active_pane = parts.next().and_then(|p| number(p, '%'));
    let active = parts.next() == Some("1");
    let name = parts.next().unwrap_or_default().to_owned();
    Some(TmuxWindow {
        id,
        name,
        layout,
        active_pane,
        active,
    })
}

/// `%output`'s data as bytes: `\ooo` is the byte with that octal value (tmux escapes bytes below
/// space and `\` itself).
fn unescape(data: &str) -> Vec<u8> {
    let bytes = data.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|b| (b'0'..=b'7').contains(b))
        {
            let value = (u32::from(bytes[i + 1] - b'0') << 6)
                | (u32::from(bytes[i + 2] - b'0') << 3)
                | u32::from(bytes[i + 3] - b'0');
            out.push(value as u8);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_layouts() {
        let layout =
            Layout::parse("bb62,159x48,0,0{79x48,0,0,1,79x48,80,0[79x24,80,0,2,79x23,80,25,3]}")
                .unwrap();
        assert_eq!(
            layout.panes(),
            [(1, 0, 0, 79, 48), (2, 80, 0, 79, 24), (3, 80, 25, 79, 23)]
        );
        assert!(matches!(layout.kind, LayoutKind::Row(_)));
        assert_eq!(
            Layout::parse("c0de,80x24,0,0,7").unwrap().panes(),
            [(7, 0, 0, 80, 24)]
        );
        assert!(Layout::parse("x,80x24,0,0{").is_none());
    }

    #[test]
    fn unescapes_output() {
        assert_eq!(unescape(r"hi\015\012a\134b"), b"hi\r\na\\b");
        assert_eq!(unescape(r"end\01"), b"end\\01");
    }

    /// A gate whose commands and events are collected.
    fn gate() -> (Gate, Arc<Mutex<Vec<u8>>>, mpsc::Receiver<SessionEvent>) {
        /// Collects what's written.
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let sent = Arc::new(Mutex::new(Vec::new()));
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(Box::new(Sink(sent.clone()))));
        let (tx, rx) = mpsc::channel();
        let notify: Arc<dyn Fn(SessionEvent) + Send + Sync> = Arc::new(move |e| {
            let _ = tx.send(e);
        });
        (
            Gate::new(writer, notify, Arc::new(Mutex::new(None))),
            sent,
            rx,
        )
    }

    #[test]
    fn a_session_from_attach_to_detach() {
        let (mut gate, sent, events) = gate();
        // The start split across reads, with ordinary output before it.
        assert_eq!(gate.filter(b"$ tmux -CC\r\n\x1bP10"), b"$ tmux -CC\r\n");
        let shown = gate.filter(b"00p%begin 1 1 0\n%end 1 1 0\n%session-changed $0 main\n");
        assert!(String::from_utf8_lossy(&shown).contains("tmux control mode"));
        assert_eq!(
            events.try_recv().unwrap(),
            SessionEvent::Tmux(TmuxEvent::Attached)
        );
        let commands = String::from_utf8(sent.lock().unwrap().clone()).unwrap();
        // The windows asked for twice: on attach, and for `%session-changed`.
        assert_eq!(commands.matches("list-windows").count(), 2);
        let client = gate.slot.lock().unwrap().clone().unwrap();

        // Replies to both, marked as ours.
        let reply = "@1\tb25d,80x24,0,0{40x24,0,0,1,39x24,41,0,2}\t%1\t1\tmain\n";
        gate.filter(format!("%begin 2 2 1\n{reply}%end 2 2 1\n").as_bytes());
        match events.try_recv().unwrap() {
            SessionEvent::Tmux(TmuxEvent::Windows(windows)) => {
                assert_eq!(windows.len(), 1);
                assert_eq!(windows[0].name, "main");
                assert_eq!(windows[0].active_pane, Some(1));
                assert_eq!(windows[0].layout.panes().len(), 2);
            }
            other => panic!("{other:?}"),
        }
        gate.filter(format!("%begin 3 3 1\n{reply}%end 3 3 1\n").as_bytes());
        let _ = events.try_recv();

        // Pane 1's output before its capture is dropped; the capture and what follows show.
        gate.filter(b"%output %1 old\n");
        let commands = String::from_utf8(sent.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("capture-pane -p -e -J -t %1"));
        gate.filter(b"%begin 4 4 1\nline one\nline two\n3 1\n%end 4 4 1\n");
        gate.filter(b"%output %1 new\\015\\012\n");
        let opened = open(&TmuxPane {
            client: client.clone(),
            pane: 1,
        });
        let mut reader = opened.reader;
        let mut first = vec![0u8; 256];
        let n = reader.read(&mut first).unwrap();
        assert_eq!(&first[..n], b"line one\r\nline two\x1b[2;4Hnew\r\n");

        // Typing goes out as hex keys.
        let mut writer = opened.writer;
        writer.write_all(b"ls\r").unwrap();
        let commands = String::from_utf8(sent.lock().unwrap().clone()).unwrap();
        assert!(commands.contains("send-keys -t %1 -H 6c 73 0d"));

        // A layout without pane 2 closes it; detaching ends control mode.
        gate.filter(b"%layout-change @1 c0de,80x24,0,0,1 c0de,80x24,0,0,1 *\n");
        assert!(matches!(
            events.try_recv().unwrap(),
            SessionEvent::Tmux(TmuxEvent::Layout { window: 1, .. })
        ));
        let shown = gate.filter(b"%exit detached\n\x1b\\$ ");
        assert_eq!(shown, b"$ ");
        assert_eq!(
            events.try_recv().unwrap(),
            SessionEvent::Tmux(TmuxEvent::Detached("detached".into()))
        );
        assert!(client.ended());
        assert!(gate.slot.lock().unwrap().is_none());
        // The pane's session sees its end.
        (opened.waiter)();
    }

    use std::io::Read;
}
