//! [`Model`], the [`Command`]s that change it and the [`Effect`]s it asks the app to perform.
//!
//! This file holds the data types (workspaces, groups, panes, tabs, agent state, saved-layout
//! specs), the read-only queries the app renders from, saving and restoring layouts
//! ([`Model::spec`] / [`Model::restore`]) and [`Model::apply`], which dispatches each command to
//! a method in one of the submodules:
//!
//! - `workspaces`: selecting, renaming, pinning and ordering workspaces, and the sidebar as
//!   "blocks" (a group's workspaces, or one ungrouped workspace) that move as a unit.
//! - `groups`: creating groups and moving workspaces into and out of them.
//! - `panes`: splitting, closing, focusing and resizing panes.
//! - `tabs`: creating, ordering, pinning, moving and closing tabs.
//! - `events`: what running programs report (titles, directories, notices, agents, pages).
//! - `windows`: opening, closing and focusing windows, routing a command to the window holding
//!   what it names, and moving workspaces and tabs between windows.
//! - `check`: the invariants [`Model::check`] verifies after every command in debug builds.

mod check;
mod events;
mod groups;
mod panes;
mod tabs;
mod windows;
mod workspaces;

use std::collections::BTreeMap;

use crate::ids::{BrowserId, GroupId, PaneId, SessionId, TabId, WindowId, WorkspaceId};
use crate::layout::{Axis, Direction, Divider, MIN_RATIO, Rect, Side, SplitNode};
use events::non_blank;

/// One sidebar entry: a named split layout of panes with one of them focused.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    /// Stable for the workspace's life; never reused.
    pub id: WorkspaceId,
    /// Shown in the sidebar. Never blank: new workspaces get "Workspace N" and blank renames
    /// are ignored.
    pub name: String,
    /// The split tree; every leaf is a pane in [`Model`]'s pane map.
    pub layout: SplitNode,
    /// The pane keyboard input goes to; always a leaf of `layout`.
    pub focused: PaneId,
    /// Notifications from every tab in a muted workspace are dropped.
    pub muted: bool,
    /// Pinned workspaces come first in the sidebar.
    pub pinned: bool,
    /// The user's choice of icon, by name; the app maps names it doesn't know to its default.
    pub icon: Option<String>,
    /// The user's choice of color, by name or `#rrggbb`; the model doesn't interpret it.
    pub color: Option<String>,
    /// The group the workspace is shown under, if any. Always a group in [`Model::groups`].
    pub group: Option<GroupId>,
    /// The workspace's folder: where its new terminals start. `None` leaves it to the app's
    /// default (`[terminal] default_dir`, else the home directory).
    /// The model doesn't check it exists; the app does when it spawns.
    pub cwd: Option<String>,
    /// Environment variables every terminal started in the workspace gets, in order.
    pub env: Vec<(String, String)>,
    /// Typed into each new shell in the workspace as it starts (not SSH tabs); the app decides
    /// which terminals count.
    pub startup: Option<String>,
    /// The pane filling the whole workspace while the others are hidden
    /// ([`Command::ToggleMaximize`]). Always the focused pane: focusing another pane, a split or
    /// a close ends it. Not saved with the session.
    pub maximized: Option<PaneId>,
}

impl Workspace {
    /// The panes on screen in `area` and where: the maximized pane alone over all of it, else
    /// every pane of the layout.
    pub fn shown_panes(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        match self.maximized {
            Some(pane) => vec![(pane, area)],
            None => self.layout.panes(area),
        }
    }

    /// The split dividers on screen in `area`: none while a pane is maximized.
    pub fn shown_dividers(&self, area: Rect) -> Vec<Divider> {
        if self.maximized.is_some() {
            Vec::new()
        } else {
            self.layout.dividers(area)
        }
    }
}

/// Workspaces shown together under a collapsible header. A group's workspaces are always next
/// to each other in the sidebar, and are all pinned or all not.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Stable for the group's life; never reused.
    pub id: GroupId,
    /// The header's text; new groups get "Group N" and blank renames are ignored.
    pub name: String,
    /// Whether the sidebar hides the group's workspaces under its header. Only the app reads it.
    pub collapsed: bool,
    /// Like a workspace's color: a name the app knows, or `#rrggbb`.
    pub color: Option<String>,
    /// Like a workspace's icon, by name; `None` shows none.
    pub icon: Option<String>,
}

/// A step for [`Command::ShiftWorkspace`] and [`Command::ShiftGroup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shift {
    /// One place earlier; a no-op at the start.
    Up,
    /// One place later; a no-op at the end.
    Down,
    /// To the start.
    Top,
    /// To the end.
    Bottom,
}

/// A leaf of a workspace's layout: a strip of tabs with one of them shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    /// Stable for the pane's life; never reused.
    pub id: PaneId,
    /// Never empty: closing a pane's last tab closes the pane. Pinned tabs come first.
    pub tabs: Vec<TabId>,
    /// The tab on screen; always one of `tabs`.
    pub active: TabId,
}

/// What a tab shows, and the id of the resource the app keeps for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabKind {
    /// A terminal running the session with this id.
    Terminal(SessionId),
    /// A browser page with this id; its address is [`Tab::url`].
    Browser(BrowserId),
}

/// A terminal or browser in a pane, with what its program and the user have set on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    /// Stable for the tab's life, including across [`Command::MoveTab`]; never reused.
    pub id: TabId,
    /// Set by the program running in the tab (OSC 0/2); `None` shows a default.
    pub title: Option<String>,
    /// The user's name for the tab, shown instead of `title`.
    pub name: Option<String>,
    /// Terminal or browser, fixed at creation.
    pub kind: TabKind,
    /// The latest notification a program in this tab raised, until the user interacts with it.
    pub attention: Option<Notice>,
    /// Text a trigger set, shown in place of the tab's usual badge until the user interacts
    /// with it (cleared along with `attention`).
    pub badge: Option<String>,
    /// A coding agent (Claude Code) running in this tab, as reported by its hooks.
    pub agent: Option<Agent>,
    /// The shell's working directory, when it reports one (OSC 7).
    pub cwd: Option<String>,
    /// A browser tab's current address.
    pub url: Option<String>,
    /// Notifications from a muted tab are dropped.
    pub muted: bool,
    /// Pinned tabs come first in their pane.
    pub pinned: bool,
    /// The user's icon for the tab, by name (as for workspaces).
    pub icon: Option<String>,
    /// The user's color for the tab, by name or `#rrggbb` (as for workspaces).
    pub color: Option<String>,
}

