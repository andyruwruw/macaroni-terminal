//! The agent setup tab: Claude Code's and Codex's setup, the user's own and the tab's
//! project's, in one list (instructions, settings, skills, subagents, commands, hooks, MCP
//! servers) beside the selected item's text, with buttons to open, create or show its file.
//! The agents' folders (`~/.claude`, `~/.codex`, the project's `.claude` and `.codex`) are
//! listed first, each with buttons to open it as a workspace, in a terminal, or in VS Code.
//!
//! The model holds the kind and the project folder ([`TabKind::AgentSetup`], [`Tab::cwd`]);
//! what was read is the app's, in `App.agent_tabs`, from [`crate::agent_setup::scan`] run off
//! the UI thread. Opening the tab reads it, and `Message::AgentTick` reads every agent setup
//! tab on screen again every [`AGENT_TICK`], so edits made elsewhere show up.
//!
//! [`Tab::cwd`]: macaroni_core::Tab::cwd

use std::path::{Path, PathBuf};

use iced::widget::{Space, button, column, container, row, text_editor};
use iced::{Element, Fill, Task, alignment};
use macaroni_core::{BrowserId, Command, PaneId, TabId, TabKind};

use super::git_tab::{centered, error_banner, separator, small_button, tooltip_button};
use super::widgets::{flat_button, icon_button, one_line, thin_scroll};
use crate::agent_setup::{Folder, Item, Scope, Section, Setup, Tool};
use crate::icons::{Icon, icon};
use crate::{App, Message};

/// How often agent setup tabs on screen are read again.
pub const AGENT_TICK: std::time::Duration = std::time::Duration::from_secs(5);

/// The top bar's height.
const BAR_HEIGHT: f32 = 44.0;

/// The list's share of the tab's width, and its limits.
const LIST_SHARE: f32 = 0.38;
/// The narrowest the list gets.
const LIST_MIN: f32 = 220.0;
/// The widest the list gets.
const LIST_MAX: f32 = 380.0;

/// An item row's height.
const ROW_HEIGHT: f32 = 40.0;

/// The size of names and body text.
const TEXT_SIZE: f32 = 12.5;

/// The size of details and headers.
const SMALL_SIZE: f32 = 11.0;

/// The size of icons in rows.
const ICON_SIZE: f32 = 14.0;

/// A section header's text size.
const HEADER_SIZE: f32 = 12.0;

/// The text size of an item's Personal or Project badge.
const BADGE_SIZE: f32 = 10.0;

/// How strongly a badge's color fills it, and how strongly it draws its edge.
const BADGE_TINT: f32 = 0.14;
/// See [`BADGE_TINT`].
const BADGE_EDGE: f32 = 0.45;

/// The room between the list and its scrollbar, which sits beside the rows, not over them.
const SCROLLBAR_GAP: f32 = 2.0;

/// The body's text size relative to the terminal's font size.
const BODY_SCALE: f32 = 0.9;

/// One agent setup tab's state, in `App.agent_tabs`.
#[derive(Debug, Default)]
pub struct AgentTab {
    /// What was read last; `None` until the first read finishes.
    setup: Option<Setup>,
    /// A read is running.
    loading: bool,
    /// The selected item's [`Item::key`].
    selected: Option<String>,
    /// VS Code's `code` command, found with the last read.
    vscode: Option<PathBuf>,
    /// The last failure (creating a file), shown until closed.
    error: Option<String>,
    /// The selected item's text in a read-only editor, so it can be selected and copied: the
    /// item's key, a hash of the text, and the editor.
    text: Option<(String, u64, text_editor::Content)>,
    /// A Markdown item shows its source instead of its rendered page.
    source: bool,
    /// The page rendered Markdown shows in, made the first time one is shown.
    page: Option<BrowserId>,
    /// What the page shows.
    published: Option<PageKey>,
    /// What's being rendered for the page, so a read meanwhile doesn't start it again.
    rendering: Option<PageKey>,
    /// Why the page couldn't start; Markdown then shows as its source.
    page_error: Option<String>,
}

