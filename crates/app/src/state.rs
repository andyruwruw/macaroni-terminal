//! The app's transient UI state: open menus, drags, renames, the find bar and hover, plus
//! [`Running`], a started terminal.
//!
//! These are plain data held in `App` fields. The handlers in `update/` change them and the
//! view in `view/` reads them; `view/drag.rs` turns [`ItemDrag`] and the hover state into a
//! [`DropTarget`] or a [`WorkspaceDrop`].

use iced::Point;
use macaroni_core::{Direction, Divider, GroupId, PaneId, SessionId, TabId, WorkspaceId};
use macaroni_term::{GridSize, Session, Snapshot};

/// A terminal that has started.
pub(crate) struct Running {
    /// The shell and its emulator. Dropping it kills the process.
    pub(crate) session: Session,
    /// What the view draws: refreshed on every wakeup and after anything that changes the
    /// screen (a resize, a selection, scrolling, a search).
    pub(crate) snapshot: Snapshot,
    /// The grid size last sent to the PTY, so a view reporting the same size doesn't resize
    /// (and reflow) the shell again.
    pub(crate) size: GridSize,
}

/// The search bar, attached to one session (the focused pane's active tab when opened).
pub(crate) struct Search {
    /// The terminal being searched.
    pub(crate) session: SessionId,
    /// What's typed in the bar.
    pub(crate) query: String,
    /// Whether the last search found a match (an empty query counts as found); the bar says
    /// "No matches" otherwise.
    pub(crate) found: bool,
}

/// What's being renamed in place, or (as `App.item_focus`) what Enter would rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Renaming {
    /// A sidebar workspace.
    Workspace(WorkspaceId),
    /// A tab.
    Tab(TabId),
    /// A sidebar group header.
    Group(GroupId),
}

/// A right-click menu, opened at `at` (window coordinates).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Menu {
    /// What the menu is for, which decides its entries.
    pub(crate) target: MenuTarget,
    /// Where it was opened: the main list's top left, before it's kept inside the window.
    pub(crate) at: Point,
    /// The list flown out beside the main one, if any.
    pub(crate) sub: Option<Submenu>,
}

/// A submenu opened by hovering its item (closed again by hovering a plain item) or by
/// clicking it (`pinned`: open until the menu closes or another submenu opens).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Submenu {
    /// Which list it shows.
    pub(crate) page: MenuPage,
    /// Opened by a click, so hovering a plain item doesn't close it.
    pub(crate) pinned: bool,
}

/// What a right-click menu was opened on.
#[derive(Debug, Clone, Copy)]
pub(crate) enum MenuTarget {
    /// A pane's new-tab button: what kind of tab to open.
    NewTab(PaneId),
    /// A tab in a pane's strip.
    Tab(TabId),
    /// A workspace in the sidebar.
    Workspace(WorkspaceId),
    /// A group header in the sidebar.
    Group(GroupId),
}

/// Which list a menu shows: its main one, or a submenu beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuPage {
    /// The list opened at the cursor.
    Main,
    /// Where to move a tab, workspace or group.
    Move,
    /// A workspace's icon.
    Icon,
    /// A workspace's or group's color.
    Color,
    /// After a group's color is picked: whether its workspaces get it too.
    MembersColor,
    /// A workspace's group.
    Group,
    /// Which workspace to combine into a workspace.
    Combine,
    /// Which group to merge into a group.
    Merge,
    /// Whether to delete a group and close its workspaces.
    Delete,
}

/// A tab or workspace being dragged: armed when pressed, dragging once the cursor has moved far
/// enough.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ItemDrag {
    /// What was pressed.
    pub(crate) item: Dragged,
    /// Where the cursor was when the drag was armed; `None` until the first move.
    pub(crate) from: Option<Point>,
    /// Where the cursor is now (window coordinates); the origin until the first move.
    pub(crate) at: Point,
    /// The cursor has moved past the drag threshold, so releasing drops rather than clicks.
    /// Stays set once set.
    pub(crate) dragging: bool,
}

/// Which kind of item an [`ItemDrag`] holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Dragged {
    /// A tab from a pane's strip.
    Tab(TabId),
    /// A workspace from the sidebar.
    Workspace(WorkspaceId),
}

impl ItemDrag {
    /// The dragged tab, or `None` when a workspace is dragged.
    pub(crate) fn tab(&self) -> Option<TabId> {
        match self.item {
            Dragged::Tab(tab) => Some(tab),
            Dragged::Workspace(_) => None,
        }
    }
}

/// Where a dragged tab would land.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DropTarget {
    /// Into a pane's tab strip, at a position.
    Strip {
        /// The pane whose strip it is.
        pane: PaneId,
        /// Where in the strip, before the clamping `MoveTo::Pane` applies.
        index: usize,
    },
    /// Onto a pane's body: into it, or split off on a side.
    Pane {
        /// The pane under the cursor.
        pane: PaneId,
        /// The side to split off on, or `None` to join the pane.
        side: Option<Direction>,
    },
    /// Along an edge of the whole workspace.
    Edge(Direction),
    /// A workspace in the sidebar.
    Workspace(WorkspaceId),
    /// The sidebar's "New workspace" button: the tab moves to a new workspace.
    NewWorkspace,
}

/// What's under the cursor in the sidebar, for dropping tabs and workspaces on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarHover {
    /// A workspace item.
    Workspace(WorkspaceId),
    /// A group header.
    Group(GroupId),
    /// The "New workspace" button.
    NewWorkspace,
}

/// Where a dragged workspace would go, as `Command::PlaceWorkspace` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorkspaceDrop {
    /// The workspace it goes before, or `None` for the end.
    pub(crate) before: Option<WorkspaceId>,
    /// The group it joins, or `None` to be ungrouped.
    pub(crate) group: Option<GroupId>,
    /// Dropped on a group's header: the header is marked rather than a gap.
    pub(crate) onto_group: bool,
}

/// A divider between panes being dragged.
pub(crate) struct Drag {
    /// The workspace whose split it is.
    pub(crate) workspace: WorkspaceId,
    /// The divider, as laid out when pressed: its split's path, axis and area (relative to the
    /// workspace area's origin).
    pub(crate) divider: Divider,
}
