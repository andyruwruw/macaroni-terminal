//! Every message the app handles, and the small enums that ride along in them.
//!
//! Widgets in `view/`, the subscriptions in `main.rs` and the session, browser and socket
//! threads all produce [`Message`]s; `App::handle` in `main.rs` dispatches each one to its
//! handler in `update/`. The action enums ([`TabAction`], [`WorkspaceAction`], [`GroupAction`])
//! are what right-click menu items carry; `update/actions.rs` turns them into model commands.

use iced::futures::channel::mpsc::UnboundedSender;
use iced::{Point, Rectangle, Size, window};
use macaroni_core::{
    BrowserId, Command, Divider, GroupId, MoveTo, PaneId, SessionId, Shift, TabId, WorkspaceId,
};
use macaroni_term::{SearchDirection, SessionEvent};

use crate::browser::BrowserMessage;
use crate::keymap::Action;
use crate::terminal::ViewEvent;

use crate::state::{MenuPage, MenuTarget, Renaming, SidebarHover};
use crate::{ipc, pointer, view};

/// An item on a tab's right-click menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TabAction {
    /// Puts a rename field in place of the tab's title.
    Rename,
    /// Moves the tab: to a new workspace, a split, or another workspace.
    Move(MoveTo),
    /// Mutes (`true`) or unmutes the tab's notifications.
    Mute(bool),
    /// Pins (`true`) or unpins the tab.
    Pin(bool),
    /// Copies the tab's id (what the CLI's `--tab` takes) to the clipboard.
    CopyId,
    /// Gives the tab one of the workspace icons, or (`None`) the default one.
    Icon(Option<&'static str>),
    /// Gives the tab a preset color by name, or (`None`) none.
    Color(Option<&'static str>),
    /// Closes the tab.
    Close,
    /// Closes every other unpinned tab in its pane.
    CloseOthers,
    /// Closes the unpinned tabs before it in its pane.
    CloseLeft,
    /// Closes the unpinned tabs after it in its pane.
    CloseRight,
    /// Makes the tab's directory its workspace's folder.
    UseAsFolder,
    /// Types `cd` to its workspace's folder into the tab's shell.
    GoToFolder,
}

/// An item on a sidebar workspace's right-click menu.
#[derive(Debug, Clone, Copy)]
pub(crate) enum WorkspaceAction {
    /// Puts a rename field in place of the workspace's name.
    Rename,
    /// Moves the workspace up, down, to the top or to the bottom.
    Shift(Shift),
    /// Mutes (`true`) or unmutes the workspace's notifications.
    Mute(bool),
    /// Pins (`true`) or unpins the workspace (its whole group, if it's in one).
    Pin(bool),
    /// Copies the workspace's id to the clipboard.
    CopyId,
    /// Closes the workspace and everything in it.
    Close,
    /// Gives the workspace an icon by name, or (`None`) the default one.
    Icon(Option<&'static str>),
    /// Gives the workspace a preset color by name, or (`None`) none.
    Color(Option<&'static str>),
    /// Starts a new group holding the workspace, then renames the group.
    NewGroup,
    /// Moves the workspace into a group, or (`None`) out of its group.
    Group(Option<GroupId>),
    /// Moves every pane of this other workspace into the workspace, which it then leaves.
    Combine(WorkspaceId),
}

/// An item on a sidebar group header's right-click menu.
#[derive(Debug, Clone)]
pub(crate) enum GroupAction {
    /// Puts a rename field in place of the group's name.
    Rename,
    /// Sets the group's color (a preset name or `#rrggbb`), or (`None`) clears it.
    Color(Option<String>),
    /// Gives the group's workspaces its color.
    ColorMembers,
    /// Gives the group an icon by name, or (`None`) none.
    Icon(Option<&'static str>),
    /// Collapses (`true`) or expands the group.
    Collapse(bool),
    /// Moves the whole group up, down, to the top or to the bottom.
    Shift(Shift),
    /// Pins (`true`) or unpins every workspace in the group.
    Pin(bool),
    /// Dissolves the group, leaving its workspaces where they are.
    Ungroup,
    /// Moves this other group's workspaces into the group, which it then leaves.
    Merge(GroupId),
    /// Closes the group's workspaces (and so the group).
    Delete,
}

/// What a color menu's custom field colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorTarget {
    /// A sidebar workspace.
    Workspace(WorkspaceId),
    /// A sidebar group.
    Group(GroupId),
    /// A tab.
    Tab(TabId),
}

/// Everything that can happen to the app. Variants are grouped by what produces them; each
/// says what triggers it and what handling it does.
#[derive(Debug, Clone)]
pub(crate) enum Message {
    /// The session subscription is up: the sender every terminal reports on. Starts the
    /// terminals whose views already reported a size.
    EventsReady(UnboundedSender<(SessionId, SessionEvent)>),
    /// A terminal's reader or waiter thread reported something (new output, a title, a bell, a
    /// notification, its working directory, a clipboard write, the process exiting).
    Session(SessionId, SessionEvent),
    /// A terminal view reported something: its grid size, typed input, a selection, scrolling,
    /// a click, a link, a shortcut, or Enter to rename the focused item.
    View(SessionId, ViewEvent),
    /// A button or item that maps straight to a model command (close a tab, new workspace...).
    /// Applied with `App::apply`.
    Command(Command),
    /// Several commands from one click, applied in order (the strip's `+` focuses its pane,
    /// then opens a tab).
    Commands(Vec<Command>),
    /// A keymap action from a button (Settings, Open config.toml). Shortcuts pressed in a
    /// terminal or a page arrive as [`ViewEvent::Action`] or a browser message instead.
    Action(Action),
    /// The config directory changed on disk: reload the file and apply it, or show why not.
    ConfigChanged,
    /// The window gained (`true`) or lost keyboard focus.
    WindowFocused(bool),
    /// The error banner's close button: hides the config errors until the next reload.
    DismissConfigErrors,
    /// A divider between panes was pressed: follow the cursor until release.
    DragStart(WorkspaceId, Divider),
    /// The cursor moved while a divider is dragged (window coordinates).
    DragMove(Point),
    /// The left button was released while a divider is dragged.
    DragEnd,
    /// The sidebar's right edge was pressed, to drag its width.
    SidebarDragStart,
    /// The sidebar's search field changed.
    SidebarSearch(String),
    /// Enter in the sidebar's search field: open the first match.
    SidebarSearchSubmit,
    /// Empty the sidebar's search field.
    SidebarSearchClear,
    /// A press anywhere: find out whether the sidebar's search field has the keyboard now.
    SidebarSearchCheck,
    /// Whether the sidebar's search field has the keyboard.
    SidebarSearchFocused(bool),
    /// A press outside the rename field: drop the edit.
    RenameCancel(Renaming),
    /// A press outside the focused item: Enter no longer renames it.
    ItemBlur(Renaming),
    /// The cursor moved to a zone (half, or the gap beside it) of a sidebar item.
    SidebarZone(SidebarHover, pointer::Zone),
    /// The cursor moved to a zone (half, or the gap beside it) of a tab: its pane and position.
    TabZone(PaneId, usize, pointer::Zone),
    /// The wheel over a pane's tab strip, which the strip's scrollable didn't use: scrolls it
    /// sideways.
    TabScroll(PaneId, iced::mouse::ScrollDelta),
    /// A pane's tab strip scrolled or changed size while its tabs overflow it.
    TabViewport(PaneId, iced::widget::scrollable::Viewport),
    /// Typing in the rename field.
    RenameInput(String),
    /// Enter in the rename field: gives the item its new name.
    RenameCommit,
    /// Escape: ends an item drag, or closes the menu or the settings dialog, or else cancels a
    /// rename. Also lets go of the focused item.
    Dismiss,
    /// A right-click on a tab, sidebar item or the new-tab button: opens its menu at the
    /// cursor (window coordinates).
    MenuOpen(MenuTarget, Point),
    /// A tab was pressed: selects it and arms a drag.
    TabPress(TabId),
    /// A sidebar workspace was pressed: selects it and arms a drag.
    WorkspacePress(WorkspaceId),
    /// The cursor moved while a tab or workspace is pressed (window coordinates).
    ItemDragMove(Point),
    /// The left button was released while a tab or workspace is pressed: drops it if it was
    /// being dragged.
    ItemDragEnd,
    /// The cursor entered (`true`) or left a tab: its pane and position.
    TabHover(PaneId, usize, bool),
    /// The cursor entered (`true`) or left a sidebar item.
    SidebarHover(SidebarHover, bool),
    /// While a dragged tab hovers a sidebar workspace: open it once it's been long enough.
    DragTick,
    /// Redraws while a selection or hover fade runs.
    Frame,
    /// A tick while the session has unsaved changes, so the last change is saved within a
    /// second even if no command follows it.
    SaveTick,
    /// Something in the settings dialog: picking a section, searching, or changing a setting
    /// (which writes the config file).
    Settings(view::settings::SettingsMessage),
    /// Something in the icon picker.
    IconPicker(view::icon_picker::IconPickerMessage),
    /// Something in the workspace folder dialog.
    Folder(view::folder_dialog::FolderMessage),
    /// A press on the dimmed backdrop around the settings dialog: closes it.
    SettingsClose,
    /// The cursor entered a menu item: a submenu's, or (`None`) a plain one.
    MenuHover(Option<MenuPage>),
    /// A submenu's item was clicked: keep it open.
    MenuPin(MenuPage),
    /// Typing in a color menu's custom `#rrggbb` field.
    ColorInput(String),
    /// Enter in a color menu's custom field: applies the color if it is one.
    ColorSubmit(ColorTarget),
    /// A click outside the open menu, or an item that only closes it.
    MenuClose,
    /// An item on a tab's menu was picked. Closes the menu.
    Tab(TabId, TabAction),
    /// An item on a workspace's menu was picked. Closes the menu.
    Workspace(WorkspaceId, WorkspaceAction),
    /// An item on a group's menu was picked. Closes the menu, except after picking a color.
    Group(GroupId, GroupAction),
    /// The clipboard's text (`None` if it had none), read for pasting into a terminal.
    Paste(SessionId, Option<String>),
    /// Typing in the find bar: searches again from the newest line.
    SearchInput(String),
    /// Enter or an arrow in the find bar: steps to the next match that way.
    SearchStep(SearchDirection),
    /// Escape or `x` in the find bar: closes it and clears the match.
    SearchClose,
    /// The control socket: listening started or failed, a connection opened or closed, or a
    /// request (valid or not) arrived.
    Ipc(ipc::IpcMessage),
    /// The window's close button (or Alt+F4): save the session, close the browsers, quit.
    CloseRequested,
    /// The new-tab menu's shell at an index into `App.shells` was picked for a pane.
    NewTabWith(PaneId, usize),
    /// The Git Bash banner's install button: opens a tab running winget.
    InstallGitBash,
    /// The Git Bash banner's "Not now": hides it for this run.
    DismissShellOffer,
    /// The browser subscription is up: the sender every browser reports on.
    BrowserEventsReady(UnboundedSender<(BrowserId, BrowserMessage)>),
    /// A browser reported something (address, title, loading state, focus, a new window, it
    /// closed itself), a shortcut was pressed in its page, or its page text arrived.
    Browser(BrowserId, BrowserMessage),
    /// Where a browser's pane body was laid out (logical pixels), reported by the view on each
    /// layout; `App::sync_browsers` moves the native window there.
    BrowserBounds(BrowserId, Rectangle),
    /// Typing in a browser's address bar.
    AddressInput(BrowserId, String),
    /// Enter in a browser's address bar: goes there (an address or a search).
    AddressSubmit(BrowserId),
    /// A browser's back, forward, reload, stop or devtools button.
    Nav(BrowserId, Nav),
    /// The new-tab menu's "Browser": opens the home page in a new tab in the pane.
    NewBrowserIn(PaneId),
    /// Opens the pane's directory in VS Code.
    OpenVsCode(PaneId),
    /// The main window opened: remember it and ask for its native handle and scale.
    WindowOpened(window::Id),
    /// A custom title bar control, or a press on a resize border.
    Window(WindowOp),
    /// The window was resized (logical pixels): remember the size and ask if it's maximized.
    WindowResized(Size),
    /// Whether the window is maximized, answered after a resize.
    Maximized(bool),
    /// The main window's native handle (`HWND`), the parent for browser windows; `None` where
    /// there isn't one.
    WindowHandle(Option<isize>),
    /// The window's scale factor, at open and whenever it changes.
    Rescaled(f32),
    /// A click in macaroni's own UI while a page had keyboard focus.
    ReclaimFocus,
    /// A key nothing else handled while a browser tab is focused: maybe a shortcut.
    Key(
        iced::keyboard::Key,
        iced::keyboard::key::Physical,
        iced::keyboard::Modifiers,
    ),
}

/// The custom title bar's controls, and resizing from the window's edges.
#[derive(Debug, Clone, Copy)]
pub(crate) enum WindowOp {
    /// Moves the window with the cursor (the title bar's empty area).
    Drag,
    /// Maximizes or restores (the maximize button, or a double-click on the title bar).
    ToggleMaximize,
    /// Minimizes the window.
    Minimize,
    /// Resizes from an edge or corner with the cursor.
    Resize(window::Direction),
}

/// A browser address bar's buttons.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Nav {
    /// Goes back in the page's history.
    Back,
    /// Goes forward in the page's history.
    Forward,
    /// Reloads the page.
    Reload,
    /// Stops loading the page.
    Stop,
    /// Opens Chromium's developer tools for the page.
    DevTools,
}