/// What an agent setup tab's page was rendered from: the item's key, a hash of its text, and
/// whether its code was highlighted for a dark background.
type PageKey = (String, u64, bool);

/// A Markdown item's page body, rendered off the UI thread.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// What it was rendered from.
    key: PageKey,
    /// The item's file, which the page's links are relative to.
    path: PathBuf,
    /// The page's body.
    html: String,
}

/// What an agent setup tab's controls send, wrapped in `Message::AgentSetup`.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    /// Read the setup again (opening, the tick, the reload button, F5).
    Refresh,
    /// A read finished: the setup and VS Code's command.
    Loaded(Box<(Setup, Option<PathBuf>)>),
    /// Select an item by its key.
    Select(String),
    /// Open a file in VS Code or the plain text editor.
    Open(PathBuf),
    /// Create an empty instructions file and open it.
    Create(PathBuf),
    /// Show a file or folder in the system's file manager.
    Reveal(PathBuf),
    /// A new workspace at a folder.
    Workspace(PathBuf),
    /// A new terminal tab at a folder, beside this tab.
    Terminal(PathBuf),
    /// Open a folder in VS Code.
    VsCode(PathBuf),
    /// Close the error banner.
    DismissError,
    /// Something done in the text's editor; edits are dropped, it only reads.
    Edit(text_editor::Action),
    /// Show a Markdown item's source (`true`) or its rendered page.
    Source(bool),
    /// A Markdown item's page body is ready.
    Rendered(Box<Rendered>),
    /// A link clicked in the rendered page.
    Link(String),
}

impl App {
    /// Opens an agent setup tab in `pane` for the project its folder is in, and reads it.
    pub(crate) fn open_agent_setup_tab(&mut self, pane: PaneId) -> Task<Message> {
        self.menu = None;
        let home = crate::home_dir();
        let folder =
            crate::agent_setup::project_of(self.pane_dir(pane).as_deref(), home.as_deref())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
        let mut tasks = Vec::new();
        if self.model.pane(pane).is_some() {
            tasks.push(self.apply(Command::FocusPane(pane)));
        }
        tasks.push(self.apply(Command::OpenAgentSetup {
            folder,
            split: None,
        }));
        let tab = self.model.focused_tab().id;
        tasks.push(self.agent_update(tab, AgentMessage::Refresh));
        Task::batch(tasks)
    }

    /// The agent setup tabs on screen, in any window.
    pub(crate) fn shown_agent_tabs(&self) -> Vec<TabId> {
        self.model
            .all_workspaces()
            .iter()
            .flat_map(|ws| ws.shown_panes(macaroni_core::Rect::UNIT))
            .filter_map(|(pane, _)| self.model.pane(pane).map(|p| p.active))
            .filter(|tab| {
                self.model
                    .tab(*tab)
                    .is_some_and(|t| t.kind == TabKind::AgentSetup)
                    && self.model.tab_visible(*tab)
            })
            .collect()
    }

    /// A key nothing captured while agent setup tab `tab` has the keyboard: Up and Down move
    /// through the items, Enter opens (or creates) the selected one's file, F5 reloads. `None`
    /// when the key isn't one of these.
    pub(crate) fn agent_key(
        &mut self,
        tab: TabId,
        key: &iced::keyboard::Key,
    ) -> Option<Task<Message>> {
        use iced::keyboard::Key::Named;
        use iced::keyboard::key::Named as N;
        let state = self.agent_tabs.get(&tab)?;
        let items = &state.setup.as_ref()?.items;
        let at = state
            .selected
            .as_ref()
            .and_then(|key| items.iter().position(|i| &i.key() == key));
        let message = match key {
            Named(N::ArrowDown) | Named(N::ArrowUp) => {
                let down = *key == Named(N::ArrowDown);
                let next = match at {
                    None => 0,
                    Some(at) if down => (at + 1).min(items.len().saturating_sub(1)),
                    Some(at) => at.saturating_sub(1),
                };
                AgentMessage::Select(items.get(next)?.key())
            }
            Named(N::Enter) => {
                let item = &items[at?];
                if item.exists {
                    AgentMessage::Open(item.path.clone())
                } else {
                    AgentMessage::Create(item.path.clone())
                }
            }
            Named(N::F5) => AgentMessage::Refresh,
            _ => return None,
        };
        Some(self.agent_update(tab, message))
    }

