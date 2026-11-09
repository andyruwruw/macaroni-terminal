//! What screen readers are told: each window as an AccessKit tree, kept current while a screen
//! reader reads it.
//!
//! - A window's tree: the sidebar (a tree of groups and workspaces, then New workspace), each
//!   shown pane (its tab list, its `+` and new tab buttons, and its open tab's content: a
//!   terminal's visible lines, a page, a Git tab), an announcer (a polite live region), and an
//!   open menu, the settings dialog or the command palette.
//! - Model items keep their model ids as node ids (unique across kinds); other nodes take ids
//!   from the top of the range ([`fixed`]) or a kind in the top byte ([`derived`]).
//! - [`App::sync_reader`] runs after every update: it builds each read window's tree, sends
//!   only the nodes that changed (and the focus) through `window::run`, and remembers what each
//!   node stands for ([`Target`]) for the screen reader's actions ([`App::reader_event`]).
//! - Terminal text and where things are on screen change too often to rebuild on every update;
//!   [`App::reader_tick`] refreshes them a few times a second (`READER_TICK`), and the newest
//!   output of the focused terminal goes to the announcer then.
//! - Focus follows the keyboard: a menu's highlighted item, the palette's highlighted match,
//!   the settings row with the ring, the sidebar or strip stop with the ring, else the focused
//!   pane's content.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use accesskit::{
    Action, Affine, Live, Node, NodeId, Rect, Role, TextPosition, TextSelection, Toggled, TreeId,
    TreeInfo, TreeUpdate,
};
use iced::widget::Id;
use iced::{Rectangle, Task, window};
use macaroni_core::{PaneId, SessionId, TabId, TabKind};
use macaroni_term::{CellFlags, Snapshot};

use super::adapter::{self, Event};
use super::bounds;
use crate::message::Message;
use crate::state::{KeyFocus, MenuTarget};
use crate::update::shortcuts::pane_letter;
use crate::view::settings::{AccessKind, AccessRow};
use crate::{App, MenuPage, view};

/// How often a read window's terminal text, positions and announcements are refreshed.
pub const READER_TICK: Duration = Duration::from_millis(400);

/// After typing in a terminal, how long changes to the cursor's line aren't read out: the
/// screen reader echoes keys itself.
const TYPING_QUIET: Duration = Duration::from_millis(600);

/// The most lines read out at once; a screenful redrawn (an editor, `clear`) isn't read whole.
const MAX_ANNOUNCED_LINES: usize = 12;

/// What a node stands for, to carry out a screen reader's action on it.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A sidebar or strip stop.
    Stop(KeyFocus),
    /// A pane's open tab's content.
    Content(PaneId, TabId),
    /// An item of the open menu: in the submenu or not, and its index in its list.
    MenuItem(bool, usize),
    /// The `n`th settings row the keyboard moves through.
    SettingsRow(usize),
    /// The `n`th match in the command palette.
    PaletteItem(usize),
}

/// What's been sent for one window.
#[derive(Default)]
struct Sent {
    /// Every node, as last sent.
    nodes: HashMap<NodeId, Node>,
    /// What each node stands for.
    targets: HashMap<NodeId, Target>,
    /// The focus last sent.
    focus: Option<NodeId>,
    /// Activated and not yet sent a whole tree.
    fresh: bool,
}

/// A terminal's visible lines as text runs, with the caret, refreshed every tick.
struct TerminalText {
    /// One text run per row.
    runs: Vec<(NodeId, Node)>,
    /// The caret, where the cursor is.
    caret: Option<TextSelection>,
}

/// Everything screen reader support keeps. Lives in `App.reader`.
#[derive(Default)]
pub struct Reader {
    /// The windows a screen reader is reading, and what each was sent.
    windows: HashMap<window::Id, Sent>,
    /// Where each item with a widget id was on screen at the last tick.
    bounds: HashMap<Id, Rectangle>,
    /// Each shown terminal's text, by session, from the last tick.
    text: HashMap<SessionId, TerminalText>,
    /// The focused terminal's lines when its output was last read out.
    heard: Option<(SessionId, Vec<String>)>,
    /// What the announcer says now.
    announcement: String,
    /// When a key was last typed into a terminal.
    pub typed: Option<Instant>,
}

impl Reader {
    /// Whether any window is being read.
    pub fn active(&self) -> bool {
        !self.windows.is_empty()
    }

    /// Forgets a window that closed.
    pub fn forget(&mut self, id: window::Id) {
        self.windows.remove(&id);
    }
}

/// The ids of nodes that aren't model items.
mod fixed {
    use accesskit::NodeId;

    /// The window.
    pub const ROOT: NodeId = NodeId(u64::MAX);
    /// The sidebar's tree of workspaces.
    pub const SIDEBAR: NodeId = NodeId(u64::MAX - 1);
    /// The shown panes.
    pub const PANES: NodeId = NodeId(u64::MAX - 2);
    /// The live region new output is read from.
    pub const ANNOUNCER: NodeId = NodeId(u64::MAX - 3);
    /// The sidebar's New workspace button.
    pub const NEW_WORKSPACE: NodeId = NodeId(u64::MAX - 4);
    /// The open menu's main list.
    pub const MENU: NodeId = NodeId(u64::MAX - 5);
    /// The open menu's submenu.
    pub const SUBMENU: NodeId = NodeId(u64::MAX - 6);
    /// The settings dialog or the command palette.
    pub const DIALOG: NodeId = NodeId(u64::MAX - 7);
    /// The command palette's field.
    pub const FIELD: NodeId = NodeId(u64::MAX - 8);
    /// The command palette's matches.
    pub const LIST: NodeId = NodeId(u64::MAX - 9);
}

