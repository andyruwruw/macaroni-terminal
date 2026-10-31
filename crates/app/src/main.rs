//! macaroni: a GPU-rendered terminal with a workspace sidebar, split panes, tabs, browser tabs
//! and attention notifications for coding agents.
//!
//! The app is one iced application (`iced::application(App::new, App::update, App::view)`)
//! around one [`App`]. `App` owns the pure [`Model`] from `macaroni-core` (workspaces, panes,
//! tabs) plus everything the model doesn't know about: running terminal sessions, browser
//! panels, the config and the look derived from it, and transient UI state (menus, drags,
//! renames, the find bar, hover; see [`state`]).
//!
//! # The update loop
//!
//! Every input is a [`Message`]. [`App::update`] hands it to [`App::handle`], a flat dispatcher
//! that calls one handler per message in the `update/` modules (one file per area: effects,
//! terminals, browser, items, menus, rename, search, shortcuts, config, dividers, window,
//! actions). Anything that changes the model goes through [`App::apply`], which runs
//! [`Model::apply`], carries out the returned [`macaroni_core::Effect`]s (start or kill a
//! session, open or close a browser, quit), broadcasts workspace changes to control API
//! subscribers and saves the session. After every message, `update` also re-places the native
//! browser windows and re-targets the selection and hover fades, since both depend on whatever
//! state the handler just changed.
//!
//! The view (`view/`, entry point [`App::view`]) is a pure function of `App`: the sidebar, the
//! panes laid out from the model's split tree with a `TerminalView` widget per terminal, menus,
//! banners and the settings dialog. Widgets report back through `Message`s; terminal views
//! through [`terminal::ViewEvent`] wrapped in [`Message::View`].
//!
//! # Where events come from
//!
//! [`App::subscription`] runs long-lived streams and, only while they matter, event listeners:
//!
//! - **Sessions**: [`session_events`] makes one channel; its sender arrives as
//!   [`Message::EventsReady`] and is cloned into each `Session`'s callback, so the reader and
//!   waiter threads of every terminal report as [`Message::Session`]. Sessions start lazily:
//!   [`macaroni_core::Effect::SpawnSession`] only marks the id pending, and the terminal starts
//!   when its view first reports a grid size, so shells never start at a placeholder size.
//! - **Browsers**: [`browser_events`] does the same for CEF windows ([`Message::Browser`]). A
//!   browser's native window starts the first time its tab is on screen
//!   ([`App::sync_browsers`]).
//! - **Control API**: [`ipc::events`] listens on the per-user socket; requests arrive as
//!   [`Message::Ipc`] and are answered on the UI thread with the same commands the keyboard
//!   uses.
//! - **Config**: [`config_events`] watches the config directory and sends
//!   [`Message::ConfigChanged`], which reloads and re-applies the file.
//! - **Window and input**: open, close request, focus, resize and scale events; cursor moves and
//!   releases only while a divider or item drag runs; Escape only while something can be
//!   dismissed; animation frames only while a fade or a drag-hover timer is running.
//!
//! Startup ([`App::new`]) loads the config, restores the saved session through [`restore`]
//! (unless a program was given on the command line), and runs the model's first effects.
//! Closing the window saves the session, closes the browsers and exits.

// Release builds are GUI apps on Windows, so launching one doesn't also open a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod anim;
mod backdrop;
mod bell;
mod browser;
mod drag_area;
mod fonts;
mod gap_line;
mod glass;
mod hair_line;
mod icons;
mod inert;
mod input;
mod ipc;
mod keymap;
mod label;
mod message;
#[cfg(windows)]
mod move_dpi;
mod notifications;
mod pointer;
mod restore;
mod right_click;
mod session;
mod side_line;
mod state;
mod terminal;
mod theme;
mod update;
mod view;
mod vscode;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::{UnboundedSender, unbounded};
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::{Event, Size, Subscription, Task, event, mouse, window};
use macaroni_config::Config;
use macaroni_core::{BrowserId, Model, PaneId, SessionId, TabId, TabKind};
use macaroni_term::shell::{self, ShellOption};
use macaroni_term::{GridSize, SessionEvent, Snapshot, is_openable};

use browser::{BrowserMessage, Panel};
use keymap::Keymap;
use message::Message;
use state::{Drag, ItemDrag, Menu, Renaming, Running, Search, SidebarHover};
use terminal::Metrics;
use theme::{Chrome, Palette};

// The view modules import these through the crate root.
use message::{ColorTarget, GroupAction, Nav, TabAction, WindowOp, WorkspaceAction};
use state::{Dragged, DropTarget, MenuPage, MenuTarget, WorkspaceDrop};