    /// Applies an agent setup tab's message.
    pub(crate) fn agent_update(&mut self, tab: TabId, message: AgentMessage) -> Task<Message> {
        let Some(project) = self
            .model
            .tab(tab)
            .filter(|t| t.kind == TabKind::AgentSetup)
            .map(|t| t.cwd.clone().map(PathBuf::from))
        else {
            return Task::none();
        };
        let state = self.agent_tabs.entry(tab).or_default();
        match message {
            AgentMessage::Refresh => {
                if state.loading {
                    return Task::none();
                }
                state.loading = true;
                let home = crate::home_dir();
                Task::perform(
                    crate::vscode::off_thread(move || {
                        let setup = crate::agent_setup::scan(home.as_deref(), project.as_deref());
                        (setup, macaroni_term::shell::vscode())
                    }),
                    move |found| Message::AgentSetup(tab, AgentMessage::Loaded(Box::new(found))),
                )
            }
            AgentMessage::Loaded(found) => {
                let (setup, vscode) = *found;
                state.loading = false;
                let still_there = state
                    .selected
                    .as_ref()
                    .is_some_and(|key| setup.items.iter().any(|i| &i.key() == key));
                if !still_there {
                    state.selected = setup
                        .items
                        .iter()
                        .find(|i| i.exists)
                        .or(setup.items.first())
                        .map(Item::key);
                }
                state.setup = Some(setup);
                state.vscode = vscode;
                self.agent_sync(tab)
            }
            AgentMessage::Select(key) => {
                state.selected = Some(key);
                self.agent_sync(tab)
            }
            AgentMessage::Open(path) => {
                crate::open_text_file(&path, None, None);
                Task::none()
            }
            AgentMessage::Create(path) => {
                let made = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| {
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&path)
                            .map(drop)
                    });
                match made {
                    // Created now, or by someone else since the last read: open it either way.
                    Ok(()) => crate::open_text_file(&path, None, None),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        crate::open_text_file(&path, None, None);
                    }
                    Err(e) => {
                        state.error = Some(format!("Couldn't create {}: {e}", path.display()));
                    }
                }
                self.agent_update(tab, AgentMessage::Refresh)
            }
            AgentMessage::Reveal(path) => {
                reveal(&path);
                Task::none()
            }
            AgentMessage::Workspace(path) => {
                let created = self.apply(Command::NewWorkspace);
                let ws = self.model.active_workspace().id;
                Task::batch([
                    created,
                    self.apply(Command::RenameWorkspace(ws, short_path(&path))),
                    // In the same update, so its first terminal starts there.
                    self.apply(Command::SetWorkspaceCwd(
                        ws,
                        Some(path.to_string_lossy().into_owned()),
                    )),
                ])
            }
            AgentMessage::Terminal(path) => {
                let mut tasks = Vec::new();
                if let Some(pane) = self.model.pane_of_tab(tab) {
                    tasks.push(self.apply(Command::FocusPane(pane)));
                }
                tasks.push(self.apply(Command::NewTab));
                // A terminal starts where a restored tab was, so it starts in the folder.
                let new = self.model.focused_tab().id;
                self.restored.insert(
                    new,
                    crate::session::SavedTab {
                        cwd: Some(path.to_string_lossy().into_owned()),
                        ..crate::session::SavedTab::default()
                    },
                );
                Task::batch(tasks)
            }
            AgentMessage::VsCode(path) => {
                if let Some(code) = &state.vscode {
                    crate::open_vscode(code, Some(path));
                }
                Task::none()
            }
            AgentMessage::Edit(action) => {
                if !action.is_edit()
                    && let Some((_, _, content)) = &mut state.text
                {
                    content.perform(action);
                }
                Task::none()
            }
            AgentMessage::Source(source) => {
                state.source = source;
                self.agent_sync(tab)
            }
            AgentMessage::Rendered(rendered) => {
                if state.rendering.as_ref() != Some(&rendered.key) {
                    return Task::none();
                }
                state.rendering = None;
                self.agent_publish(tab, *rendered);
                Task::none()
            }
            AgentMessage::Link(link) => {
                let Some(document) = selected_item(state).map(|i| i.path.clone()) else {
                    return Task::none();
                };
                self.open_page_link(tab, &document, &link)
            }
            AgentMessage::DismissError => {
                state.error = None;
                Task::none()
            }
        }
    }

    /// An agent setup tab's content, `width` wide; right-clicking opens the tab's menu.
    pub(crate) fn agent_body(&self, tab: TabId, width: f32) -> Element<'_, Message> {
        crate::right_click::RightClick::new(self.agent_content(tab, width), move |at| {
            Message::MenuOpen(crate::state::MenuTarget::Tab(tab), at)
        })
        .into()
    }

    /// Brings tab `tab`'s text and page up to date with the selected item: a new editor when
    /// the item or its text changed, and for Markdown shown rendered, a render off the UI
    /// thread when the page shows something else (or was highlighted for the other
    /// background), or a restyle when only the app's look changed.
    fn agent_sync(&mut self, tab: TabId) -> Task<Message> {
        let look = self.preview_look();
        let installed: Vec<_> = self.viewers.installed.iter().copied().collect();
        let Some(state) = self.agent_tabs.get_mut(&tab) else {
            return Task::none();
        };
        let Some(item) = selected_item(state).filter(|i| i.exists) else {
            state.text = None;
            return Task::none();
        };
        let (key, path, body) = (item.key(), item.path.clone(), item.body.clone());
        let hash = text_hash(&body);
        if state
            .text
            .as_ref()
            .is_none_or(|(k, h, _)| *k != key || *h != hash)
        {
            state.text = Some((key.clone(), hash, text_editor::Content::with_text(&body)));
        }
        if !is_markdown(&path)
            || state.source
            || state.page_error.is_some()
            || !crate::preview::AVAILABLE
        {
            return Task::none();
        }
        let want = (key, hash, look.dark);
        if state.published.as_ref() == Some(&want) {
            if let (Some(id), Some(server)) = (state.page, crate::preview::server::running())
                && server.look(id).is_some_and(|old| old != look)
            {
                server.restyle(id, look);
            }
            return Task::none();
        }
        if state.rendering.as_ref() == Some(&want) {
            return Task::none();
        }
        state.rendering = Some(want.clone());
        let render = crate::preview::html::Render {
            dark: look.dark,
            installed,
        };
        Task::perform(
            crate::vscode::off_thread(move || crate::preview::html::markdown(&body, &render)),
            move |html| {
                Message::AgentSetup(
                    tab,
                    AgentMessage::Rendered(Box::new(Rendered {
                        key: want,
                        path,
                        html,
                    })),
                )
            },
        )
    }

    /// Shows a rendered Markdown item in tab `tab`'s page, making the page the first time.
    /// Where the preview server can't start, the item shows as its source and says why.
    fn agent_publish(&mut self, tab: TabId, rendered: Rendered) {
        let look = self.preview_look();
        let installed = self.viewers.installed.iter().copied().collect();
        let server = crate::preview::server::get();
        let Some(state) = self.agent_tabs.get_mut(&tab) else {
            return;
        };
        let server = match server {
            Ok(server) => server,
            Err(error) => {
                state.page_error = Some(error);
                return;
            }
        };
        let id = *state.page.get_or_insert_with(super::file_tab::next_page);
        self.browsers
            .entry(id)
            .or_insert_with(|| crate::browser::Panel::for_file(tab));
        server.publish(
            id,
            crate::preview::Page {
                document: rendered.path,
                body: rendered.html,
                look,
                installed,
                whole: false,
            },
        );
        state.published = Some(rendered.key);
    }

    /// The page agent setup tab `tab` shows right now: the selected item is Markdown, shown
    /// rendered, and the page holds its current text and hasn't failed to start.
    pub(crate) fn agent_page_shown(&self, tab: TabId) -> Option<BrowserId> {
        let state = self.agent_tabs.get(&tab)?;
        let item = selected_item(state).filter(|i| i.exists && is_markdown(&i.path))?;
        let (key, hash, _) = state.text.as_ref()?;
        let (shown_key, shown_hash, _) = state.published.as_ref()?;
        if state.source || item.key() != *key || shown_key != key || shown_hash != hash {
            return None;
        }
        let id = state.page?;
        self.browsers
            .get(&id)
            .is_some_and(|p| p.error.is_none())
            .then_some(id)
    }

    /// The selected text of agent setup tab `tab`'s source, if any.
    pub(crate) fn agent_selection(&self, tab: TabId) -> Option<String> {
        self.agent_tabs.get(&tab)?.text.as_ref()?.2.selection()
    }

    /// What [`App::agent_body`] shows.
    fn agent_content(&self, tab: TabId, width: f32) -> Element<'_, Message> {
        let c = self.chrome;
        let send = move |m: AgentMessage| Message::AgentSetup(tab, m);
        let Some((state, setup)) = self
            .agent_tabs
            .get(&tab)
            .and_then(|s| Some((s, s.setup.as_ref()?)))
        else {
            return centered(c, "Reading the agents' setup...");
        };
        let selected = state
            .selected
            .as_ref()
            .and_then(|key| setup.items.iter().find(|i| &i.key() == key));
        let list_width = (width * LIST_SHARE).clamp(LIST_MIN, LIST_MAX);
        let rule = container(Space::new().width(1).height(Fill)).style(move |_| container::Style {
            background: Some(c.separator.into()),
            ..container::Style::default()
        });
        let mut body = column![self.agent_bar(tab, setup), separator(c)];
        if let Some(error) = &state.error {
            body = body.push(error_banner(c, error, send(AgentMessage::DismissError)));
        }
        body.push(
            row![
                container(self.agent_list(tab, state, setup)).width(list_width),
                rule,
                self.agent_detail(tab, state, selected),
            ]
            .height(Fill),
        )
        .into()
    }

    /// The top bar: what the tab is for, the project, and reload.
    fn agent_bar<'a>(&self, tab: TabId, setup: &'a Setup) -> Element<'a, Message> {
        let c = self.chrome;
        let project = match &setup.project {
            Some(project) => format!("Yours, and {}'s", short_path(project)),
            None => "Yours (this folder isn't a project)".to_owned(),
        };
        container(
            row![
                icon(Icon::Bot, 16.0, c.label),
                column![
                    one_line("Agent setup", TEXT_SIZE, c.text),
                    one_line(project, SMALL_SIZE, c.muted),
                ]
                .spacing(1)
                .width(Fill),
                icon_button(
                    c,
                    Icon::Reload,
                    Message::AgentSetup(tab, AgentMessage::Refresh)
                ),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center),
        )
        .height(BAR_HEIGHT)
        .padding([0, 12])
        .align_y(alignment::Vertical::Center)
        .into()
    }

    /// The list: the agents' folders, then each section's items.
    fn agent_list<'a>(
        &self,
        tab: TabId,
        state: &'a AgentTab,
        setup: &'a Setup,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let mut list = column![].spacing(1).padding([4, 0]);
        list = list.push(header(c, "Folders", setup.folders.len(), true));
        if setup.folders.is_empty() {
            list = list.push(none_line(c, "No .claude or .codex folder yet"));
        }
        for folder in &setup.folders {
            list = list.push(self.folder_row(tab, state, folder));
        }
        for section in Section::ALL {
            let items: Vec<&Item> = setup
                .items
                .iter()
                .filter(|i| i.section == section)
                .collect();
            list = list.push(header(c, section.title(), items.len(), false));
            if items.is_empty() {
                list = list.push(none_line(c, "None"));
            }
            for item in items {
                let on = state.selected.as_ref() == Some(&item.key());
                let badge = match item.scope {
                    Scope::Personal => rgb(self.palette.ansi[4]),
                    Scope::Project => rgb(self.palette.ansi[2]),
                };
                list = list.push(item_row(c, tab, item, on, badge));
            }
        }
        thin_scroll(c, list)
            .spacing(SCROLLBAR_GAP)
            .height(Fill)
            .into()
    }

    /// A folder's row: where it is, whose, and its buttons.
    fn folder_row<'a>(
        &self,
        tab: TabId,
        state: &'a AgentTab,
        folder: &'a Folder,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: AgentMessage| Some(Message::AgentSetup(tab, m));
        let path = folder.path.clone();
        let mut buttons = row![
            tooltip_button(
                c,
                Icon::Plus,
                "Open as a workspace",
                send(AgentMessage::Workspace(path.clone())),
            ),
            tooltip_button(
                c,
                Icon::Terminal,
                "Open in a terminal",
                send(AgentMessage::Terminal(path.clone())),
            ),
        ];
        if state.vscode.is_some() {
            buttons = buttons.push(tooltip_button(
                c,
                Icon::Code,
                "Open in VS Code",
                send(AgentMessage::VsCode(path.clone())),
            ));
        }
        buttons = buttons.push(tooltip_button(
            c,
            Icon::Folder,
            "Show in the file manager",
            send(AgentMessage::Reveal(path)),
        ));
        row![
            container(icon(tool_icon(folder.tool), ICON_SIZE, c.label)).padding([0, 2]),
            column![
                one_line(short_path(&folder.path), TEXT_SIZE, c.text),
                one_line(
                    format!("{} \u{b7} {}", folder.scope.name(), folder.tool.name()),
                    SMALL_SIZE,
                    c.muted
                ),
            ]
            .spacing(1)
            .width(Fill)
            .clip(true),
            buttons,
        ]
        .spacing(8)
        .padding([2, 10])
        .align_y(alignment::Vertical::Center)
        .into()
    }

    /// The selected item: its name, whose and where, its buttons, and its text.
    fn agent_detail<'a>(
        &self,
        tab: TabId,
        state: &'a AgentTab,
        item: Option<&'a Item>,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: AgentMessage| Some(Message::AgentSetup(tab, m));
        let Some(item) = item else {
            return centered(c, "Select something on the left to see it here.");
        };
        let mut buttons = row![].spacing(6);
        // Only where it can show rendered: without a page, Markdown shows as its source.
        if item.exists
            && is_markdown(&item.path)
            && crate::preview::AVAILABLE
            && state.page_error.is_none()
        {
            buttons = buttons.push(small_button(
                c,
                if state.source { "Rendered" } else { "Source" },
                false,
                send(AgentMessage::Source(!state.source)),
            ));
        }
        if item.exists {
            buttons = buttons.push(small_button(
                c,
                "Open",
                false,
                send(AgentMessage::Open(item.path.clone())),
            ));
            buttons = buttons.push(small_button(
                c,
                "Show in folder",
                false,
                send(AgentMessage::Reveal(item.path.clone())),
            ));
        } else {
            buttons = buttons.push(small_button(
                c,
                "Create",
                false,
                send(AgentMessage::Create(item.path.clone())),
            ));
        }
        let heading = column![
            row![
                icon(tool_icon(item.tool), ICON_SIZE, c.label),
                one_line(item.name.clone(), TEXT_SIZE + 1.5, c.text).width(Fill),
                buttons,
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
            one_line(
                format!(
                    "{} \u{b7} {} \u{b7} {}",
                    item.scope.name(),
                    item.tool.name(),
                    item.path.display()
                ),
                SMALL_SIZE,
                c.muted
            ),
        ]
        .spacing(4)
        .padding([10, 14]);
        let mut detail = column![heading, separator(c)].width(Fill);
        if let Some(error) = &state.page_error
            && is_markdown(&item.path)
        {
            detail = detail.push(
                container(one_line(
                    format!("Showing the source: {error}"),
                    SMALL_SIZE,
                    c.muted,
                ))
                .padding([6, 14]),
            );
        }
        let content: Element<'a, Message> = if !item.exists {
            centered(c, &missing_note(item))
        } else if item.body.trim().is_empty() {
            centered(c, "This file is empty.")
        } else if let Some(id) = self.agent_page_shown(tab) {
            crate::browser::BrowserHost::new(id, move |bounds| Message::BrowserBounds(id, bounds))
                .into()
        } else if let Some((_, _, content)) = state.text.as_ref() {
            self.agent_editor(tab, content, &item.path)
        } else {
            Space::new().width(Fill).height(Fill).into()
        };
        detail.push(content).into()
    }

    /// An item's text in a read-only editor (selectable and copyable) in the terminal font,
    /// highlighted by its file's extension, its lines wrapped.
    fn agent_editor<'a>(
        &self,
        tab: TabId,
        content: &'a text_editor::Content,
        path: &Path,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let selection = crate::theme::to_iced(self.palette.selection, 1.0);
        text_editor(content)
            .on_action(move |a| Message::AgentSetup(tab, AgentMessage::Edit(a)))
            .font(self.metrics.font)
            .size((self.metrics.font_size * BODY_SCALE).round())
            .height(Fill)
            .padding([10, 14])
            .wrapping(iced::widget::text::Wrapping::WordOrGlyph)
            .style(move |theme, s| {
                let mut style = text_editor::default(theme, s);
                style.background = iced::Color::TRANSPARENT.into();
                style.border = iced::Border::default();
                style.value = c.text;
                style.selection = selection;
                style
            })
            .highlight(&crate::file_view::syntax(path), self.highlight_theme())
            .into()
    }
}