/// What a [`derived`] node is, in the top byte of its id.
#[derive(Clone, Copy)]
enum Kind {
    /// A pane's tab list.
    TabList = 1,
    /// A pane's `+`.
    Add = 2,
    /// A pane's new tab button.
    Button = 3,
    /// A pane's open tab's content.
    Content = 4,
    /// A terminal's row.
    Run = 5,
    /// An item of the menu's main list.
    MenuItem = 6,
    /// An item of the submenu.
    SubmenuItem = 7,
    /// A settings row.
    Row = 8,
    /// A palette match.
    Match = 9,
}

/// A node id for something about `id` (a model id, under 2^32 in any real session), `n`
/// (under 2^24) telling several apart.
fn derived(kind: Kind, id: u64, n: usize) -> NodeId {
    NodeId(((kind as u64) << 56) | ((n as u64 & 0xff_ffff) << 32) | (id & 0xffff_ffff))
}

/// The node id of a sidebar or strip stop.
fn stop_id(stop: KeyFocus) -> NodeId {
    match stop {
        KeyFocus::Workspace(ws) => NodeId(ws.0),
        KeyFocus::Group(group) => NodeId(group.0),
        KeyFocus::Tab(tab) => NodeId(tab.0),
        KeyFocus::NewWorkspace => fixed::NEW_WORKSPACE,
        KeyFocus::Add(pane) => derived(Kind::Add, pane.0, 0),
        KeyFocus::Button(pane, n) => derived(Kind::Button, pane.0, n),
    }
}

/// The widget id of a terminal view's container, for where it is.
pub fn terminal_id(session: SessionId) -> Id {
    Id::from(format!("terminal-{}", session.0))
}

/// `r` as AccessKit's rectangle.
fn rect(r: Rectangle) -> Rect {
    Rect {
        x0: f64::from(r.x),
        y0: f64::from(r.y),
        x1: f64::from(r.x + r.width),
        y1: f64::from(r.y + r.height),
    }
}

/// A node with `role`, `label` and `actions`.
fn node(role: Role, label: &str, actions: &[Action]) -> Node {
    let mut node = Node::new(role);
    if !label.is_empty() {
        node.set_label(label);
    }
    for action in actions {
        node.add_action(*action);
    }
    node
}

/// What a window's tree is: its nodes, what they stand for, and the focus.
struct Built {
    /// Every node.
    nodes: Vec<(NodeId, Node)>,
    /// What each node stands for.
    targets: HashMap<NodeId, Target>,
    /// The focused node.
    focus: NodeId,
}

impl Built {
    /// Adds a node standing for `target`.
    fn push(&mut self, id: NodeId, node: Node, target: Option<Target>) {
        self.nodes.push((id, node));
        if let Some(target) = target {
            self.targets.insert(id, target);
        }
    }
}

impl App {
    /// Sends each read window's tree, as far as it changed since last time.
    pub(crate) fn sync_reader(&mut self) -> Task<Message> {
        if !self.reader.active() {
            return Task::none();
        }
        let ids: Vec<window::Id> = self.reader.windows.keys().copied().collect();
        let mut tasks = Vec::new();
        for id in ids {
            if let Some(update) = self.next_update(id) {
                tasks.push(window::run(id, move |_| adapter::update(id, update)).discard());
            }
        }
        Task::batch(tasks)
    }

    /// What window `id`'s screen reader needs to hear now: the nodes that changed since the
    /// last update (all of them, the first time) and the focus; `None` when nothing changed or
    /// the window has gone.
    fn next_update(&mut self, id: window::Id) -> Option<TreeUpdate> {
        if !self.wins.contains_key(&id) {
            self.reader.forget(id);
            return None;
        }
        let built = self.build_tree(id);
        let sent = self.reader.windows.get_mut(&id)?;
        let changed: Vec<(NodeId, Node)> = if sent.fresh {
            built.nodes.clone()
        } else {
            built
                .nodes
                .iter()
                .filter(|(nid, node)| sent.nodes.get(nid) != Some(node))
                .cloned()
                .collect()
        };
        if changed.is_empty() && sent.focus == Some(built.focus) {
            return None;
        }
        let update = TreeUpdate {
            nodes: changed,
            tree: sent.fresh.then(|| {
                let mut info = TreeInfo::new(fixed::ROOT);
                info.toolkit_name = Some("macaroni".to_owned());
                info.toolkit_version = Some(env!("CARGO_PKG_VERSION").to_owned());
                info
            }),
            tree_id: TreeId::ROOT,
            focus: built.focus,
        };
        sent.fresh = false;
        sent.focus = Some(built.focus);
        sent.targets = built.targets;
        sent.nodes = built.nodes.into_iter().collect();
        Some(update)
    }

