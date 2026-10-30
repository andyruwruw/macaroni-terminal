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
//! - `check`: the invariants [`Model::check`] verifies after every command in debug builds.

mod check;
mod events;
mod groups;
mod panes;
mod tabs;
mod workspaces;

use std::collections::BTreeMap;

use crate::ids::{BrowserId, GroupId, PaneId, SessionId, TabId, WorkspaceId};
use crate::layout::{Axis, Direction, MIN_RATIO, Side, SplitNode};
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
    /// The workspace's folder: where its new terminals start. `None` is the home directory.
    /// The model doesn't check it exists; the app does when it spawns.
    pub cwd: Option<String>,
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
    /// The user typed into or clicked the tab: drops its notice.
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
}

/// The whole app state: workspaces, their split trees, panes and tabs.
///
/// Only [`Model::apply`] changes it. Sessions are referred to by id and owned elsewhere, so
/// rearranging the layout never touches a running shell.
#[derive(Debug, Clone)]
pub struct Model {
    /// Every workspace in sidebar order: pinned ones first, each group's members contiguous.
    workspaces: Vec<Workspace>,
    /// Every group; each has at least one workspace (empty ones are dropped after each command).
    groups: Vec<Group>,
    /// How many groups were ever made, for the "Group N" default name.
    groups_created: u64,
    /// The workspace on screen; always one of `workspaces`.
    active: WorkspaceId,
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
            panes: BTreeMap::new(),
            tabs: BTreeMap::new(),
            next_id: 1,
            workspaces_created: 0,
        };
        let mut effects = Vec::new();
        model.add_workspace(&mut effects);
        (model, effects)
    }

    /// The layout as plain data, plus the tabs in the same order [`Model::restore`] returns
    /// them (workspace order, then panes left to right / top to bottom, then tab order).
    ///
    /// Returns the workspace specs, the active workspace's index, and the tabs. The app saves
    /// the specs and pairs the tabs with their per-tab details (agent, directory, name, pinned).
    pub fn spec(&self) -> (Vec<WorkspaceSpec>, usize, Vec<TabId>) {
        let mut tabs = Vec::new();
        let workspaces = self
            .workspaces
            .iter()
            .map(|ws| WorkspaceSpec {
                name: ws.name.clone(),
                layout: self.spec_of(&ws.layout, &mut tabs),
                focused: ws
                    .layout
                    .leaves()
                    .iter()
                    .position(|p| *p == ws.focused)
                    .unwrap_or(0),
            })
            .collect();
        let active = self
            .workspaces
            .iter()
            .position(|w| w.id == self.active)
            .unwrap_or(0);
        (workspaces, active, tabs)
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
    /// Out-of-range values are clamped: `active`, each workspace's `focused`, each pane's
    /// `active` and each split ratio (see [`LayoutSpec::Split::ratio`]). A pane with no tabs gets
    /// one terminal, and an empty list gives the default single terminal ([`Model::new`]).
    /// Returns the model, one [`Effect::SpawnSession`] or [`Effect::OpenBrowser`] per tab, and
    /// the new tabs in the same order as [`Model::spec`].
    pub fn restore(
        workspaces: Vec<WorkspaceSpec>,
        active: usize,
    ) -> (Self, Vec<Effect>, Vec<TabId>) {
        if workspaces.is_empty() {
            let (model, effects) = Self::new();
            let tabs = model.spec().2;
            return (model, effects, tabs);
        }
        let mut model = Self {
            workspaces: Vec::new(),
            groups: Vec::new(),
            groups_created: 0,
            active: WorkspaceId(0),
            panes: BTreeMap::new(),
            tabs: BTreeMap::new(),
            next_id: 1,
            workspaces_created: 0,
        };
        let mut effects = Vec::new();
        let mut tabs = Vec::new();
        for spec in workspaces {
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
            });
        }
        model.active = model.workspaces[active.min(model.workspaces.len() - 1)].id;
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

    /// Every workspace in sidebar order.
    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    /// The workspace on screen.
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

    /// The group with this id, if it still exists.
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// The workspace with this id, if it still exists.
    pub fn workspace(&self, id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
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

    /// Whether the tab is on screen: the active tab of a pane in the active workspace.
    pub fn tab_visible(&self, tab: TabId) -> bool {
        self.active_workspace()
            .layout
            .leaves()
            .iter()
            .filter_map(|p| self.panes.get(p))
            .any(|p| p.active == tab)
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
        match command {
            Command::NewWorkspace => {
                self.add_workspace(&mut effects);
            }
            Command::CloseWorkspace(id) => self.close_workspace(id, &mut effects),
            Command::CloseActiveWorkspace => self.close_workspace(self.active, &mut effects),
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

            Command::SplitPane(direction) => self.split(direction, &mut effects),
            Command::ClosePane => self.close_focused_pane(&mut effects),
            Command::FocusPane(pane) => self.focus_pane(pane),
            Command::FocusDirection(direction) => self.focus_direction(direction),
            Command::SetSplitRatio {
                workspace,
                path,
                ratio,
            } => self.set_split_ratio(workspace, &path, ratio),

            Command::NewTab => self.new_tab(&mut effects),
            Command::OpenBrowser { url, split } => self.open_browser(&url, split, &mut effects),
            Command::OpenBrowserBeside { beside, url } => {
                self.open_browser_beside(beside, &url, &mut effects)
            }
            Command::CloseTab(tab) => self.close_tab(tab, &mut effects),
            Command::CloseActiveTab => self.close_tab(self.focused_tab().id, &mut effects),
            Command::SelectTab(tab) => self.select_tab(tab),
            Command::RenameTab(tab, name) => self.edit_tab(tab, |t| {
                t.name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
            }),
            Command::SetTabMuted(tab, muted) => self.set_tab_muted(tab, muted),
            Command::SetTabPinned(tab, pinned) => self.set_pinned(tab, pinned),
            Command::SetTabIcon(tab, icon) => self.edit_tab(tab, |t| t.icon = icon),
            Command::SetTabColor(tab, color) => self.edit_tab(tab, |t| t.color = color),
            Command::MoveTab { tab, to } => self.move_tab(tab, to),
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
            Command::ClearAttention(tab) => self.edit_tab(tab, |t| t.attention = None),
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
                    self.close_tab(tab, &mut effects);
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
                    self.close_tab(tab, &mut effects);
                }
            }
        }
        // Closing or moving a group's last workspace can leave the group empty; dropping it here
        // keeps every command from having to.
        self.drop_empty_groups();
        debug_assert_eq!(self.check(), Ok(()));
        effects
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

    /// The workspace whose layout holds the pane, if the pane exists.
    pub fn workspace_of_pane(&self, pane: PaneId) -> Option<WorkspaceId> {
        self.workspaces
            .iter()
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