/// A section's header in the list: its title in capitals with how many it has, under a line
/// that ends the section before (unless it's the `first`).
fn header<'a>(
    c: crate::theme::Chrome,
    title: &'a str,
    count: usize,
    first: bool,
) -> Element<'a, Message> {
    let font = iced::Font {
        weight: iced::font::Weight::Semibold,
        ..iced::Font::DEFAULT
    };
    let mut line = row![one_line(title.to_uppercase(), HEADER_SIZE, c.label).font(font)]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
    if count > 0 {
        line = line.push(
            container(one_line(count.to_string(), SMALL_SIZE, c.muted))
                .padding([0, 6])
                .style(move |_| container::Style {
                    background: Some(c.sunken.into()),
                    border: iced::Border::default().rounded(8.0),
                    ..container::Style::default()
                }),
        );
    }
    let heading = container(line).padding(iced::Padding {
        top: if first { 8.0 } else { 12.0 },
        right: 12.0,
        bottom: 6.0,
        left: 12.0,
    });
    if first {
        heading.into()
    } else {
        column![Space::new().height(8), separator(c), heading].into()
    }
}

/// A faint line saying a section has nothing.
fn none_line<'a>(c: crate::theme::Chrome, message: &'a str) -> Element<'a, Message> {
    container(one_line(message, SMALL_SIZE, c.faint))
        .padding([2, 12])
        .into()
}