impl Tab {
    /// The user's name for the tab, else the program's title. `None` means the app shows its
    /// default label. Use this anywhere a tab is shown.
    pub fn label(&self) -> Option<&str> {
        self.name.as_deref().or(self.title.as_deref())
    }
}

/// Where [`Command::MoveTab`] takes a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveTo {
    /// The focused pane of another workspace, after its active tab. Ignored for the tab's own
    /// workspace.
    Workspace(WorkspaceId),
    /// A new workspace (appended and made active) holding just this tab. Ignored when the tab
    /// is already alone in its workspace.
    NewWorkspace,
    /// Into a pane, at a position among its tabs (moved as needed to keep pinned tabs first).
    /// In the tab's own pane this reorders it.
    Pane {
        /// The target pane, in any workspace.
        pane: PaneId,
        /// Position among the pane's tabs before the move. In the tab's own pane it counts the
        /// tab itself, so dropping a tab just after itself leaves it in place. Clamped.
        index: usize,
    },
    /// A new pane on the given side of `pane`, which may be the tab's own.
    SplitPane {
        /// The pane to split, in any workspace. Ignored when it's the tab's own pane and the
        /// tab is its only tab.
        pane: PaneId,
        /// Which side of `pane` the new pane goes on.
        direction: Direction,
    },
    /// A new window, in a new workspace there. Ignored when the tab is alone in the only
    /// workspace of its window.
    NewWindow,
    /// A new pane along an edge of a whole workspace, as tall or wide as the workspace.
    SplitWorkspace {
        /// The workspace whose whole layout is wrapped in a 50/50 split with the new pane; it
        /// becomes active. Ignored when it's the tab's own workspace and the tab is alone there.
        workspace: WorkspaceId,
        /// Which edge the new pane goes along.
        direction: Direction,
    },
}

/// A tab in a saved layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabSpec {
    /// A terminal; restoring spawns a fresh session (the app may resume an agent in it).
    Terminal,
    /// A browser at this address (empty if it had none).
    Browser(String),
}

/// A saved layout: what [`Model::spec`] produces and [`Model::restore`] rebuilds.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutSpec {
    /// A pane and its tabs, mirroring [`SplitNode::Leaf`].
    Pane {
        /// The pane's tabs in order. Restoring an empty list gives one terminal.
        tabs: Vec<TabSpec>,
        /// Index of the active tab in `tabs`; clamped on restore.
        active: usize,
    },
    /// A split, mirroring [`SplitNode::Split`].
    Split {
        /// Which way the area is divided.
        axis: Axis,
        /// Share given to `first`. On restore, clamped to `MIN_RATIO..=1 - MIN_RATIO`, and a
        /// non-finite value becomes 0.5.
        ratio: f32,
        /// The left (row) or top (column) subtree.
        first: Box<LayoutSpec>,
        /// The right (row) or bottom (column) subtree.
        second: Box<LayoutSpec>,
    },
}

/// A saved workspace: what [`Model::spec`] produces and [`Model::restore`] rebuilds. Only the
/// name, layout and focus are here; the app saves and reapplies the rest (pinning, muting,
/// icons, colors, groups) itself.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSpec {
    /// The workspace's name; a blank one is replaced by "Workspace N" on restore.
    pub name: String,
    /// The split tree with each pane's tabs.
    pub layout: LayoutSpec,
    /// Index of the focused pane, counting panes left to right / top to bottom; clamped on
    /// restore.
    pub focused: usize,
    /// Which window it's in, counting windows from 0; restore puts it in the last one past
    /// the end.
    pub window: usize,
}

/// A whole model as plain data ([`Model::spec`], [`Model::restore`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// Every workspace, each window's in its sidebar order, windows in order.
    pub workspaces: Vec<WorkspaceSpec>,
    /// For each window, the index in `workspaces` of its active workspace.
    pub windows: Vec<usize>,
    /// Which window has focus.
    pub focused: usize,
}

/// One window's sidebar, for drawing it ([`Model::window`]).
#[derive(Debug, Clone, Copy)]
pub struct WindowView<'a> {
    /// The window.
    pub id: WindowId,
    /// Its workspaces in sidebar order.
    pub workspaces: &'a [Workspace],
    /// Its groups.
    pub groups: &'a [Group],
    /// Its active workspace.
    pub active: WorkspaceId,
}

/// A window's sidebar while another window has focus ([`Model::parked`]).
#[derive(Debug, Clone)]
struct Parked {
    /// Its workspaces in sidebar order.
    workspaces: Vec<Workspace>,
    /// Its groups.
    groups: Vec<Group>,
    /// Its active workspace.
    active: WorkspaceId,
}

/// What a coding agent in a tab is doing, as its hooks report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// Running a turn.
    Working,
    /// Waiting on the user, e.g. for a permission prompt.
    NeedsInput,
    /// Started, or finished its turn, and waiting for the next prompt.
    Idle,
}

/// A coding agent (Claude Code) running in a tab. Set by [`Command::SetAgent`], cleared by
/// [`Command::ClearAgent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// Its latest reported state.
    pub state: AgentState,
    /// What it's doing right now, e.g. "Running cargo test".
    pub detail: Option<String>,
    /// The agent's own session id, used to resume it after a restart.
    pub session_id: Option<String>,
    /// The directory the agent runs in, used to resume it there.
    pub cwd: Option<String>,
    /// A one-line summary from the agent's status line, e.g. "Opus · 45% context".
    pub summary: Option<String>,
}

/// A program's request for attention (an OSC 9/777/99 notification or an agent hook), held on
/// its tab until the user interacts with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// The notification's title, if it had one.
    pub title: Option<String>,
    /// The notification's text.
    pub body: String,
    /// Increases with every notice, so the newest one in a workspace can be found.
    pub seq: u64,
}