/// The app's name, in window titles and notifications.
const APP_NAME: &str = "macaroni";
/// The window's size when it first opens.
const DEFAULT_WINDOW_SIZE: Size = Size::new(1200.0, 720.0);

/// The window's icon: the white logo on the app's background, 256 pixels square.
const APP_ICON: &[u8] = include_bytes!("../assets/app-icon.png");
/// The font size terminals are measured at until the config applies (at the end of
/// [`App::new`]).
const PLACEHOLDER_FONT_SIZE: f32 = 14.0;
/// The smallest font size zooming or the settings allow.
const MIN_FONT_SIZE: f32 = 6.0;
/// The largest font size zooming or the settings allow.
const MAX_FONT_SIZE: f32 = 72.0;
/// How many session or browser events a subscription's output holds before the forwarding
/// task waits. Senders never wait: they push into an unbounded channel in front of it.
const EVENT_BUFFER: usize = 256;
/// How many config change notices the subscription's output holds before forwarding waits.
const CONFIG_EVENT_BUFFER: usize = 8;
/// How often unsaved session changes are checked for a save that's come due.
const SAVE_TICK: Duration = Duration::from_secs(1);

/// Runs the app, or a Chromium helper process when CEF started this executable as one.
fn main() -> iced::Result {
    // Before anything creates a wgpu instance: CEF on Windows needs iced on the GL backend.
    browser::prefer_gl();
    // Chromium's renderer, GPU and utility processes are this executable too.
    if let Some(code) = macaroni_browser::helper_process() {
        std::process::exit(code);
    }
    fonts::load();
    let result = iced::application(App::new, App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(|app: &App| app.iced_theme.clone())
        .window(window_settings())
        // Close requests become `Message::CloseRequested`, so the session is saved first.
        .exit_on_close_request(false)
        .run();
    // After the event loop: `App::exit` has already closed every browser window.
    browser::shutdown();
    result
}

/// The main window's settings. On Windows macaroni draws its own title bar (`view::title_bar`),
/// so the window has no system frame, only the shadow and rounded corners.
fn window_settings() -> window::Settings {
    let settings = window::Settings {
        size: DEFAULT_WINDOW_SIZE,
        decorations: !view::CUSTOM_FRAME,
        // The taskbar's and Alt+Tab's icon. The exe's own (Explorer, pinned shortcuts) is the
        // .ico that build.rs embeds; both are drawn from the same `app-icon.svg`.
        icon: window::icon::from_file_data(APP_ICON, None).ok(),
        ..window::Settings::default()
    };
    #[cfg(windows)]
    let settings = window::Settings {
        platform_specific: window::settings::PlatformSpecific {
            undecorated_shadow: true,
            corner_preference: window::settings::platform::CornerPreference::Round,
            ..window::settings::PlatformSpecific::default()
        },
        ..settings
    };
    settings
}

/// The whole application state: the model, everything running for it, and the UI state around
/// it. Handlers live in `update/`, drawing in `view/`.
struct App {
    /// Workspaces, panes and tabs. Changed only through [`App::apply`].
    model: Model,
    /// Every terminal that has started, by session.
    running: BTreeMap<SessionId, Running>,
    /// Sessions the model asked for that haven't started yet. Each starts once its view has
    /// reported a size (and the event channel exists), so shells start at their real size.
    pending: BTreeMap<SessionId, Option<GridSize>>,
    /// Where sessions send their events; `None` until the subscription reports
    /// [`Message::EventsReady`]. Each session's callback holds a clone.
    events: Option<UnboundedSender<(SessionId, SessionEvent)>>,
    /// `macaroni [program [args...]]` runs a program in the first terminal instead of the shell.
    /// Taken by the first terminal that starts.
    first_command: Option<(String, Vec<String>)>,
    /// This run was started with a program: it didn't restore the session and won't save one.
    ran_program: bool,
    /// Where `config.toml` and `session.json` live.
    config_dir: PathBuf,
    /// The config in effect.
    config: Config,
    /// Problems with the config file (or with values in it), shown in a banner. Other errors
    /// (the control socket, saving the session) are added here too.
    config_errors: Vec<String>,
    /// Shortcuts in effect, for terminal views and keys nothing else handled.
    keymap: Keymap,
    /// The terminal colors from the theme.
    palette: Palette,
    /// The interface's colors and sizes, derived from the palette and the config.
    chrome: Chrome,
    /// The iced theme built from `chrome`, for built-in widgets.
    iced_theme: iced::Theme,
    /// The terminal font and its cell size, at the configured size plus `zoom`.
    metrics: Metrics,
    /// Drawn for terminals that haven't started yet.
    empty: Snapshot,
    /// The divider being dragged, if any.
    drag: Option<Drag>,
    /// Whether the window has keyboard focus (kept `true` while a browser page has it), for
    /// deciding whether a notification needs the user's attention.
    window_focused: bool,
    /// The item being renamed in place and the name typed so far.
    renaming: Option<(Renaming, String)>,
    /// The open right-click menu.
    menu: Option<Menu>,
    /// The open settings dialog.
    settings: Option<view::settings::Settings>,
    /// The open icon picker.
    icon_picker: Option<view::icon_picker::IconPicker>,
    /// The open workspace folder dialog.
    folder_dialog: Option<view::folder_dialog::FolderDialog>,
    /// The sidebar's dragged width and whether it's folded away (see `update/sidebar.rs`).
    sidebar: session::SidebarLayout,
    /// Whether the sidebar's edge is being dragged.
    sidebar_drag: bool,
    /// What's typed in the sidebar's search field; workspaces that don't match are hidden.
    sidebar_query: String,
    /// Whether the sidebar's search field has the keyboard (terminals don't, meanwhile).
    sidebar_search_focused: bool,
    /// The tab or workspace pressed and maybe being dragged.
    item_drag: Option<ItemDrag>,
    /// The tab under the cursor (its pane and position), for hover styles and drop positions.
    hovered_tab: Option<(PaneId, usize)>,
    /// The sidebar entry under the cursor, and since when: hovering a workspace while dragging
    /// a tab opens it.
    sidebar_hover: Option<(SidebarHover, Instant)>,
    /// Where the cursor is on the tab or sidebar item it's over (or last left, into a gap,
    /// while dragging), for where a dragged one would be inserted.
    hover_zone: pointer::Zone,
    /// The tab, workspace or group last clicked: Enter renames it, as in a macOS list. Typing
    /// anything else, or clicking anywhere else, lets go of it.
    item_focus: Option<Renaming>,
    /// Eased selection and hover levels of workspaces and tabs, retargeted by
    /// [`App::sync_fades`].
    fades: anim::Fades,
    /// Where each pane's tab strip is scrolled to and how wide it shows, as it last reported
    /// while its tabs overflowed, for [`App::reveal_tab`].
    tab_views: BTreeMap<PaneId, iced::widget::scrollable::Viewport>,
    /// Panes whose open tab changed and should be scrolled into view once the strip reports
    /// its next viewport (a new tab only widens the strip on the next layout).
    tab_reveals: Vec<PaneId>,
    /// The custom `#rrggbb` field on a workspace's, group's or tab's color menu.
    color_input: String,
    /// The window's size in logical pixels, for laying out menus and drop targets.
    window_size: Size,
    /// The open find bar.
    search: Option<Search>,
    /// The control API's connections, subscribers and endpoint.
    ipc: ipc::IpcState,
    /// For tabs rebuilt from the saved session: where to start and which agent to resume.
    /// Each entry is taken when its terminal starts.
    restored: BTreeMap<TabId, session::SavedTab>,
    /// When the session was last written, to save at most once a second.
    session_saved: Option<Instant>,
    /// The model changed since the session was last written.
    session_dirty: bool,
    /// Shells for the new-tab menu, the default first.
    shells: Vec<ShellOption>,
    /// VS Code's `code` command, for the new-tab menu, if installed.
    vscode: Option<PathBuf>,
    /// Shells picked for tabs whose terminals haven't started yet.
    tab_shells: BTreeMap<SessionId, ShellOption>,
    /// Terminals running an `[[ssh]]` profile, by its name, so the session saves them as SSH
    /// tabs and a restart connects them again.
    ssh_tabs: BTreeMap<SessionId, String>,
    /// SSH terminals whose connection ended with an error: they stay open to show it, and
    /// Enter connects again.
    disconnected: BTreeSet<SessionId>,
    /// Offer to install Git Bash (Windows, not installed, no shell configured).
    offer_git_bash: bool,
    /// Every browser tab's panel (address bar state, bounds, and its browser once started).
    browsers: BTreeMap<BrowserId, Panel>,
    /// VS Code in browser tabs: whether it's downloaded, and its server.
    vscode_web: vscode::WebVsCode,
    /// Where browsers send their events; `None` until [`Message::BrowserEventsReady`].
    browser_events: Option<UnboundedSender<(BrowserId, BrowserMessage)>>,
    /// The main window, once it has opened.
    window: Option<window::Id>,
    /// Whether the window is maximized, refreshed on every resize (no resize borders then).
    maximized: bool,
    /// The main window's native handle (`HWND`), the parent of every browser window.
    parent: Option<isize>,
    /// The window's scale factor, for placing browser windows in physical pixels.
    scale: f32,
    /// Points added to the configured font size by zooming, for this run only.
    zoom: f32,
    /// The browser holding keyboard focus, when a page rather than macaroni has it.
    page_focus: Option<BrowserId>,
    /// The focused tab as of the last update, to notice focus moving to or from a browser.
    last_focused_tab: Option<TabId>,
    /// The keymap, shared with browsers so shortcuts work while a page has focus.
    shared_keymap: Arc<Mutex<Keymap>>,
    /// Control API callers waiting for a page's text: connection and request id.
    text_requests: BTreeMap<u64, (ipc::ConnId, u64)>,
    /// The id of the last page-text request, to tell their answers apart.
    next_text_request: u64,
}

impl App {
    /// Builds the app at launch: reads the command line and the config, restores the saved
    /// session if enabled (and no program was given), and runs the model's first effects.
    ///
    /// Side effects: writes the config template on first run and queues the first sessions and
    /// browsers. The control socket and the session channel come later, from subscriptions.
    fn new() -> Self {
        let mut args = std::env::args().skip(1);
        let first_command = args.next().map(|program| (program, args.collect()));
        let config_dir = macaroni_config::config_dir();
        icons::set_custom_dir(config_dir.join("icons"));
        let (config, load_error) = match Config::load_or_init(&config_dir) {
            Ok(config) => (config, None),
            Err(err) => (Config::default(), Some(err.0)),
        };
        let saved = (config.session.restore && first_command.is_none())
            .then(|| session::load(&session::path(&config_dir)))
            .flatten();
        let sidebar = saved.as_ref().and_then(|s| s.sidebar);
        let (model, effects, restored) = restore::startup(saved);
        let mut app = Self::with_defaults(model, restored, first_command, config_dir);
        // Read from the loaded config, since `app.config` is still the default until `apply_config`.
        app.offer_git_bash =
            cfg!(windows) && config.shell.program.is_none() && shell::git_bash().is_none();
        app.apply_config(config);
        // After `apply_config`, which drops a dragged width when the configured one changes
        // (and the default config's differs from the real one).
        if let Some(sidebar) = sidebar {
            app.sidebar = sidebar;
            app.sync_sidebar();
        }
        // After `apply_config`, which replaces `config_errors` with the config's own problems.
        app.config_errors.extend(load_error);
        // Startup effects only queue sessions and browsers; none of them quits, so there's no
        // task to keep.
        let _ = app.run_effects(effects);
        app
    }

    /// The app around `model` with the default config, before the real one applies. Probes the
    /// installed shells and picks the control socket's endpoint, but starts nothing.
    fn with_defaults(
        model: Model,
        restored: BTreeMap<TabId, session::SavedTab>,
        first_command: Option<(String, Vec<String>)>,
        config_dir: PathBuf,
    ) -> Self {
        let defaults = Config::default();
        let palette = Palette::from_theme(&defaults.theme);
        let chrome = Chrome::new(&palette, &defaults);
        Self {
            model,
            running: BTreeMap::new(),
            pending: BTreeMap::new(),
            events: None,
            ran_program: first_command.is_some(),
            first_command,
            config_dir,
            iced_theme: chrome.iced_theme(&defaults.theme_name),
            config: defaults,
            config_errors: Vec::new(),
            keymap: Keymap::new(&[]).0,
            palette,
            chrome,
            metrics: Metrics::measure(
                fonts::PRIMARY,
                PLACEHOLDER_FONT_SIZE,
                false,
                macaroni_config::FontSettings::default().line_height,
            ),
            empty: Snapshot::default(),
            drag: None,
            window_focused: false,
            renaming: None,
            menu: None,
            settings: None,
            icon_picker: None,
            folder_dialog: None,
            sidebar: session::SidebarLayout::default(),
            sidebar_drag: false,
            sidebar_query: String::new(),
            sidebar_search_focused: false,
            item_drag: None,
            hovered_tab: None,
            sidebar_hover: None,
            hover_zone: pointer::Zone::Before,
            item_focus: None,
            fades: anim::Fades::default(),
            tab_views: BTreeMap::new(),
            tab_reveals: Vec::new(),
            color_input: String::new(),
            window_size: DEFAULT_WINDOW_SIZE,
            search: None,
            ipc: ipc::IpcState::new(macaroni_ipc::endpoint()),
            restored,
            session_saved: None,
            session_dirty: false,
            shells: shell::available(),
            vscode: shell::vscode(),
            tab_shells: BTreeMap::new(),
            ssh_tabs: BTreeMap::new(),
            disconnected: BTreeSet::new(),
            offer_git_bash: false,
            browsers: BTreeMap::new(),
            vscode_web: vscode::WebVsCode::new(),
            browser_events: None,
            window: None,
            maximized: false,
            parent: None,
            scale: 1.0,
            zoom: 0.0,
            page_focus: None,
            last_focused_tab: None,
            shared_keymap: Arc::new(Mutex::new(Keymap::new(&[]).0)),
            text_requests: BTreeMap::new(),
            next_text_request: 0,
        }
    }

    /// The window title: the focused tab's label, else the active workspace's name.
    fn title(&self) -> String {
        match self.model.focused_tab().label() {
            Some(title) => format!("{title} - {APP_NAME}"),
            None => format!("{} - {APP_NAME}", self.model.active_workspace().name),
        }
    }

    /// iced's update function: handles `message`, then brings the native browser windows and
    /// the fades in line with the new state.
    fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        // Both run after every message, whichever handler ran: browser windows aren't drawn by
        // iced, so any change to the model, a menu, a drag or the settings dialog may have to
        // move, hide or focus one, and the fades need the selection and hover as they are now.
        self.sync_browsers();
        self.sync_fades();
        task
    }

    /// Sets each workspace's and tab's fade targets (selected, hovered) from the current state.
    /// The view reads the eased levels, and `window::frames` runs while any is still moving.
    fn sync_fades(&mut self) {
        let active = self.model.active_workspace().id;
        let hovered_ws = match self.sidebar_hover {
            Some((SidebarHover::Workspace(ws), _)) => Some(ws),
            _ => None,
        };
        let model = &self.model;
        let hovered_tab = self.hovered_tab;
        let workspaces = model.workspaces().iter().map(|ws| {
            (
                anim::Item::Workspace(ws.id),
                ws.id == active,
                hovered_ws == Some(ws.id),
            )
        });
        let tabs = model
            .workspaces()
            .iter()
            .flat_map(|ws| ws.layout.leaves())
            .filter_map(|p| model.pane(p))
            .flat_map(|pane| {
                pane.tabs.iter().enumerate().map(move |(i, &tab)| {
                    (
                        anim::Item::Tab(tab),
                        tab == pane.active,
                        hovered_tab == Some((pane.id, i)),
                    )
                })
            });
        self.fades.sync(Instant::now(), workspaces.chain(tabs));
    }

    /// Hands each message to its area's handler in `update/`; a few that only set a field are
    /// handled inline. Returns the handler's task.
    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Command(command) => self.apply(command),
            Message::Commands(commands) => self.apply_all(commands),
            Message::Action(action) => self.perform(action),
            Message::Key(key, physical, mods) => self.unhandled_key(key, physical, mods),
            Message::Ipc(message) => self.ipc_message(message),

            Message::EventsReady(events) => self.events_ready(events),
            Message::Session(id, event) => self.session_event(id, event),
            Message::View(id, event) => self.view_event(id, event),
            Message::Paste(id, Some(text)) => self.paste(id, text),
            Message::Paste(_, None) => Task::none(),

            Message::ConfigChanged => self.config_changed(),
            Message::DismissConfigErrors => {
                self.config_errors.clear();
                Task::none()
            }
            Message::Settings(message) => self.settings_update(message),
            Message::IconPicker(message) => self.icon_picker_update(message),
            Message::Folder(message) => self.folder_update(message),
            Message::SettingsClose => {
                self.settings = None;
                Task::none()
            }

            Message::WindowOpened(id) => self.window_opened(id),
            Message::Window(op) => self.window_op(op),
            Message::WindowResized(size) => self.window_resized(size),
            Message::WindowFocused(focused) => self.set_window_focused(focused),
            Message::Maximized(maximized) => {
                self.maximized = maximized;
                Task::none()
            }
            Message::WindowHandle(handle) => {
                self.parent = handle;
                Task::none()
            }
            Message::Rescaled(scale) => {
                self.scale = scale;
                Task::none()
            }
            Message::ReclaimFocus => self.reclaim_focus(),
            Message::CloseRequested => self.close_requested(),

            Message::DragStart(workspace, divider) => self.divider_press(workspace, divider),
            Message::DragMove(position) if self.sidebar_drag => self.sidebar_drag_move(position.x),
            Message::DragMove(position) => self.divider_move(position),
            Message::DragEnd => {
                self.drag = None;
                self.sidebar_drag = false;
                Task::none()
            }
            Message::SidebarDragStart => self.sidebar_drag_start(),
            Message::SidebarSearch(query) => {
                self.sidebar_query = query;
                Task::none()
            }
            Message::SidebarSearchSubmit => self.sidebar_search_submit(),
            Message::SidebarSearchClear => {
                self.sidebar_query.clear();
                Task::none()
            }
            Message::SidebarSearchCheck => {
                iced::widget::operation::is_focused(view::SIDEBAR_SEARCH)
                    .map(Message::SidebarSearchFocused)
            }
            Message::SidebarSearchFocused(focused) => {
                self.sidebar_search_focused = focused;
                Task::none()
            }

            Message::TabPress(tab) => self.tab_press(tab),
            Message::WorkspacePress(ws) => self.workspace_press(ws),
            Message::ItemDragMove(at) => self.item_drag_move(at),
            Message::ItemDragEnd => self.item_drag_end(),
            Message::TabHover(pane, index, entered) => self.tab_hover(pane, index, entered),
            Message::TabZone(pane, index, zone) => self.tab_zone(pane, index, zone),
            Message::SidebarHover(hover, entered) => self.sidebar_hover(hover, entered),
            Message::SidebarZone(hover, zone) => self.sidebar_zone(hover, zone),
            Message::DragTick => self.drag_tick(),
            Message::TabScroll(pane, delta) => self.tab_scroll(pane, delta),
            Message::TabViewport(pane, viewport) => self.tab_viewport(pane, viewport),
            Message::Frame => Task::none(),
            Message::SaveTick => {
                self.save_session(false);
                Task::none()
            }

            Message::RenameInput(value) => self.rename_input(value),
            Message::RenameCommit => self.rename_commit(),
            Message::RenameCancel(target) => self.rename_cancel(target),
            Message::ItemBlur(target) => self.item_blur(target),

            Message::MenuOpen(target, at) => self.menu_open(target, at),
            Message::MenuHover(page) => self.menu_hover(page),
            Message::MenuPin(page) => self.menu_pin(page),
            Message::MenuClose => {
                self.menu = None;
                Task::none()
            }
            Message::Dismiss => self.dismiss(),
            Message::ColorInput(value) => {
                self.color_input = value;
                Task::none()
            }
            Message::ColorSubmit(target) => self.color_submit(target),
            Message::Tab(tab, action) => self.tab_menu(tab, action),
            Message::Workspace(ws, action) => self.workspace_menu(ws, action),
            Message::Group(group, action) => self.group_menu(group, action),
            Message::NewTabWith(pane, index) => self.new_tab_with(pane, index),
            Message::NewSshTab(pane, index) => self.new_ssh_tab(pane, index),
            Message::NewBrowserIn(pane) => self.new_browser_in(pane),
            Message::OpenVsCode(pane) => self.open_vscode_in(pane),
            Message::VsCodeTab(pane) => self.vscode_tab(pane),
            Message::VsCodeReady(listening) => self.vscode_ready(listening),
            Message::VsCodeInstalled(result) => self.vscode_installed(result),
            Message::InstallGitBash => self.install_git_bash(),
            Message::DismissShellOffer => {
                self.offer_git_bash = false;
                Task::none()
            }

            Message::SearchInput(query) => self.search_input(query),
            Message::SearchStep(direction) => self.search_step(direction),
            Message::SearchClose => self.search_close(),

            Message::BrowserEventsReady(events) => {
                self.browser_events = Some(events);
                Task::none()
            }
            Message::Browser(id, message) => self.browser_message(id, message),
            Message::BrowserBounds(id, bounds) => self.browser_bounds(id, bounds),
            Message::AddressInput(id, text) => self.address_input(id, text),
            Message::AddressSubmit(id) => self.address_submit(id),
            Message::Nav(id, nav) => self.nav(id, nav),
        }
    }

    /// The focused tab's session, or `None` when a browser tab is focused.
    fn focused_session(&self) -> Option<SessionId> {
        match self.model.focused_tab().kind {
            TabKind::Terminal(session) => Some(session),
            TabKind::Browser(_) => None,
        }
    }

    /// Everything the app listens to. The long-lived streams (sessions, config, control socket,
    /// browsers, window events) always run; the rest only while they're needed. iced rebuilds
    /// the set after every update, so a listener is added or dropped as the state changes.
    fn subscription(&self) -> Subscription<Message> {
        let drag = if self.drag.is_some() || self.sidebar_drag {
            event::listen_with(drag_events)
        } else {
            Subscription::none()
        };
        let item_drag = if self.item_drag.is_some() {
            event::listen_with(item_drag_events)
        } else {
            Subscription::none()
        };
        // Only while a dragged tab hovers a workspace, to open it after a moment
        // (`WORKSPACE_HOVER_OPEN`, checked on each frame).
        let tick = if self
            .item_drag
            .is_some_and(|d| d.dragging && d.tab().is_some())
            && matches!(self.sidebar_hover, Some((SidebarHover::Workspace(_), _)))
        {
            window::frames().map(|_| Message::DragTick)
        } else {
            Subscription::none()
        };
        // `save_session` does nothing for a one-off program run or with restore off, so the
        // flag would never clear: no ticks then.
        let save = if self.session_dirty && self.config.session.restore && !self.ran_program {
            Subscription::run(save_ticks)
        } else {
            Subscription::none()
        };
        let frames = if self.fades.animating() {
            window::frames().map(|_| Message::Frame)
        } else {
            Subscription::none()
        };
        let search = if self.search.is_some() {
            event::listen_with(search_keys)
        } else if self.settings.is_some() {
            event::listen_with(settings_keys)
        } else {
            Subscription::none()
        };
        // Only while there's something to dismiss: otherwise Escape belongs to the terminal.
        let dismiss = if self.menu.is_some()
            || self.renaming.is_some()
            || self.item_drag.is_some()
            || self.settings.is_some()
            || self.icon_picker.is_some()
            || self.folder_dialog.is_some()
            || self.sidebar_search_focused
        {
            event::listen_with(escape_keys)
        } else {
            Subscription::none()
        };
        // A page is another native window, so clicking macaroni's UI doesn't take the keyboard
        // back from it by itself.
        let reclaim = if self.page_focus.is_some() {
            event::listen_with(clicks)
        } else {
            Subscription::none()
        };
        // Terminals look up shortcuts themselves; a browser tab's pane has nothing that does.
        // Only `Ignored` events count (see `unhandled_keys`), so typing in a text field isn't
        // taken for a shortcut.
        let shortcuts = if self.focused_session().is_none() {
            event::listen_with(unhandled_keys)
        } else {
            Subscription::none()
        };
        // Only while VS Code's server is starting: tells the app once it listens.
        let vscode = match &self.vscode_web.server {
            Some(server) if !self.vscode_web.ready => {
                Subscription::run_with(server.port, vscode_listening)
            }
            _ => Subscription::none(),
        };
        Subscription::batch([
            vscode,
            search,
            dismiss,
            Subscription::run(session_events),
            Subscription::run(config_events),
            Subscription::run(ipc::events),
            Subscription::run(browser_events),
            window::open_events().map(Message::WindowOpened),
            reclaim,
            shortcuts,
            iced::window::close_requests().map(|_| Message::CloseRequested),
            event::listen_with(window_focus_events),
            // Clicking the search field focuses it and clicking anything else blurs it, and
            // neither reaches the app any other way.
            event::listen_with(presses),
            drag,
            item_drag,
            tick,
            frames,
            save,
        ])
    }
}