    /// A screen reader started reading a window, or asked for an action.
    pub(crate) fn reader_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Activated(id) => {
                self.reader.windows.insert(
                    id,
                    Sent {
                        fresh: true,
                        ..Sent::default()
                    },
                );
                self.reader_tick()
            }
            Event::Action(id, request) => {
                let target = self
                    .reader
                    .windows
                    .get(&id)
                    .and_then(|s| s.targets.get(&request.target_node))
                    .cloned();
                match target {
                    Some(target) => self.reader_action(target, request.action),
                    None => Task::none(),
                }
            }
        }
    }

    /// Carries out a screen reader's `action` on what `target` stands for.
    fn reader_action(&mut self, target: Target, action: Action) -> Task<Message> {
        match (target, action) {
            (Target::Stop(stop), Action::Focus) => {
                self.key_focus = Some(stop);
                Task::none()
            }
            (Target::Stop(stop), Action::Click) => self.activate(stop, true),
            (Target::Stop(KeyFocus::Group(group)), Action::Expand | Action::Collapse) => {
                Task::done(Message::Group(
                    group,
                    crate::GroupAction::Collapse(action == Action::Collapse),
                ))
            }
            (Target::Stop(stop), Action::ShowContextMenu) => self.open_focus_menu(stop),
            (Target::Content(pane, tab), Action::Focus | Action::Click) => {
                self.key_focus = None;
                self.apply_all(vec![
                    macaroni_core::Command::FocusPane(pane),
                    macaroni_core::Command::SelectTab(tab),
                ])
            }
            (Target::MenuItem(sub, index), Action::Click) => {
                let Some(menu) = self.menu else {
                    return Task::none();
                };
                let page = match menu.sub {
                    Some(s) if sub => s.page,
                    _ => MenuPage::Main,
                };
                let kind = self
                    .menu_items(menu.target, page)
                    .into_iter()
                    .find(|(i, _)| *i == index)
                    .map(|(_, k)| k);
                match kind {
                    Some(view::MenuEntryKind::Action(message)) => Task::done(message),
                    Some(view::MenuEntryKind::Submenu(page)) => self.menu_pin(page),
                    None => Task::none(),
                }
            }
            (Target::PaletteItem(n), Action::Click) => {
                Task::done(Message::Palette(view::palette::PaletteMessage::Run(n)))
            }
            _ => Task::none(),
        }
    }

    /// Asks where every item of the read windows is now, for [`App::reader_bounds`].
    pub(crate) fn reader_tick(&self) -> Task<Message> {
        if !self.reader.active() {
            return Task::none();
        }
        let mut ids = Vec::new();
        for (&id, win) in &self.wins {
            if !self.reader.windows.contains_key(&id) {
                continue;
            }
            let Some(w) = self.model.window(win.model) else {
                continue;
            };
            self.viewing.set(Some(id));
            ids.extend(self.sidebar_stops().into_iter().map(KeyFocus::widget_id));
            if let Some(ws) = self.model.workspace(w.active) {
                for (pane, _) in ws.shown_panes(macaroni_core::Rect::UNIT) {
                    ids.extend(self.strip_stops(pane).into_iter().map(KeyFocus::widget_id));
                    if let Some(session) = self.pane_session(pane) {
                        ids.push(terminal_id(session));
                    }
                }
            }
            self.viewing.set(None);
        }
        bounds::find_all(ids).map(Message::ReaderBounds)
    }

    /// Where things are now: refreshes the shown terminals' text and reads out new output.
    pub(crate) fn reader_bounds(&mut self, found: HashMap<Id, Rectangle>) -> Task<Message> {
        self.reader.bounds = found;
        let mut sessions = HashSet::new();
        for (&id, win) in &self.wins {
            if !self.reader.windows.contains_key(&id) {
                continue;
            }
            if let Some(ws) = self
                .model
                .window(win.model)
                .and_then(|w| self.model.workspace(w.active))
            {
                sessions.extend(
                    ws.shown_panes(macaroni_core::Rect::UNIT)
                        .into_iter()
                        .filter_map(|(pane, _)| self.pane_session(pane)),
                );
            }
        }
        let text: HashMap<SessionId, TerminalText> = sessions
            .into_iter()
            .filter_map(|s| Some((s, self.terminal_text(s)?)))
            .collect();
        self.reader.text = text;
        self.announce();
        self.sync_reader()
    }

    /// The terminal the open tab of `pane` shows (a tmux window's focused pane), if any.
    fn pane_session(&self, pane: PaneId) -> Option<SessionId> {
        let tab = self.model.tab(self.model.pane(pane)?.active)?;
        match tab.kind {
            TabKind::Terminal(session) => Some(session),
            TabKind::Tmux => self.tmux_focused_pane(tab.id),
            TabKind::Browser(_) | TabKind::Git | TabKind::AgentSetup | TabKind::File => None,
        }
    }

    /// A terminal's visible rows as text runs, placed where the view draws them.
    fn terminal_text(&self, session: SessionId) -> Option<TerminalText> {
        let snapshot = &self.running.get(&session)?.snapshot;
        let at = self.reader.bounds.get(&terminal_id(session)).copied();
        let pad = self.chrome.s.panes.terminal_padding;
        let (cw, ch) = (self.metrics.cell_width, self.metrics.cell_height);
        let mut runs = Vec::with_capacity(snapshot.rows.len());
        let mut caret = None;
        let last = snapshot.rows.len().saturating_sub(1);
        for (r, cells) in snapshot.rows.iter().enumerate() {
            let id = derived(Kind::Run, session.0, r);
            let mut value = String::new();
            let mut lengths = Vec::new();
            let mut positions = Vec::new();
            let mut widths = Vec::new();
            // The character index each column starts, for the caret.
            let mut columns = Vec::with_capacity(cells.len());
            for (col, cell) in cells.iter().enumerate() {
                columns.push(lengths.len());
                if cell.flags.contains(CellFlags::WIDE_SPACER) {
                    continue;
                }
                let wide = cell.flags.contains(CellFlags::WIDE);
                value.push(cell.ch);
                lengths.push(cell.ch.len_utf8() as u8);
                positions.push(col as f32 * cw);
                widths.push(if wide { cw * 2.0 } else { cw });
            }
            // Trailing blanks say nothing, but the caret may sit among them.
            let kept = value.trim_end_matches(' ').chars().count();
            value = value.chars().take(kept).collect();
            lengths.truncate(kept);
            positions.truncate(kept);
            widths.truncate(kept);
            let word_starts = word_starts(&value);
            if r < last {
                value.push('\n');
                lengths.push(1);
                positions.push(kept as f32 * cw);
                widths.push(0.0);
            }
            if let Some(cursor) = snapshot.cursor.filter(|c| c.row == r) {
                let index = columns.get(cursor.col).copied().unwrap_or(kept).min(kept);
                let position = TextPosition {
                    node: id,
                    character_index: index,
                };
                caret = Some(TextSelection {
                    anchor: position,
                    focus: position,
                });
            }
            let mut run = Node::new(Role::TextRun);
            run.set_value(value);
            run.set_character_lengths(lengths);
            run.set_character_positions(positions);
            run.set_character_widths(widths);
            run.set_word_starts(word_starts);
            if let Some(at) = at {
                let y = at.y + pad + r as f32 * ch;
                run.set_bounds(rect(Rectangle::new(
                    iced::Point::new(at.x + pad, y),
                    iced::Size::new(cells.len() as f32 * cw, ch),
                )));
            }
            runs.push((id, run));
        }
        Some(TerminalText { runs, caret })
    }

    /// Reads out what's new in the focused terminal since the last tick, unless
    /// `[accessibility] announce_output` is off.
    fn announce(&mut self) {
        let Some(session) = self.focused_session() else {
            self.reader.heard = None;
            return;
        };
        let Some(running) = self.running.get(&session) else {
            return;
        };
        let lines = screen_lines(&running.snapshot);
        let heard = self.reader.heard.replace((session, lines.clone()));
        // A terminal just focused is read by moving through it, not all at once.
        let Some((_, previous)) = heard.filter(|(s, _)| *s == session) else {
            return;
        };
        if !self.config.accessibility.announce_output {
            return;
        }
        // While typing, the cursor's line changes with every key, and the screen reader
        // already echoes them.
        let quiet = self
            .reader
            .typed
            .is_some_and(|t| t.elapsed() < TYPING_QUIET)
            .then(|| running.snapshot.cursor.map(|c| c.row))
            .flatten();
        let new = new_lines(&previous, &lines, quiet);
        if !new.is_empty() {
            self.reader.announcement = new.join("\n");
        }
    }

    /// Builds window `id`'s tree as it is now.
    fn build_tree(&self, id: window::Id) -> Built {
        // The view's own notion of "the window", so `shown` and the rest answer for this one.
        self.viewing.set(Some(id));
        let built = self.build_tree_here(id);
        self.viewing.set(None);
        built
    }

    /// [`App::build_tree`], with `viewing` set.
    fn build_tree_here(&self, id: window::Id) -> Built {
        let mut b = Built {
            nodes: Vec::new(),
            targets: HashMap::new(),
            focus: fixed::ROOT,
        };
        let focused_window = self.drawing_focused();
        let mut root = node(Role::Window, &self.title(id), &[]);
        root.set_transform(Affine::scale(f64::from(self.scale())));
        let mut children = Vec::new();
        if !self.sidebar.collapsed {
            children.push(fixed::SIDEBAR);
            self.build_sidebar(&mut b);
        }
        children.push(fixed::PANES);
        let content_focus = self.build_panes(&mut b);
        b.focus = content_focus.unwrap_or(fixed::ROOT);
        // The focus has to be a node in the tree: a stop that's gone keeps it on the content.
        if let Some(stop) = self.key_focus.filter(|_| focused_window)
            && b.targets.contains_key(&stop_id(stop))
        {
            b.focus = stop_id(stop);
        }
        let mut announcer = node(Role::Status, &self.reader.announcement, &[]);
        announcer.set_live(Live::Polite);
        b.push(fixed::ANNOUNCER, announcer, None);
        children.push(fixed::ANNOUNCER);
        if focused_window {
            if let Some(focus) = self.build_menu(&mut b) {
                children.push(fixed::MENU);
                b.focus = focus;
            }
            if let Some(focus) = self.build_dialog(&mut b) {
                children.push(fixed::DIALOG);
                b.focus = focus;
            }
        }
        root.set_children(children);
        b.push(fixed::ROOT, root, None);
        b
    }

    /// The sidebar: a tree of group headers holding their workspaces, the ungrouped
    /// workspaces, then New workspace.
    fn build_sidebar(&self, b: &mut Built) {
        let shown = self.shown();
        let mut top = Vec::new();
        let mut group: Option<(NodeId, Node, Vec<NodeId>)> = None;
        let finish = |b: &mut Built, group: Option<(NodeId, Node, Vec<NodeId>)>| {
            if let Some((id, mut node, members)) = group {
                node.set_children(members);
                b.nodes.push((id, node));
            }
        };
        for stop in self.sidebar_stops() {
            let id = stop_id(stop);
            let mut item = match stop {
                KeyFocus::Group(g) => {
                    let Some(g) = self.model.group(g) else {
                        continue;
                    };
                    let mut item = node(
                        Role::TreeItem,
                        &g.name,
                        &[
                            Action::Click,
                            Action::Focus,
                            Action::ShowContextMenu,
                            if g.collapsed {
                                Action::Expand
                            } else {
                                Action::Collapse
                            },
                        ],
                    );
                    item.set_expanded(!g.collapsed);
                    item
                }
                KeyFocus::Workspace(ws) => {
                    let Some(w) = self.model.workspace(ws) else {
                        continue;
                    };
                    let mut item = node(
                        Role::TreeItem,
                        &w.name,
                        &[Action::Click, Action::Focus, Action::ShowContextMenu],
                    );
                    item.set_selected(ws == shown.active);
                    if self.model.workspace_notice(ws).is_some() {
                        item.set_description("Waiting for you");
                    }
                    item
                }
                _ => node(
                    Role::Button,
                    "New workspace",
                    &[Action::Click, Action::Focus],
                ),
            };
            if let Some(at) = self.reader.bounds.get(&stop.widget_id()) {
                item.set_bounds(rect(*at));
            }
            b.targets.insert(id, Target::Stop(stop));
            match stop {
                KeyFocus::Group(_) => {
                    finish(b, group.take());
                    top.push(id);
                    group = Some((id, item, Vec::new()));
                }
                KeyFocus::Workspace(ws)
                    if self.model.workspace(ws).is_some_and(|w| w.group.is_some()) =>
                {
                    if let Some((_, _, members)) = &mut group {
                        members.push(id);
                    } else {
                        top.push(id);
                    }
                    b.nodes.push((id, item));
                }
                _ => {
                    finish(b, group.take());
                    top.push(id);
                    b.nodes.push((id, item));
                }
            }
        }
        finish(b, group.take());
        let mut sidebar = node(Role::Tree, "Workspaces", &[]);
        sidebar.set_children(top);
        b.push(fixed::SIDEBAR, sidebar, None);
    }

    /// The shown panes, each its tab strip and its open tab's content. Returns the focused
    /// pane's content, where the keyboard is otherwise.
    fn build_panes(&self, b: &mut Built) -> Option<NodeId> {
        let ws = self.shown_workspace();
        let panes = ws.shown_panes(macaroni_core::Rect::UNIT);
        let several = panes.len() > 1;
        let mut focus = None;
        let mut pane_ids = Vec::new();
        for (i, (pane_id, _)) in panes.into_iter().enumerate() {
            let Some(pane) = self.model.pane(pane_id) else {
                continue;
            };
            let mut strip = Vec::new();
            for stop in self.strip_stops(pane_id) {
                let id = stop_id(stop);
                let mut item = match stop {
                    KeyFocus::Tab(tab) => {
                        let Some(tab) = self.model.tab(tab) else {
                            continue;
                        };
                        let mut item = node(
                            Role::Tab,
                            &view::tab_title(tab),
                            &[Action::Click, Action::Focus, Action::ShowContextMenu],
                        );
                        item.set_selected(tab.id == pane.active);
                        if tab.attention.is_some() {
                            item.set_description("Waiting for you");
                        }
                        item
                    }
                    KeyFocus::Add(_) => node(
                        Role::Button,
                        "New tab",
                        &[Action::Click, Action::Focus, Action::ShowContextMenu],
                    ),
                    KeyFocus::Button(_, n) => {
                        let Some(button) = self.strip_buttons(pane_id).into_iter().nth(n) else {
                            continue;
                        };
                        let mut item =
                            node(Role::Button, button.name, &[Action::Click, Action::Focus]);
                        if button.message.is_none() {
                            item.set_disabled();
                        }
                        item
                    }
                    _ => continue,
                };
                if let Some(at) = self.reader.bounds.get(&stop.widget_id()) {
                    item.set_bounds(rect(*at));
                }
                b.push(id, item, Some(Target::Stop(stop)));
                strip.push(id);
            }
            let list_id = derived(Kind::TabList, pane_id.0, 0);
            let mut list = node(Role::TabList, "Tabs", &[]);
            list.set_children(strip);
            b.push(list_id, list, None);
            let content = self.build_content(b, pane_id, pane.active);
            if pane_id == ws.focused {
                focus = content;
            }
            let label = match pane_letter(i).filter(|_| several) {
                Some(letter) => format!("Pane {}", letter.to_ascii_uppercase()),
                None => "Pane".to_owned(),
            };
            let mut node = node(Role::Group, &label, &[]);
            node.set_children(std::iter::once(list_id).chain(content).collect::<Vec<_>>());
            b.push(NodeId(pane_id.0), node, None);
            pane_ids.push(NodeId(pane_id.0));
        }
        let mut panes = node(Role::Group, "Panes", &[]);
        panes.set_children(pane_ids);
        b.push(fixed::PANES, panes, None);
        focus
    }

    /// A pane's open tab's content: a terminal's lines, a page, or a Git tab.
    fn build_content(&self, b: &mut Built, pane: PaneId, tab: TabId) -> Option<NodeId> {
        let t = self.model.tab(tab)?;
        let id = derived(Kind::Content, tab.0, 0);
        let title = view::tab_title(t);
        let actions = [Action::Focus, Action::Click];
        let mut content = match t.kind {
            TabKind::Terminal(_) | TabKind::Tmux => {
                let mut content = node(Role::Terminal, &title, &actions);
                if let Some(session) = self.pane_session(pane) {
                    if let Some(at) = self.reader.bounds.get(&terminal_id(session)) {
                        content.set_bounds(rect(*at));
                    }
                    if let Some(text) = self.reader.text.get(&session) {
                        content
                            .set_children(text.runs.iter().map(|(id, _)| *id).collect::<Vec<_>>());
                        if let Some(caret) = text.caret {
                            content.set_text_selection(caret);
                        }
                        b.nodes.extend(text.runs.iter().cloned());
                    }
                }
                content
            }
            TabKind::Browser(_) => node(Role::Document, &format!("Page, {title}"), &actions),
            TabKind::Git => node(Role::Group, &format!("Git, {title}"), &actions),
            TabKind::AgentSetup => node(Role::Group, &title, &actions),
            TabKind::File => node(Role::Document, &format!("File, {title}"), &actions),
        };
        content.set_read_only();
        b.push(id, content, Some(Target::Content(pane, tab)));
        Some(id)
    }

    /// The open menu (this window's, when it's the focused one): its main list and any
    /// submenu. Returns the highlighted item, else the menu, as the focus.
    fn build_menu(&self, b: &mut Built) -> Option<NodeId> {
        let menu = self.menu?;
        let mut focus = fixed::MENU;
        let mut lists = vec![(false, MenuPage::Main, fixed::MENU, Kind::MenuItem)];
        if let Some(sub) = menu.sub {
            lists.push((true, sub.page, fixed::SUBMENU, Kind::SubmenuItem));
        }
        let label = match menu.target {
            MenuTarget::Workspace(_) => "Workspace",
            MenuTarget::Group(_) => "Group",
            MenuTarget::Tab(_) => "Tab",
            MenuTarget::NewTab(_) => "New tab",
            MenuTarget::Link => "Link",
        };
        let mut main_children = Vec::new();
        for (sub, page, list_id, kind) in lists {
            let mut items = Vec::new();
            for (index, label, enabled, opens) in self.menu_labels(menu.target, page) {
                let id = derived(kind, 0, index);
                let mut item = node(Role::MenuItem, &label, &[Action::Click]);
                if !enabled {
                    item.set_disabled();
                }
                if opens {
                    item.set_has_popup(accesskit::HasPopup::Menu);
                    item.set_expanded(
                        menu.sub
                            .is_some_and(|s| Some(s.page) == self.menu_page_of(menu.target, index)),
                    );
                }
                if menu
                    .cursor
                    .is_some_and(|c| c.sub == sub && c.index == index)
                {
                    focus = id;
                }
                b.push(id, item, Some(Target::MenuItem(sub, index)));
                items.push(id);
            }
            if sub {
                let mut list = node(Role::Menu, "Submenu", &[]);
                list.set_children(items);
                b.push(list_id, list, None);
                main_children.push(list_id);
            } else {
                main_children.extend(items);
            }
        }
        let mut list = node(Role::Menu, label, &[]);
        list.set_children(main_children);
        b.push(fixed::MENU, list, None);
        Some(focus)
    }

    /// The submenu the main list's item at `index` opens, if it opens one.
    fn menu_page_of(&self, target: MenuTarget, index: usize) -> Option<MenuPage> {
        self.menu_items(target, MenuPage::Main)
            .into_iter()
            .find_map(|(i, kind)| match kind {
                view::MenuEntryKind::Submenu(page) if i == index => Some(page),
                _ => None,
            })
    }

    /// The command palette or the settings dialog, if one is open. Returns its focus.
    fn build_dialog(&self, b: &mut Built) -> Option<NodeId> {
        if let Some(view::palette::PaletteAccess {
            purpose,
            query,
            matches,
            selected,
        }) = self.palette_access()
        {
            let mut dialog = node(Role::Dialog, purpose, &[]);
            dialog.set_modal();
            let mut field = node(Role::SearchInput, purpose, &[Action::Focus]);
            field.set_value(query);
            let mut options = Vec::new();
            let mut focus = fixed::FIELD;
            for (n, (label, detail)) in matches.into_iter().enumerate() {
                let id = derived(Kind::Match, 0, n);
                let mut option = node(Role::ListBoxOption, &label, &[Action::Click]);
                if !detail.is_empty() {
                    option.set_description(detail);
                }
                option.set_selected(n == selected);
                if n == selected {
                    focus = id;
                }
                b.push(id, option, Some(Target::PaletteItem(n)));
                options.push(id);
            }
            let mut list = node(Role::ListBox, "Matches", &[]);
            list.set_children(options);
            b.push(fixed::FIELD, field, None);
            b.push(fixed::LIST, list, None);
            dialog.set_children(vec![fixed::FIELD, fixed::LIST]);
            b.push(fixed::DIALOG, dialog, None);
            return Some(focus);
        }
        let (title, rows) = self.settings_access()?;
        let mut dialog = node(Role::Dialog, &title, &[]);
        dialog.set_modal();
        let mut focus = fixed::DIALOG;
        let mut children = Vec::new();
        for (n, row) in rows.into_iter().enumerate() {
            let id = derived(Kind::Row, 0, n);
            if row.ringed {
                focus = id;
            }
            b.push(id, settings_row(row), Some(Target::SettingsRow(n)));
            children.push(id);
        }
        dialog.set_children(children);
        b.push(fixed::DIALOG, dialog, None);
        Some(focus)
    }
}