/// Everything that can change the [`Model`], passed to [`Model::apply`].
///
/// Commands that name something missing (a closed workspace, a tab id from an old message) are
/// ignored rather than failing, so input that raced a close is harmless. Only commands that
/// create or close tabs produce effects; everything else is pure bookkeeping.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Opens a window with one workspace holding one new terminal, and focuses it. Emits
    /// [`Effect::OpenWindow`] then [`Effect::SpawnSession`].
    NewWindow,
    /// Closes the window and everything in it, as closing each of its workspaces would; the
    /// last window quits instead (see [`Command::CloseWorkspace`]). Emits
    /// [`Effect::CloseWindow`].
    CloseWindow(WindowId),
    /// The window got focus: commands without a target now act on its sidebar. Ignored for a
    /// missing window.
    FocusWindow(WindowId),
    /// Moves a workspace, running, to another window's sidebar (`Some`) or a new window
    /// (`None`), out of any group, where it becomes active and the window focused. A window it
    /// leaves empty closes ([`Effect::CloseWindow`]). Ignored for its own window, a missing one,
    /// or a new window when it's alone in its own.
    MoveWorkspaceToWindow {
        /// The workspace to move.
        workspace: WorkspaceId,
        /// Where to, or `None` for a new window ([`Effect::OpenWindow`]).
        window: Option<WindowId>,
    },
    /// Appends a workspace named "Workspace N" holding one new terminal, and makes it active.
    /// Emits [`Effect::SpawnSession`].
    NewWorkspace,
    /// Closes the workspace, killing every session and closing every browser in it; the
    /// workspace after it (or the new last one) becomes active if it was. Closing the last
    /// workspace instead leaves the model unchanged and emits a kill or close for every tab
    /// plus [`Effect::Quit`].
    CloseWorkspace(WorkspaceId),
    /// [`Command::CloseWorkspace`] on the active workspace.
    CloseActiveWorkspace,
    /// Makes the workspace active.
    SelectWorkspace(WorkspaceId),
    /// Makes the workspace at this zero-based sidebar position active; ignored past the end.
    SelectWorkspaceIndex(usize),
    /// Activates the next workspace in sidebar order, wrapping from the last to the first.
    NextWorkspace,
    /// Activates the previous workspace in sidebar order, wrapping from the first to the last.
    PreviousWorkspace,
    /// Renames the workspace to the trimmed name; a blank name is ignored.
    RenameWorkspace(WorkspaceId, String),
    /// To a position in the sidebar, kept within the pinned or the unpinned part. A grouped
    /// workspace moves only within its group (the index is relative to the whole list and
    /// clamped to the group); an ungrouped one goes before the first block starting at or after
    /// the index, so it never splits a group. Used by tests and scripts; the sidebar uses
    /// [`Command::PlaceWorkspace`].
    MoveWorkspace(WorkspaceId, usize),
    /// Mutes or unmutes every tab in the workspace (see [`Model::is_muted`]). Muting also clears
    /// the attention of every tab in it.
    SetWorkspaceMuted(WorkspaceId, bool),
    /// Pins or unpins the workspace, moving it to the end of the pinned workspaces or the start
    /// of the unpinned ones. Pinning a grouped workspace pins (and moves) its whole group. A
    /// no-op when the flag doesn't change.
    SetWorkspacePinned(WorkspaceId, bool),
    /// Sets or clears the workspace's icon name.
    SetWorkspaceIcon(WorkspaceId, Option<String>),
    /// Sets or clears the workspace's color (a name or `#rrggbb`).
    SetWorkspaceColor(WorkspaceId, Option<String>),
    /// Sets or clears (back to the home directory) the workspace's folder; a blank path clears
    /// it.
    SetWorkspaceCwd(WorkspaceId, Option<String>),
    /// Sets the environment variables the workspace's new terminals get (names the app has
    /// checked); entries with a blank name are dropped.
    SetWorkspaceEnv(WorkspaceId, Vec<(String, String)>),
    /// Sets or clears the command typed into the workspace's new shells; blank clears it.
    SetWorkspaceStartup(WorkspaceId, Option<String>),
    /// Moves every pane of `from` into `into`, beside its own panes (a 50/50 split, `from`'s on
    /// the right), and removes `from` from the sidebar; `into` becomes active with `from`'s
    /// focused pane focused. Sessions and browsers keep running. Ignored when they're the same
    /// workspace or either is missing.
    CombineWorkspaces {
        /// The workspace that stays.
        into: WorkspaceId,
        /// The workspace whose panes move, and which goes away.
        from: WorkspaceId,
    },
    /// Moves a workspace a step within its group, or, outside groups, past the next workspace
    /// or whole group. Never crosses the pinned/unpinned line.
    ShiftWorkspace(WorkspaceId, Shift),
    /// A new group holding just this workspace (taken out of any group it was in, landing just
    /// after it), named "Group N".
    NewGroup(WorkspaceId),
    /// Into the end of a group, taking the group's pinned state, or with `None` out of its group
    /// to just after it. Ignored when the workspace is already there or the group is missing.
    SetWorkspaceGroup(WorkspaceId, Option<GroupId>),
    /// Into `group` (or, with `None`, out of any), just before `before`: at the end of the
    /// group, or of the list, when `before` is `None` or not in it. Ungrouped, it can't split
    /// a group, so before a group's later member means just after that group. Kept within the
    /// pinned or the unpinned part. This is what sidebar drags apply.
    ///
    /// Ignored when `before` is the workspace itself, `group` is missing, or the workspace is
    /// the only member of `group` (taking it out first would leave nothing to place it into).
    PlaceWorkspace {
        /// The workspace being placed.
        id: WorkspaceId,
        /// The workspace it should end up just before, if any.
        before: Option<WorkspaceId>,
        /// The group it should end up in, or `None` for none.
        group: Option<GroupId>,
    },
    /// Renames the group to the trimmed name; a blank name is ignored.
    RenameGroup(GroupId, String),
    /// Collapses or expands the group's header in the sidebar.
    SetGroupCollapsed(GroupId, bool),
    /// Sets or clears the group's color (a name or `#rrggbb`).
    SetGroupColor(GroupId, Option<String>),
    /// Sets or clears the group's icon name.
    SetGroupIcon(GroupId, Option<String>),
    /// Pins or unpins every workspace in the group, moving the group to the end of the pinned
    /// part or the start of the unpinned one.
    SetGroupPinned(GroupId, bool),
    /// Moves the whole group a step among the blocks (groups and ungrouped workspaces) with
    /// the same pinned state.
    ShiftGroup(GroupId, Shift),
    /// Moves every workspace of `from` to the end of `into`, in order, taking `into`'s pinned
    /// state; `from`, left empty, is dropped. Ignored when they're the same group or either is
    /// missing.
    MergeGroups {
        /// The group that stays.
        into: GroupId,
        /// The group whose workspaces move, and which goes away.
        from: GroupId,
    },
    /// Dissolves the group; its workspaces stay where they are.
    Ungroup(GroupId),

    /// Splits the focused pane 50/50; the new pane (with a new terminal) goes on the given side
    /// and takes focus. Emits [`Effect::SpawnSession`].
    SplitPane(Direction),
    /// Closes the focused pane and its tabs, focusing the nearest leaf of its sibling. Closing a
    /// workspace's only pane closes the workspace (and the last workspace quits; see
    /// [`Command::CloseWorkspace`]).
    ClosePane,
    /// Focuses the pane and makes its workspace active.
    FocusPane(PaneId),
    /// Focuses the nearest pane past the focused pane's edge in the active workspace (see
    /// [`crate::neighbour`]); ignored when there's none.
    FocusDirection(Direction),
    /// Makes the active workspace's focused pane fill the workspace, hiding the others, or
    /// brings the layout back. Ignored with a single pane.
    ToggleMaximize,
    /// Gives the active workspace's panes equal shares: panes side by side along one axis get
    /// the same width (or height).
    EqualizePanes,
    /// Swaps the focused pane with its neighbour in a direction (as [`Command::FocusDirection`]
    /// finds it); the pane keeps focus in its new place. Ignored when there's none.
    SwapPane(Direction),
    /// Moves every pane of the active workspace one place along in leaf order (left to right,
    /// top to bottom), the last to the first place; the focused pane keeps focus. Ignored with a
    /// single pane.
    RotatePanes,
    /// Shows the next tab waiting for the user, in the order their notices arrived: the one
    /// after the focused tab when that's waiting too, else the oldest, wrapping around. Like any
    /// selection it leaves the notice in place. Ignored when nothing is waiting.
    NextWaiting,
    /// Sets the ratio of the split at `path` in `workspace`, clamped to
    /// `MIN_RATIO..=1 - MIN_RATIO`. Ignored when `path` doesn't lead to a split or `ratio` isn't
    /// finite. Sent repeatedly while a divider is dragged.
    SetSplitRatio {
        /// The workspace whose layout holds the split.
        workspace: WorkspaceId,
        /// The split's [`crate::Divider::path`] from the layout's root.
        path: Vec<Side>,
        /// The new share for the split's first child.
        ratio: f32,
    },

    /// Adds a terminal tab to the focused pane, after its active tab (or after the pinned tabs),
    /// and activates it. Emits [`Effect::SpawnSession`].
    NewTab,
    /// Opens a browser tab in the focused pane, or in a new pane on the given side of it.
    /// Emits [`Effect::OpenBrowser`]; no terminal is spawned for a split.
    OpenBrowser {
        /// The page to load; stored as the tab's [`Tab::url`].
        url: String,
        /// `None` adds the tab to the focused pane; a direction makes a new, focused pane
        /// holding only the browser.
        split: Option<Direction>,
    },
    /// Opens a browser tab right after `beside` in its pane, e.g. for a page's popup window.
    /// Ignored (no effect emitted) when `beside` doesn't exist.
    OpenBrowserBeside {
        /// The tab the new one goes after; it needn't be a browser.
        beside: TabId,
        /// The page to load.
        url: String,
    },
    /// Closes the tab, emitting [`Effect::KillSession`] or [`Effect::CloseBrowser`]. If it was
    /// active, the tab that took its place (or the new last one) becomes active. Closing a
    /// pane's only tab closes the pane as [`Command::ClosePane`] does.
    CloseTab(TabId),
    /// [`Command::CloseTab`] on the focused pane's active tab.
    CloseActiveTab,
    /// Sets the user's name for the tab, trimmed. `None` or a blank name goes back to the
    /// program's title.
    RenameTab(TabId, Option<String>),
    /// Mutes or unmutes the tab; muting clears its attention, and [`Command::Notify`] on a muted
    /// tab does nothing.
    SetTabMuted(TabId, bool),
    /// Pins or unpins the tab, moving it to the end of its pane's pinned tabs or the start of
    /// the rest. The active tab doesn't change. A no-op when the flag doesn't change, which
    /// restoring a session relies on.
    SetTabPinned(TabId, bool),
    /// Sets or clears the tab's icon name.
    SetTabIcon(TabId, Option<String>),
    /// Sets or clears the tab's color (a name or `#rrggbb`).
    SetTabColor(TabId, Option<String>),
    /// Moves a tab, still running, to another pane; it becomes focused there. Emits nothing: the
    /// session or browser keeps running. A pane left empty is removed, and so is its workspace
    /// if that was its only pane. Pointless moves are ignored (see [`MoveTo`]).
    MoveTab {
        /// The tab to move.
        tab: TabId,
        /// Where it goes.
        to: MoveTo,
    },
    /// Activates the tab and focuses its pane and workspace.
    SelectTab(TabId),
    /// Activates the next tab in the focused pane, wrapping around.
    NextTab,
    /// Activates the previous tab in the focused pane, wrapping around.
    PreviousTab,

    /// The program in the session set its title (OSC 0/2); a blank title clears it.
    SetTabTitle(SessionId, Option<String>),
    /// A program asked for attention; marks its tab until [`Command::ClearAttention`].
    /// Replaces any earlier notice on the tab and takes a fresh [`Notice::seq`]. Ignored when
    /// the tab or its workspace is muted, or no tab runs the session.
    Notify {
        /// The session whose tab is marked.
        session: SessionId,
        /// The notification's title, if any.
        title: Option<String>,
        /// The notification's text.
        body: String,
    },
    /// Sets the badge of the session's tab ([`Tab::badge`]); a blank one clears it. Ignored
    /// when the tab or its workspace is muted.
    SetTabBadge(SessionId, Option<String>),
    /// The user typed into or clicked the tab: drops its notice and its trigger badge.
    ClearAttention(TabId),
    /// An agent in the tab reported its state. `session_id` and `cwd` are kept when `None`.
    /// Creates [`Tab::agent`] if the tab had none. `detail` is replaced every time (a blank one
    /// clears it).
    SetAgent {
        /// The tab the agent runs in.
        tab: TabId,
        /// Its new state.
        state: AgentState,
        /// What it's doing now; replaces the previous detail.
        detail: Option<String>,
        /// The agent's own session id, kept from earlier reports when `None`.
        session_id: Option<String>,
        /// The agent's working directory, kept from earlier reports when `None`.
        cwd: Option<String>,
    },
    /// Sets the status-line summary of the agent in the tab; a blank one clears it. Ignored when
    /// no agent is running there (it doesn't create one).
    SetAgentSummary {
        /// The tab the agent runs in.
        tab: TabId,
        /// The new summary.
        summary: Option<String>,
    },
    /// The agent in the tab exited: drops [`Tab::agent`].
    ClearAgent(TabId),
    /// The shell in the session reported its directory (OSC 7); a blank one clears it.
    SetTabCwd(SessionId, Option<String>),
    /// The session's process exited: closes its tab as [`Command::CloseTab`] does, which emits
    /// [`Effect::KillSession`] so the app drops the dead `Session`. Ignored when no tab runs it.
    SessionExited(SessionId),
    /// The browser navigated: records its address as [`Tab::url`].
    SetBrowserUrl(BrowserId, String),
    /// The page's title changed; a blank one clears it.
    SetBrowserTitle(BrowserId, Option<String>),
    /// The page closed itself (e.g. `window.close()`): closes its tab as [`Command::CloseTab`]
    /// does. Ignored when no tab shows it.
    BrowserClosed(BrowserId),
}