/// Cursor moves and the left button's release, while a divider is dragged.
fn drag_events(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::DragMove(position)),
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => Some(Message::DragEnd),
        _ => None,
    }
}

/// Cursor moves and the left button's release, while a tab or workspace is pressed or dragged.
fn item_drag_events(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::ItemDragMove(position))
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::ItemDragEnd)
        }
        _ => None,
    }
}

/// Escape, while the find bar is open: closes it.
fn search_keys(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::SearchClose),
        _ => None,
    }
}

/// Ctrl+F (Cmd+F on macOS), while the settings dialog is open: focuses its search field.
fn settings_keys(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. })
            if modifiers.command() && key.as_ref() == iced::keyboard::Key::Character("f") =>
        {
            Some(Message::Settings(
                view::settings::SettingsMessage::FocusSearch,
            ))
        }
        _ => None,
    }
}

/// Escape, while a menu, rename, item drag or the settings dialog is open: dismisses it.
fn escape_keys(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(Message::Dismiss),
        _ => None,
    }
}

/// Any mouse press, for [`Message::SidebarSearchCheck`].
fn presses(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::ButtonPressed(_)) => Some(Message::SidebarSearchCheck),
        _ => None,
    }
}

/// Key presses no widget captured, while a browser tab is focused: they may be shortcuts.
fn unhandled_keys(event: Event, status: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key,
            physical_key,
            modifiers,
            ..
        }) if status == event::Status::Ignored => Some(Message::Key(key, physical_key, modifiers)),
        _ => None,
    }
}