/// A settings row's node.
fn settings_row(row: AccessRow) -> Node {
    let actions = [Action::Focus];
    let mut node = match row.kind {
        AccessKind::Switch(on) => {
            let mut n = node(Role::Switch, &row.label, &actions);
            n.set_toggled(Toggled::from(on));
            n
        }
        AccessKind::Choice(value) => {
            let mut n = node(Role::ComboBox, &row.label, &actions);
            n.set_value(value);
            n
        }
        AccessKind::Number(value, min, max, step, unit) => {
            let mut n = node(Role::SpinButton, &row.label, &actions);
            n.set_numeric_value(f64::from(value));
            n.set_min_numeric_value(f64::from(min));
            n.set_max_numeric_value(f64::from(max));
            n.set_numeric_value_step(f64::from(step));
            n.set_value(format!("{} {unit}", trim_number(value)));
            n
        }
        AccessKind::Text(value) => {
            let mut n = node(Role::TextInput, &row.label, &actions);
            n.set_value(value);
            n
        }
        AccessKind::Button => node(Role::Button, &row.label, &actions),
        AccessKind::Other(value) => {
            let mut n = node(Role::Label, &row.label, &actions);
            if !value.is_empty() {
                n.set_value(value);
            }
            n
        }
    };
    if !row.description.is_empty() {
        node.set_description(row.description);
    }
    node
}

