//! The app's transient UI state: open menus, drags, renames, the find bar and hover, plus
//! [`Running`], a started terminal, and [`Starting`], one whose shell is on its way.
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

/// A terminal whose shell is being started off the UI thread ([`crate::message::Started`]).
pub(crate) struct Starting {
    /// Its tab, so a tab closed meanwhile can still drop its restored entry.
    pub(crate) tab: Option<TabId>,
    /// The size the shell is started at.
    pub(crate) spawned: GridSize,
    /// The size its view last reported, which the PTY is resized to if it changed meanwhile.
    pub(crate) size: GridSize,
    /// What the tab was before a restart: its saved scrollback and the agent to resume.
    pub(crate) restored: crate::session::SavedTab,
}

/// The search bar, attached to one session (the focused pane's active tab when opened).
pub(crate) struct Search {
    /// The terminal or page being searched.
    pub(crate) target: SearchTarget,
    /// What's typed in the bar.
    pub(crate) query: String,
    /// Whether the last search found a match (an empty query counts as found); the bar says
    /// "No matches" otherwise.
    pub(crate) found: bool,
}

/// What the find bar searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchTarget {
    /// A terminal's scrollback.
    Terminal(SessionId),
    /// A browser tab's page.
    Page(macaroni_core::BrowserId),
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
    /// The item the keyboard has highlighted, once a key has moved it (or the menu was opened
    /// from the keyboard).
    pub(crate) cursor: Option<MenuCursor>,
}

/// The item the keyboard highlights in an open menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuCursor {
    /// In the open submenu rather than the main list.
    pub(crate) sub: bool,
    /// The entry's index in its list (always an item, never a separator or header).
    pub(crate) index: usize,
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

/// What the keyboard has focused outside the terminals (F6, `update/keyboard.rs`): a sidebar
/// row or a tab strip's stop. Each has a focus ring and a widget id
/// ([`KeyFocus::widget_id`]) on its outermost container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KeyFocus {
    /// A workspace in the sidebar.
    Workspace(WorkspaceId),
    /// A group header in the sidebar.
    Group(GroupId),
    /// The sidebar's New workspace button.
    NewWorkspace,
    /// A tab in a pane's strip.
    Tab(TabId),
    /// A pane's `+` button.
    Add(PaneId),
    /// One of a pane's new tab buttons, by its place in `App::strip_buttons`.
    Button(PaneId, usize),
}

impl KeyFocus {
    /// The widget id of the item's outermost container, to find where it is on screen.
    pub(crate) fn widget_id(self) -> iced::widget::Id {
        // Model ids never share a value across kinds, so the raw value is enough for those.
        iced::widget::Id::from(match self {
            Self::Workspace(WorkspaceId(id)) | Self::Group(GroupId(id)) | Self::Tab(TabId(id)) => {
                format!("item-{id}")
            }
            Self::NewWorkspace => "new-workspace".to_owned(),
            Self::Add(PaneId(pane)) => format!("add-{pane}"),
            Self::Button(PaneId(pane), n) => format!("button-{pane}-{n}"),
        })
    }

    /// Whether it's in the sidebar rather than a tab strip.
    pub(crate) fn in_sidebar(self) -> bool {
        matches!(
            self,
            Self::Workspace(_) | Self::Group(_) | Self::NewWorkspace
        )
    }
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
    /// A link or file Ctrl+right-clicked in a terminal: where to open it (`App.clicked`).
    Link,
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
    /// Which plugins are on in a tab.
    Plugins,
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
    /// Which window the cursor is over: the one the drag started in, another, or none.
    pub(crate) over: Over,
}

/// Which window a dragged item is over ([`ItemDrag::over`]). The OS keeps sending the cursor to
/// the window the drag started in (the focused one), in its coordinates, wherever it goes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum Over {
    /// The window the drag started in.
    #[default]
    Here,
    /// Another window, with the cursor in its coordinates.
    Window(iced::window::Id, Point),
    /// No window: a drop opens a new one there (screen coordinates).
    Outside(Point),
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
    /// Along an edge of a whole workspace (the active one of the window under the cursor).
    Edge(WorkspaceId, Direction),
    /// A workspace in the sidebar.
    Workspace(WorkspaceId),
    /// The sidebar's "New workspace" button: the tab moves to a new workspace.
    NewWorkspace,
    /// Outside every window: the tab moves to a new window.
    NewWindow,
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