/// Any mouse press in the window, while a page has keyboard focus.
fn clicks(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::ButtonPressed(_)) => Some(Message::ReclaimFocus),
        _ => None,
    }
}

/// The window gaining or losing focus, being rescaled, or being resized.
fn window_focus_events(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    match event {
        Event::Window(window::Event::Focused) => Some(Message::WindowFocused(true)),
        Event::Window(window::Event::Unfocused) => Some(Message::WindowFocused(false)),
        Event::Window(window::Event::Rescaled(scale)) => Some(Message::Rescaled(scale)),
        Event::Window(window::Event::Resized(size)) => Some(Message::WindowResized(size)),
        _ => None,
    }
}

/// The channel every terminal session reports on: first hands its sender to the app
/// ([`Message::EventsReady`]), then forwards each event as [`Message::Session`].
fn session_events() -> impl Stream<Item = Message> {
    iced::stream::channel(EVENT_BUFFER, async |mut output| {
        let (sender, mut receiver) = unbounded();
        let _ = output.send(Message::EventsReady(sender)).await;
        while let Some((id, event)) = receiver.next().await {
            let _ = output.send(Message::Session(id, event)).await;
        }
    })
}

/// [`Message::SaveTick`] every [`SAVE_TICK`], from a thread that stops once the subscription
/// is dropped (the send fails). The default executor has no timers, hence the thread.
fn save_ticks() -> impl Stream<Item = Message> {
    iced::stream::channel(1, async |mut output| {
        let (sender, mut receiver) = unbounded();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(SAVE_TICK);
                if sender.unbounded_send(()).is_err() {
                    break;
                }
            }
        });
        while receiver.next().await.is_some() {
            let _ = output.send(Message::SaveTick).await;
        }
    })
}