/// An item's row in the list: its agent's icon, name and detail, and whose it is as a badge in
/// `badge`'s color.
fn item_row<'a>(
    c: crate::theme::Chrome,
    tab: TabId,
    item: &'a Item,
    on: bool,
    badge: iced::Color,
) -> Element<'a, Message> {
    let name_color = if item.exists { c.text } else { c.muted };
    let content = row![
        icon(tool_icon(item.tool), ICON_SIZE, c.label),
        column![
            one_line(item.name.as_str(), TEXT_SIZE, name_color),
            one_line(item.detail.as_str(), SMALL_SIZE, c.muted),
        ]
        .spacing(1)
        .width(Fill)
        .clip(true),
        container(one_line(item.scope.name(), BADGE_SIZE, badge))
            .padding([1, 6])
            .style(move |_| container::Style {
                background: Some(badge.scale_alpha(BADGE_TINT).into()),
                border: iced::Border {
                    color: badge.scale_alpha(BADGE_EDGE),
                    width: 1.0,
                    radius: 4.0.into(),
                },
                ..container::Style::default()
            }),
    ]
    .spacing(8)
    // The button's fixed height would otherwise leave the row at its top.
    .height(Fill)
    .align_y(alignment::Vertical::Center);
    container(
        button(content)
            .on_press(Message::AgentSetup(tab, AgentMessage::Select(item.key())))
            .width(Fill)
            .height(ROW_HEIGHT)
            .padding([0, 8])
            .style(move |_, s| flat_button(c, s, on.then_some(c.raised))),
    )
    .padding([0, 6])
    .into()
}