/// A side effect [`Model::apply`] asks the app to perform; the model never does I/O itself.
///
/// Every id in a spawn or open is new and never reused, and each one is matched by exactly one
/// kill or close when its tab goes away (or on quit), which the randomized test checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A terminal tab was created: start a shell for this session id.
    SpawnSession(SessionId),
    /// A terminal tab was closed (or the app is quitting): end the session and drop it.
    KillSession(SessionId),
    /// A browser tab was created; its page is the tab's `url`.
    OpenBrowser(BrowserId),
    /// A browser tab was closed (or the app is quitting): destroy the browser.
    CloseBrowser(BrowserId),
    /// The last workspace was closed: exit the app. The model is left as it was, and the kills
    /// and closes for every tab come before this in the same batch.
    Quit,
    /// A window was created: open one for it. It has focus.
    OpenWindow(WindowId),
    /// A window was closed (its tabs' kills and closes come before this): close it.
    CloseWindow(WindowId),
    /// A command moved focus to this window: bring it to the front.
    FocusWindow(WindowId),
}

/// The whole app state: workspaces, their split trees, panes and tabs.
///
/// Only [`Model::apply`] changes it. Sessions are referred to by id and owned elsewhere, so
/// rearranging the layout never touches a running shell.
#[derive(Debug, Clone)]
pub struct Model {
    /// The focused window's workspaces in sidebar order: pinned ones first, each group's
    /// members contiguous. Other windows' are in `parked`.
    workspaces: Vec<Workspace>,
    /// The focused window's groups; each has at least one workspace (empty ones are dropped
    /// after each command).
    groups: Vec<Group>,
    /// How many groups were ever made, for the "Group N" default name.
    groups_created: u64,
    /// The focused window's workspace on screen; always one of `workspaces`.
    active: WorkspaceId,
    /// The focused window, whose sidebar is `workspaces`, `groups` and `active`.
    window: WindowId,
    /// Every other window's sidebar. A command naming something in one swaps it in first
    /// (`windows.rs`), so the sidebar code only ever deals with the focused window's.
    parked: BTreeMap<WindowId, Parked>,
    /// Every pane, each a leaf of exactly one workspace's layout.
    panes: BTreeMap<PaneId, Pane>,
    /// Every tab, each in exactly one pane.
    tabs: BTreeMap<TabId, Tab>,
    /// The next value of the single id counter every id kind (and [`Notice::seq`]) draws from.
    next_id: u64,
    /// How many workspaces were ever made, for the "Workspace N" default name.
    workspaces_created: u64,
}