/// `v` without a trailing `.0`.
fn trim_number(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Where each word of `text` starts, as character indices (at most 255, as AccessKit stores
/// them in bytes).
fn word_starts(text: &str) -> Vec<u8> {
    let mut starts = Vec::new();
    let mut previous = ' ';
    for (i, c) in text.chars().enumerate().take(256) {
        if !c.is_whitespace() && previous.is_whitespace() {
            starts.push(i as u8);
        }
        previous = c;
    }
    starts
}

/// A terminal's visible lines, trailing blanks trimmed.
fn screen_lines(snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .rows
        .iter()
        .map(|cells| {
            let line: String = cells
                .iter()
                .filter(|c| !c.flags.contains(CellFlags::WIDE_SPACER))
                .map(|c| c.ch)
                .collect();
            line.trim_end().to_owned()
        })
        .collect()
}

/// What's new on a screen that showed `before` and now shows `after`: the screen may have
/// scrolled by any number of lines, and its old last line may have grown. Lines are read
/// from where the old content ends; blank lines are skipped, and `quiet` (the cursor's row,
/// while typing) is left out. At most [`MAX_ANNOUNCED_LINES`], the newest.
pub(crate) fn new_lines(before: &[String], after: &[String], quiet: Option<usize>) -> Vec<String> {
    let end = |lines: &[String]| {
        lines
            .iter()
            .rposition(|l| !l.is_empty())
            .map_or(0, |i| i + 1)
    };
    let (before, after_all) = (&before[..end(before)], after);
    let after = &after_all[..end(after_all)];
    if before == after {
        return Vec::new();
    }
    // How far the screen scrolled: the least shift that lines up everything old but its last
    // line (which may have grown) with the top of the new screen.
    let n = before.len();
    let shift = (0..=n).find(|&s| {
        let kept = &before[s..];
        let fixed = kept.len().saturating_sub(1);
        fixed <= after.len() && kept[..fixed] == after[..fixed]
    });
    let mut out = Vec::new();
    let start = match shift {
        Some(s) if s < n => {
            let at = n - s - 1;
            let (old, new) = (&before[n - 1], after.get(at));
            if let Some(new) = new.filter(|new| *new != old) {
                let grown = new.strip_prefix(old.as_str()).unwrap_or(new);
                if quiet != Some(at) && !grown.trim().is_empty() {
                    out.push(grown.trim().to_owned());
                }
            }
            at + 1
        }
        // Nothing lines up: a new screen.
        _ => 0,
    };
    out.extend(
        after
            .iter()
            .enumerate()
            .skip(start)
            .filter(|(i, l)| quiet != Some(*i) && !l.trim().is_empty())
            .map(|(_, l)| l.trim().to_owned()),
    );
    let skip = out.len().saturating_sub(MAX_ANNOUNCED_LINES);
    out.split_off(skip)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use accesskit_consumer::{NodeRef, Tree, TreeChangeHandler as ChangeHandler};
    use macaroni_core::{Command, Model};

    use super::*;
    use crate::keymap::Action as KeyAction;
    use crate::view::palette::PaletteMessage;

    /// Ignores what changed; applying an update is what checks it.
    struct Ignore;

    impl ChangeHandler for Ignore {
        fn node_added(&mut self, _: &NodeRef) {}
        fn node_updated(&mut self, _: &NodeRef, _: &NodeRef) {}
        fn focus_moved(&mut self, _: Option<&NodeRef>, _: Option<&NodeRef>) {}
        fn node_removed(&mut self, _: &NodeRef) {}
    }

    /// An app with one window that a screen reader reads.
    fn read_app() -> (App, window::Id) {
        let (model, _) = Model::new();
        let mut app = App::with_defaults(model, BTreeMap::new(), None, PathBuf::from("."));
        let id = window::Id::unique();
        app.wins.insert(
            id,
            crate::Win {
                model: app.model.focused_window(),
                size: iced::Size::new(1200.0, 800.0),
                position: None,
                maximized: false,
                parent: None,
                scale: 1.0,
                focused: true,
            },
        );
        let _ = app.reader_event(Event::Activated(id));
        (app, id)
    }

    #[test]
    fn every_update_makes_a_valid_tree() {
        let (mut app, id) = read_app();
        let mut tree = Tree::new(app.next_update(id).expect("a first tree"), true);
        let mut step = |app: &mut App| {
            if let Some(update) = app.next_update(id) {
                tree.update_and_process_changes(update, &mut Ignore);
            }
        };
        // Splits, more tabs and workspaces, a group, then closing some again.
        for command in [
            Command::SplitPane(macaroni_core::Direction::Right),
            Command::NewTab,
            Command::NewWorkspace,
            Command::SplitPane(macaroni_core::Direction::Down),
            Command::NextWorkspace,
        ] {
            let _ = app.apply(command);
            step(&mut app);
        }
        let ws = app.model.active_workspace().id;
        let _ = app.apply(Command::NewGroup(ws));
        step(&mut app);
        // The keyboard on the sidebar and the strip, then a menu from it.
        let _ = app.cycle_region(true);
        step(&mut app);
        let _ = app.cycle_region(true);
        step(&mut app);
        let tab = app.model.focused_tab().id;
        let _ = app.key_menu(MenuTarget::Tab(tab), None);
        step(&mut app);
        let _ = app.focus_key(
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown),
            iced::keyboard::key::Physical::Code(iced::keyboard::key::Code::ArrowDown),
            iced::keyboard::Modifiers::empty(),
        );
        step(&mut app);
        app.menu = None;
        app.key_focus = None;
        step(&mut app);
        // The palette, then the settings dialog.
        let _ = app.palette_update(PaletteMessage::Open);
        step(&mut app);
        let _ = app.palette_update(PaletteMessage::Move(1));
        step(&mut app);
        app.command_palette = None;
        let _ = app.perform(KeyAction::OpenSettings);
        step(&mut app);
        app.settings = None;
        let _ = app.apply(Command::CloseActiveTab);
        step(&mut app);
        let _ = app.apply(Command::CloseActiveWorkspace);
        step(&mut app);
    }

    #[test]
    fn a_stop_is_focused_while_the_keyboard_is_on_it() {
        let (mut app, id) = read_app();
        let _ = app.cycle_region(true);
        let built = app.build_tree(id);
        let tab = app.model.focused_tab().id;
        assert_eq!(built.focus, NodeId(tab.0));
        assert_eq!(
            built.targets.get(&built.focus),
            Some(&Target::Stop(KeyFocus::Tab(tab)))
        );
    }

    fn lines(s: &[&str]) -> Vec<String> {
        s.iter().map(|l| (*l).to_owned()).collect()
    }

    #[test]
    fn output_below_the_prompt_is_new() {
        let before = lines(&["$ ls", ""]);
        let after = lines(&["$ ls", "a.txt  b.txt", "$"]);
        assert_eq!(new_lines(&before, &after, None), ["a.txt  b.txt", "$"]);
    }

    #[test]
    fn a_scrolled_screen_reads_only_what_came_in() {
        let before = lines(&["one", "two", "three"]);
        let after = lines(&["two", "three", "four"]);
        assert_eq!(new_lines(&before, &after, None), ["four"]);
    }

    #[test]
    fn a_grown_line_reads_its_new_end() {
        let before = lines(&["Building"]);
        let after = lines(&["Building... done"]);
        assert_eq!(new_lines(&before, &after, None), ["... done"]);
    }

    #[test]
    fn the_typing_line_stays_quiet() {
        let before = lines(&["$ ec"]);
        let after = lines(&["$ echo"]);
        assert!(new_lines(&before, &after, Some(0)).is_empty());
        assert_eq!(new_lines(&before, &after, None), ["ho"]);
    }

    #[test]
    fn a_redrawn_screen_reads_its_last_lines() {
        let before = lines(&["a", "b"]);
        let after: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
        let heard = new_lines(&before, &after, None);
        assert_eq!(heard.len(), MAX_ANNOUNCED_LINES);
        assert_eq!(heard.last().map(String::as_str), Some("line 29"));
    }

    #[test]
    fn nothing_new_is_silent() {
        let same = lines(&["$ ls", "a"]);
        assert!(new_lines(&same, &same, None).is_empty());
    }
}