/// A palette color as iced's.
fn rgb(c: macaroni_term::Rgb) -> iced::Color {
    iced::Color::from_rgb8(c.r, c.g, c.b)
}

/// The item selected in `state`, if it's still there.
fn selected_item(state: &AgentTab) -> Option<&Item> {
    let key = state.selected.as_ref()?;
    state.setup.as_ref()?.items.iter().find(|i| &i.key() == key)
}

/// Whether `path` is shown as rendered Markdown.
fn is_markdown(path: &Path) -> bool {
    crate::file_view::Kind::of(path) == crate::file_view::Kind::Markdown
}

/// A hash of an item's text, to tell when it changed without keeping a copy.
fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// The icon standing for an agent.
fn tool_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Claude => Icon::Sparkles,
        Tool::Codex => Icon::Bot,
    }
}

/// What a missing instructions file is for, and that it can be made.
fn missing_note(item: &Item) -> String {
    let reads = match (item.tool, item.scope) {
        (Tool::Claude, Scope::Personal) => "Claude Code reads it in every project",
        (Tool::Claude, Scope::Project) => "Claude Code reads it in this project",
        (Tool::Codex, Scope::Personal) => "Codex reads it in every project",
        (Tool::Codex, Scope::Project) => {
            "Codex (and other agents that read AGENTS.md) read it in this project"
        }
    };
    format!(
        "{} doesn't exist yet. {reads}: write instructions there that it should always follow.\n\nCreate makes it empty and opens it.",
        item.name
    )
}

/// `path` with the home folder as `~`.
pub(super) fn short_path(path: &Path) -> String {
    let shown = path.to_string_lossy().into_owned();
    match crate::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        None => shown,
    }
}

/// Shows `path` in the system's file manager: a folder opened, a file selected in its folder.
/// Spawns and doesn't wait; failures are ignored.
pub(crate) fn reveal(path: &Path) {
    let mut command = if cfg!(windows) {
        let mut explorer = std::process::Command::new("explorer");
        if path.is_file() {
            explorer.arg(format!("/select,{}", path.display()));
        } else {
            explorer.arg(path);
        }
        explorer
    } else if cfg!(target_os = "macos") {
        let mut open = std::process::Command::new("open");
        if path.is_file() {
            open.arg("-R");
        }
        open.arg(path);
        open
    } else {
        let mut open = std::process::Command::new("xdg-open");
        open.arg(if path.is_file() {
            path.parent().unwrap_or(path)
        } else {
            path
        });
        open
    };
    let _ = command.spawn();
}