impl Model {
    /// A model with one workspace holding one terminal, and the [`Effect::SpawnSession`] for it.
    pub fn new() -> (Self, Vec<Effect>) {
        let mut model = Self {
            workspaces: Vec::new(),
            groups: Vec::new(),
            groups_created: 0,
            active: WorkspaceId(0),
            window: WindowId(0),
            parked: BTreeMap::new(),
            panes: BTreeMap::new(),
            tabs: BTreeMap::new(),
            next_id: 1,
            workspaces_created: 0,
        };
        model.window = WindowId(model.next_id());
        let mut effects = Vec::new();
        model.add_workspace(&mut effects);
        (model, effects)
    }

    /// The windows and layouts as plain data, plus the tabs in the same order
    /// [`Model::restore`] returns them (windows in order, each one's workspaces in sidebar
    /// order, then panes left to right / top to bottom, then tab order). The app saves the spec
    /// and pairs the tabs with their per-tab details (agent, directory, name, pinned); its
    /// workspaces come in the same order as [`Model::all_workspaces`].
    pub fn spec(&self) -> (Spec, Vec<TabId>) {
        let mut tabs = Vec::new();
        let mut spec = Spec {
            workspaces: Vec::new(),
            windows: Vec::new(),
            focused: 0,
        };
        for (index, window) in self.window_views().into_iter().enumerate() {
            if window.id == self.window {
                spec.focused = index;
            }
            let first = spec.workspaces.len();
            for ws in window.workspaces {
                if ws.id == window.active {
                    spec.windows.push(spec.workspaces.len());
                }
                spec.workspaces.push(WorkspaceSpec {
                    name: ws.name.clone(),
                    layout: self.spec_of(&ws.layout, &mut tabs),
                    focused: ws
                        .layout
                        .leaves()
                        .iter()
                        .position(|p| *p == ws.focused)
                        .unwrap_or(0),
                    window: index,
                });
            }
            // The active workspace is always in its sidebar; this only keeps the lists aligned.
            if spec.windows.len() == index {
                spec.windows.push(first);
            }
        }
        (spec, tabs)
    }

    /// The spec of one layout subtree, appending its tabs to `tabs` in leaf order.
    ///
    /// # Panics
    ///
    /// If a leaf or tab is missing from the model, which [`Model::check`] rules out.
    fn spec_of(&self, node: &SplitNode, tabs: &mut Vec<TabId>) -> LayoutSpec {
        match node {
            SplitNode::Leaf(pane) => {
                let pane = &self.panes[pane];
                tabs.extend(&pane.tabs);
                LayoutSpec::Pane {
                    tabs: pane
                        .tabs
                        .iter()
                        .map(|t| match self.tabs[t].kind {
                            TabKind::Terminal(_) => TabSpec::Terminal,
                            TabKind::Browser(_) => {
                                TabSpec::Browser(self.tabs[t].url.clone().unwrap_or_default())
                            }
                        })
                        .collect(),
                    active: pane
                        .tabs
                        .iter()
                        .position(|t| *t == pane.active)
                        .unwrap_or(0),
                }
            }
            SplitNode::Split {
                axis,
                ratio,
                first,
                second,
            } => LayoutSpec::Split {
                axis: *axis,
                ratio: *ratio,
                first: Box::new(self.spec_of(first, tabs)),
                second: Box::new(self.spec_of(second, tabs)),
            },
        }
    }