/// [`Message::VsCodeReady`] once VS Code's server listens on `port`, or gives up.
fn vscode_listening(port: &u16) -> impl Stream<Item = Message> + use<> {
    let port = *port;
    iced::stream::channel(1, async move |mut output| {
        let listening = vscode::off_thread(move || vscode::wait_listening(port)).await;
        let _ = output.send(Message::VsCodeReady(listening)).await;
    })
}

/// The channel every browser reports on: first hands its sender to the app
/// ([`Message::BrowserEventsReady`]), then forwards each message as [`Message::Browser`].
fn browser_events() -> impl Stream<Item = Message> {
    iced::stream::channel(EVENT_BUFFER, async |mut output| {
        let (sender, mut receiver) = unbounded();
        let _ = output.send(Message::BrowserEventsReady(sender)).await;
        while let Some((id, message)) = receiver.next().await {
            let _ = output.send(Message::Browser(id, message)).await;
        }
    })
}

/// The window's `HWND` on Windows, which browsers are parented to; `None` on other platforms
/// or if the handle isn't available.
fn native_handle(window: &dyn window::Window) -> Option<isize> {
    use window::raw_window_handle::RawWindowHandle;
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Sends [`Message::ConfigChanged`] whenever the config directory changes. The watcher lives
/// as long as the stream.
fn config_events() -> impl Stream<Item = Message> {
    iced::stream::channel(CONFIG_EVENT_BUFFER, async |mut output| {
        let (sender, mut receiver) = unbounded();
        let _watcher = macaroni_config::watch(&macaroni_config::config_dir(), move || {
            let _ = sender.unbounded_send(());
        });
        while receiver.next().await.is_some() {
            let _ = output.send(Message::ConfigChanged).await;
        }
    })
}

/// Opens a file with the system's handler for it (on macOS, the default text editor). Spawns a
/// process and doesn't wait for it; failures are ignored.
fn open_in_editor(path: &std::path::Path) {
    let mut command = if cfg!(windows) {
        std::process::Command::new("explorer")
    } else if cfg!(target_os = "macos") {
        let mut open = std::process::Command::new("open");
        open.arg("-t");
        open
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = command.arg(path).spawn();
}

/// The user's home directory, from `USERPROFILE` on Windows and `HOME` elsewhere.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Runs VS Code's `code` command on `dir` (or with no folder). Spawns a process and doesn't
/// wait for it; failures are ignored.
fn open_vscode(code: &std::path::Path, dir: Option<PathBuf>) {
    let mut command = std::process::Command::new(code);
    command.args(dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `code` is a batch file: without this a console window flashes up.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command.spawn();
}

/// Opens an http(s) URL in the default browser. Other schemes are refused: a terminal link can
/// come from any program's output, and `file:` or custom handlers could launch programs.
/// Spawns the system's opener and doesn't wait for it.
fn open_url(url: &str) {
    if !is_openable(url) {
        return;
    }
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = command.arg(url).spawn();
}