    /// Rebuilds a saved layout with fresh ids and a new terminal or browser per tab.
    ///
    /// Out-of-range values are clamped: each window's active workspace (one outside it means
    /// its first), the focused window, each workspace's window and `focused`, each pane's
    /// `active` and each split ratio (see [`LayoutSpec::Split::ratio`]). Windows left with no
    /// workspaces are dropped. A pane with no tabs gets one terminal, and no workspaces at all
    /// gives the default single terminal ([`Model::new`]). Returns the model, one
    /// [`Effect::SpawnSession`] or [`Effect::OpenBrowser`] per tab, and the new tabs in the
    /// same order as [`Model::spec`]. No [`Effect::OpenWindow`]: the app opens one window per
    /// [`Model::windows`] itself.
    pub fn restore(spec: Spec) -> (Self, Vec<Effect>, Vec<TabId>) {
        let Spec {
            workspaces,
            windows,
            focused,
        } = spec;
        if workspaces.is_empty() {
            let (model, effects) = Self::new();
            let tabs = model.spec().1;
            return (model, effects, tabs);
        }
        let mut model = Self {
            workspaces: Vec::new(),
            groups: Vec::new(),
            groups_created: 0,
            active: WorkspaceId(0),
            window: WindowId(0),
            parked: BTreeMap::new(),
            panes: BTreeMap::new(),
            tabs: BTreeMap::new(),
            next_id: 1,
            workspaces_created: 0,
        };
        let mut effects = Vec::new();
        let mut tabs = Vec::new();
        let last_window = windows.len().saturating_sub(1);
        // Each workspace's new id and window, in spec order.
        let mut placed: Vec<(WorkspaceId, usize)> = Vec::new();
        for spec in workspaces {
            let window = spec.window.min(last_window);
            let layout = model.build(&spec.layout, &mut effects, &mut tabs);
            let leaves = layout.leaves();
            let focused = leaves[spec.focused.min(leaves.len() - 1)];
            let id = WorkspaceId(model.next_id());
            model.workspaces_created += 1;
            let name = spec.name.trim();
            model.workspaces.push(Workspace {
                id,
                name: if name.is_empty() {
                    format!("Workspace {}", model.workspaces_created)
                } else {
                    name.to_owned()
                },
                layout,
                focused,
                muted: false,
                pinned: false,
                icon: None,
                color: None,
                group: None,
                cwd: None,
                env: Vec::new(),
                startup: None,
                maximized: None,
            });
            placed.push((id, window));
        }
        model.place_restored(&placed, &windows, focused);
        debug_assert_eq!(model.check(), Ok(()));
        (model, effects, tabs)
    }

    /// Builds a layout subtree from its spec, creating its panes and tabs and appending the
    /// new tabs to `tabs` in leaf order.
    fn build(
        &mut self,
        spec: &LayoutSpec,
        effects: &mut Vec<Effect>,
        tabs: &mut Vec<TabId>,
    ) -> SplitNode {
        match spec {
            LayoutSpec::Pane {
                tabs: specs,
                active,
            } => {
                let mut new: Vec<TabId> = specs
                    .iter()
                    .map(|spec| match spec {
                        TabSpec::Terminal => self.new_terminal_tab(effects),
                        TabSpec::Browser(url) => self.new_browser_tab(url, effects),
                    })
                    .collect();
                if new.is_empty() {
                    new.push(self.new_terminal_tab(effects));
                }
                tabs.extend(&new);
                let id = PaneId(self.next_id());
                let active = new[(*active).min(new.len() - 1)];
                self.panes.insert(
                    id,
                    Pane {
                        id,
                        tabs: new,
                        active,
                    },
                );
                SplitNode::Leaf(id)
            }
            LayoutSpec::Split {
                axis,
                ratio,
                first,
                second,
            } => SplitNode::Split {
                axis: *axis,
                ratio: if ratio.is_finite() {
                    ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO)
                } else {
                    0.5
                },
                first: Box::new(self.build(first, effects, tabs)),
                second: Box::new(self.build(second, effects, tabs)),
            },
        }
    }

    /// The focused window's workspaces in sidebar order.
    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    /// Every workspace in every window: windows in order, each one's in sidebar order.
    pub fn all_workspaces(&self) -> Vec<&Workspace> {
        self.window_views()
            .into_iter()
            .flat_map(|w| w.workspaces.iter())
            .collect()
    }

    /// The focused window's workspace on screen.
    ///
    /// # Panics
    ///
    /// Never while the invariants hold: the active workspace always exists.
    pub fn active_workspace(&self) -> &Workspace {
        self.workspace(self.active)
            .expect("the active workspace is in the model")
    }

    /// Every group, in creation order (not sidebar order; that follows the workspaces).
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The group with this id, in any window, if it still exists.
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups
            .iter()
            .chain(self.parked.values().flat_map(|p| &p.groups))
            .find(|g| g.id == id)
    }

    /// The workspace with this id, in any window, if it still exists.
    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .chain(self.parked.values().flat_map(|p| &p.workspaces))
            .find(|w| w.id == id)
    }

    /// The pane with this id, if it still exists.
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.get(&id)
    }

    /// The tab with this id, if it still exists.
    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.get(&id)
    }

    /// The active tab of the active workspace's focused pane: where keyboard input goes.
    ///
    /// # Panics
    ///
    /// Never while the invariants hold: the focused pane and its active tab always exist.
    pub fn focused_tab(&self) -> &Tab {
        let pane = &self.panes[&self.active_workspace().focused];
        &self.tabs[&pane.active]
    }

    /// The tab showing the browser, if any.
    pub fn tab_for_browser(&self, browser: BrowserId) -> Option<TabId> {
        self.tabs
            .values()
            .find(|t| t.kind == TabKind::Browser(browser))
            .map(|t| t.id)
    }

    /// Every browser a live tab refers to.
    pub fn browsers(&self) -> impl Iterator<Item = BrowserId> + '_ {
        self.tabs.values().filter_map(|t| match t.kind {
            TabKind::Browser(browser) => Some(browser),
            TabKind::Terminal(_) => None,
        })
    }

    /// Whether notifications from the tab are dropped: it or its workspace is muted.
    pub fn is_muted(&self, tab: TabId) -> bool {
        self.tabs.get(&tab).is_some_and(|t| t.muted)
            || self
                .pane_of_tab(tab)
                .and_then(|p| self.workspace_of_pane(p))
                .and_then(|w| self.workspace(w))
                .is_some_and(|w| w.muted)
    }

    /// The tab running the session, if any.
    pub fn tab_for_session(&self, session: SessionId) -> Option<TabId> {
        self.tabs
            .values()
            .find(|t| t.kind == TabKind::Terminal(session))
            .map(|t| t.id)
    }

    /// Whether the tab is on screen: the active tab of a pane shown in some window's active
    /// workspace.
    pub fn tab_visible(&self, tab: TabId) -> bool {
        self.window_views().into_iter().any(|window| {
            let Some(ws) = self.workspace(window.active) else {
                return false;
            };
            let shown = match ws.maximized {
                Some(pane) => vec![pane],
                None => ws.layout.leaves(),
            };
            shown
                .iter()
                .filter_map(|p| self.panes.get(p))
                .any(|p| p.active == tab)
        })
    }

    /// Whether any tab in the pane is waiting for the user.
    pub fn pane_attention(&self, pane: PaneId) -> bool {
        self.panes.get(&pane).is_some_and(|p| {
            p.tabs
                .iter()
                .any(|t| self.tabs.get(t).is_some_and(|t| t.attention.is_some()))
        })
    }

    /// The newest notice from any tab in the workspace (highest [`Notice::seq`]), or `None` if
    /// none is waiting or the workspace doesn't exist.
    pub fn workspace_notice(&self, workspace: WorkspaceId) -> Option<&Notice> {
        let ws = self.workspace(workspace)?;
        ws.layout
            .leaves()
            .into_iter()
            .filter_map(|p| self.panes.get(&p))
            .flat_map(|p| &p.tabs)
            .filter_map(|t| self.tabs.get(t)?.attention.as_ref())
            .max_by_key(|n| n.seq)
    }

    /// Every tab waiting for the user (with a notice), oldest notice first.
    pub fn waiting_tabs(&self) -> Vec<TabId> {
        let mut waiting: Vec<(u64, TabId)> = self
            .tabs
            .values()
            .filter_map(|t| Some((t.attention.as_ref()?.seq, t.id)))
            .collect();
        waiting.sort_unstable();
        waiting.into_iter().map(|(_, tab)| tab).collect()
    }

    /// Every session a live tab refers to.
    pub fn sessions(&self) -> impl Iterator<Item = SessionId> + '_ {
        self.tabs.values().filter_map(|t| match t.kind {
            TabKind::Terminal(session) => Some(session),
            TabKind::Browser(_) => None,
        })
    }

    /// Applies one command, returning the side effects for the app to perform.
    ///
    /// Each [`Command`] variant documents what it does and what it emits. After every command,
    /// groups left with no workspaces are dropped, and debug builds assert [`Model::check`].
    pub fn apply(&mut self, command: Command) -> Vec<Effect> {
        let mut effects = Vec::new();
        // The tab `NextWaiting` picks can be in any window, so it's a routed `SelectTab`.
        let command = match command {
            Command::NextWaiting => match self.next_waiting() {
                Some(tab) => Command::SelectTab(tab),
                None => return effects,
            },
            other => other,
        };
        let home = self.window;
        let Some(target) = self.window_for(&command) else {
            return effects;
        };
        self.swap_to(target);
        let follows = matches!(
            command,
            Command::SelectWorkspace(_) | Command::SelectTab(_) | Command::FocusPane(_)
        );
        self.run(command, &mut effects);
        // Back to the window that had focus, unless the command was about showing something
        // there or closed it; a command that opened or moved to a window keeps it focused.
        let chosen = effects
            .iter()
            .any(|e| matches!(e, Effect::OpenWindow(_) | Effect::FocusWindow(_)));
        if self.window != home && !chosen {
            if !follows && self.parked.contains_key(&home) {
                self.swap_to(home);
            } else {
                effects.push(Effect::FocusWindow(self.window));
            }
        }
        self.tidy();
        debug_assert_eq!(self.check(), Ok(()));
        effects
    }

    /// Runs one command on the window it's for, now the focused one.
    fn run(&mut self, command: Command, effects: &mut Vec<Effect>) {
        match command {
            Command::NewWindow => self.new_window(effects),
            Command::CloseWindow(id) => self.close_window(id, effects),
            Command::FocusWindow(_) => {}
            Command::MoveWorkspaceToWindow { workspace, window } => {
                self.move_workspace_to_window(workspace, window, effects)
            }
            Command::NewWorkspace => {
                self.add_workspace(effects);
            }
            Command::CloseWorkspace(id) => self.close_workspace(id, effects),
            Command::CloseActiveWorkspace => self.close_workspace(self.active, effects),
            Command::SelectWorkspace(id) => self.select_workspace(id),
            Command::SelectWorkspaceIndex(index) => self.select_workspace_index(index),
            Command::NextWorkspace => self.cycle_workspace(1),
            Command::PreviousWorkspace => self.cycle_workspace(-1),
            Command::RenameWorkspace(id, name) => self.rename_workspace(id, &name),
            Command::MoveWorkspace(id, to) => self.move_workspace(id, to),
            Command::ShiftWorkspace(id, shift) => self.shift_workspace(id, shift),
            Command::CombineWorkspaces { into, from } => self.combine_workspaces(into, from),
            Command::SetWorkspaceIcon(id, icon) => self.edit_workspace(id, |w| w.icon = icon),
            Command::SetWorkspaceColor(id, color) => self.edit_workspace(id, |w| w.color = color),
            Command::SetWorkspaceCwd(id, cwd) => {
                self.edit_workspace(id, |w| w.cwd = non_blank(cwd))
            }
            Command::SetWorkspaceEnv(id, env) => self.edit_workspace(id, |w| {
                w.env = env
                    .into_iter()
                    .filter(|(name, _)| !name.trim().is_empty())
                    .collect();
            }),
            Command::SetWorkspaceStartup(id, startup) => {
                self.edit_workspace(id, |w| w.startup = non_blank(startup))
            }
            Command::SetWorkspaceMuted(id, muted) => self.set_workspace_muted(id, muted),
            Command::SetWorkspacePinned(id, pinned) => self.set_pinned_block(id, pinned),

            Command::NewGroup(id) => self.new_group(id),
            Command::SetWorkspaceGroup(id, group) => self.set_workspace_group(id, group),
            Command::PlaceWorkspace { id, before, group } => {
                self.place_workspace(id, before, group)
            }
            Command::RenameGroup(id, name) => self.rename_group(id, &name),
            Command::SetGroupCollapsed(id, collapsed) => {
                self.edit_group(id, |g| g.collapsed = collapsed)
            }
            Command::SetGroupColor(id, color) => self.edit_group(id, |g| g.color = color),
            Command::SetGroupIcon(id, icon) => self.edit_group(id, |g| g.icon = icon),
            Command::SetGroupPinned(id, pinned) => self.set_group_pinned(id, pinned),
            Command::MergeGroups { into, from } => self.merge_groups(into, from),
            Command::ShiftGroup(id, shift) => self.shift_group(id, shift),
            Command::Ungroup(id) => self.ungroup(id),

            Command::SplitPane(direction) => self.split(direction, effects),
            Command::ClosePane => self.close_focused_pane(effects),
            Command::FocusPane(pane) => self.focus_pane(pane),
            Command::FocusDirection(direction) => self.focus_direction(direction),
            Command::ToggleMaximize => self.toggle_maximize(),
            Command::EqualizePanes => {
                self.workspace_mut(self.active).layout.equalize(None);
            }
            Command::SwapPane(direction) => self.swap_pane(direction),
            Command::RotatePanes => self.rotate_panes(),
            // `apply` turns it into a `SelectTab`.
            Command::NextWaiting => {}
            Command::SetSplitRatio {
                workspace,
                path,
                ratio,
            } => self.set_split_ratio(workspace, &path, ratio),

            Command::NewTab => self.new_tab(effects),
            Command::OpenBrowser { url, split } => self.open_browser(&url, split, effects),
            Command::OpenBrowserBeside { beside, url } => {
                self.open_browser_beside(beside, &url, effects)
            }
            Command::CloseTab(tab) => self.close_tab(tab, effects),
            Command::CloseActiveTab => self.close_tab(self.focused_tab().id, effects),
            Command::SelectTab(tab) => self.select_tab(tab),
            Command::RenameTab(tab, name) => self.edit_tab(tab, |t| {
                t.name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
            }),
            Command::SetTabMuted(tab, muted) => self.set_tab_muted(tab, muted),
            Command::SetTabPinned(tab, pinned) => self.set_pinned(tab, pinned),
            Command::SetTabIcon(tab, icon) => self.edit_tab(tab, |t| t.icon = icon),
            Command::SetTabColor(tab, color) => self.edit_tab(tab, |t| t.color = color),
            Command::MoveTab { tab, to } => {
                if !self.move_tab_between_windows(tab, to, effects) {
                    self.move_tab(tab, to);
                }
            }
            Command::NextTab => self.cycle_tab(1),
            Command::PreviousTab => self.cycle_tab(-1),

            Command::SetTabTitle(session, title) => {
                self.edit_session_tab(session, |t| t.title = non_blank(title))
            }
            Command::Notify {
                session,
                title,
                body,
            } => self.notify(session, title, body),
            Command::SetTabBadge(session, badge) => {
                if let Some(tab) = self.tab_for_session(session)
                    && !self.is_muted(tab)
                {
                    self.edit_tab(tab, |t| t.badge = non_blank(badge));
                }
            }
            Command::ClearAttention(tab) => self.edit_tab(tab, |t| {
                t.attention = None;
                t.badge = None;
            }),
            Command::SetAgent {
                tab,
                state,
                detail,
                session_id,
                cwd,
            } => self.set_agent(tab, state, detail, session_id, cwd),
            Command::SetAgentSummary { tab, summary } => self.set_agent_summary(tab, summary),
            Command::ClearAgent(tab) => self.edit_tab(tab, |t| t.agent = None),
            Command::SetTabCwd(session, cwd) => {
                self.edit_session_tab(session, |t| t.cwd = non_blank(cwd))
            }
            Command::SessionExited(session) => {
                if let Some(tab) = self.tab_for_session(session) {
                    self.close_tab(tab, effects);
                }
            }
            Command::SetBrowserUrl(browser, url) => {
                self.edit_browser_tab(browser, |t| t.url = Some(url))
            }
            Command::SetBrowserTitle(browser, title) => {
                self.edit_browser_tab(browser, |t| t.title = non_blank(title))
            }
            Command::BrowserClosed(browser) => {
                if let Some(tab) = self.tab_for_browser(browser) {
                    self.close_tab(tab, effects);
                }
            }
        }
    }

    /// What every command leaves to be cleaned up. Closing or moving a group's last workspace
    /// can leave the group empty; dropping it here keeps every command from having to. Likewise
    /// a maximized pane ends whenever focus leaves it (a focus move, a split, a close, a tab
    /// dragged elsewhere), so no command has to undo it.
    fn tidy(&mut self) {
        self.drop_empty_groups();
        for ws in &mut self.workspaces {
            if ws.maximized != Some(ws.focused) {
                ws.maximized = None;
            }
        }
    }

    /// Takes the next value from the shared id counter. Every id kind draws from it, so no two
    /// ids are ever equal and none is reused.
    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The workspace with this id, mutably.
    ///
    /// # Panics
    ///
    /// If the workspace doesn't exist; callers pass ids they just looked up.
    fn workspace_mut(&mut self, id: WorkspaceId) -> &mut Workspace {
        self.workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .expect("the workspace is in the model")
    }

    /// The workspace whose layout holds the pane, in any window, if the pane exists.
    pub fn workspace_of_pane(&self, pane: PaneId) -> Option<WorkspaceId> {
        self.workspaces
            .iter()
            .chain(self.parked.values().flat_map(|p| &p.workspaces))
            .find(|w| w.layout.contains(pane))
            .map(|w| w.id)
    }

    /// The pane holding the tab, if the tab exists.
    pub fn pane_of_tab(&self, tab: TabId) -> Option<PaneId> {
        self.panes
            .values()
            .find(|p| p.tabs.contains(&tab))
            .map(|p| p.id)
    }
}
