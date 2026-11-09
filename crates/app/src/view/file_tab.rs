//! The file tab: one file shown in a pane. Text in the terminal font, highlighted by its
//! extension, selectable and copyable but not editable; Markdown rendered or as its source;
//! diagram files drawn or as their source; pictures and SVGs fitted to the tab; videos and
//! sounds with a player.
//!
//! Rendered Markdown, diagrams and media are a web page (see [`crate::preview`]) where browser
//! tabs exist, so the downloaded viewers can draw diagrams and math and the web view can play
//! media; the page is a browser panel (`Panel::file`) over the tab, its body published to the
//! preview server with each read. Elsewhere, or when the page can't start, Markdown is drawn
//! by iced's own renderer and media gets a note.
//!
//! The file's name in the bar opens a list of the files in its folder, and a bar along the
//! bottom steps to the previous or next one (as do the arrow keys on a picture); either points
//! the same tab at the other file ([`Command::SetFile`]). The bar's first button shows the
//! folder tree at the left (see [`super::file_tree`]), whose files do the same.
//!
//! The model holds the kind and the path ([`TabKind::File`], [`Tab::file`]); what was read is
//! the app's, in `App.file_tabs`, from [`crate::file_view::read`] run off the UI thread.
//! Opening the tab reads the file, and `Message::FileTick` checks every file tab on screen
//! every [`FILE_TICK`], reading it again when its size or time changed, so edits made
//! elsewhere (an agent's, say) show up.
//!
//! [`Tab::file`]: macaroni_core::Tab::file

use std::path::{Path, PathBuf};

use iced::widget::{
    Space, button, column, container, image, markdown, mouse_area, pin, rich_text, row, scrollable,
    stack, svg, text_editor,
};
use iced::{ContentFit, Element, Fill, Padding, Task, alignment, border};
use macaroni_core::{BrowserId, Command, PaneId, TabId, TabKind};

use super::file_tree::{Tree, TreeMessage};
use super::git_tab::{centered, menu_title, popup, separator, small_button, tooltip_button};
use super::widgets::{
    flat_button, icon_button, one_line, thin_scroll, thin_scroll_style, thin_scrollbar,
};
use crate::file_view::{Kind, Media, Read, Stamp};
use crate::icons::{Icon, icon};
use crate::{App, Message};

/// How often file tabs on screen check whether their file changed.
pub const FILE_TICK: std::time::Duration = std::time::Duration::from_secs(2);

/// The top bar's height.
const BAR_HEIGHT: f32 = 44.0;

/// The bottom bar's height (previous and next).
const STEP_HEIGHT: f32 = 30.0;

/// The size of the file's name.
const TEXT_SIZE: f32 = 12.5;

/// The size of details.
const SMALL_SIZE: f32 = 11.0;

/// Text and code relative to the terminal's font size.
const BODY_SCALE: f32 = 0.9;

/// Rendered Markdown's body text relative to the terminal's font size.
const PROSE_SCALE: f32 = 1.05;

/// The widest rendered Markdown gets, so lines stay readable in a wide pane.
const PROSE_WIDTH: f32 = 860.0;

/// Text longer than this is shown without highlighting, which would slow scrolling through it.
const HIGHLIGHT_MAX: usize = 1024 * 1024;

/// The next id for a file tab's page, counting down from the top so it never meets the
/// model's ids, which count up from zero.
static NEXT_PAGE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// What a finished read hands back.
#[derive(Debug, Clone)]
pub struct Fresh {
    /// The file's stamp.
    stamp: Option<Stamp>,
    /// What was read.
    read: Read,
    /// The page's body, when the file is shown as a page.
    body: Option<String>,
    /// The files in its folder, when they were listed with it.
    siblings: Option<Vec<PathBuf>>,
}

/// One file tab's state, in `App.file_tabs`.
#[derive(Debug, Default)]
pub struct FileTab {
    /// What was read last; `None` until the first read finishes.
    shown: Option<Shown>,
    /// The file's stamp when `shown` was read.
    stamp: Option<Stamp>,
    /// A read is running.
    loading: bool,
    /// Markdown shows its source instead of rendered.
    source: bool,
    /// Lines the text was scrolled down with the wheel, so reading the file again keeps the
    /// place.
    scrolled: i32,
    /// The page showing it rendered, once its first body was published.
    page: Option<BrowserId>,
    /// Whether the page's body was built for a dark background (its code colors).
    dark: bool,
    /// Why the page can't be shown (the preview server didn't start).
    page_error: Option<String>,
    /// Counts reads, so one started for a file the tab has since left is ignored.
    reads: u64,
    /// The files in the file's folder, as last listed (opening the tab, reloading, opening the
    /// list).
    siblings: Vec<PathBuf>,
    /// The list of the folder's files is open.
    menu: bool,
    /// A line (from 1) to select and scroll to once the text is shown.
    line: Option<u32>,
    /// The folder tree beside it.
    pub(super) tree: Tree,
}

/// What a file tab shows.
#[derive(Debug)]
enum Shown {
    /// Text in a read-only editor.
    Text {
        /// The text.
        content: text_editor::Content,
        /// Only the start of the file is in `content`.
        cut: bool,
    },
    /// Markdown, rendered from `items` or shown as its source in `content`.
    Markdown {
        /// The parsed document.
        items: Vec<markdown::Item>,
        /// The source.
        content: text_editor::Content,
        /// Only the start of the file is in `content`.
        cut: bool,
    },
    /// A picture.
    Image(image::Handle),
    /// An SVG drawing.
    Svg(svg::Handle),
    /// Media the page plays or shows.
    Media,
    /// Why there's nothing to show (missing, a folder, binary, unreadable).
    Note(String),
}

/// What a file tab's controls send, wrapped in `Message::File`.
#[derive(Debug, Clone)]
pub enum FileMessage {
    /// Read the file if it changed (the tick), or anyway (`force`: opening, reload, F5).
    Refresh {
        /// Read even when the file looks unchanged.
        force: bool,
    },
    /// Read number `n` finished: what was read, or `None` when the file was unchanged.
    Loaded(u64, Option<Box<Fresh>>),
    /// The text was clicked, scrolled or a key was pressed in it; edits are dropped.
    Edit(text_editor::Action),
    /// Show Markdown's source (`true`) or render it.
    Source(bool),
    /// Select line `n` (from 1) and scroll to it, once the text is read; Markdown shows its
    /// source for it.
    Line(u32),
    /// A link in rendered Markdown was clicked.
    Link(String),
    /// Open the file in VS Code or the text editor (a picture in its usual app).
    Open,
    /// Show the file in the system's file manager.
    Reveal,
    /// Open or close the list of the folder's files.
    Menu,
    /// The folder's files were listed.
    Siblings(Vec<PathBuf>),
    /// Show another file in this tab.
    Go(PathBuf),
    /// Show the next file in the folder (`true`) or the previous one.
    Step(bool),
    /// The folder tree's.
    Tree(TreeMessage),
}

impl App {
    /// Asks for a file, starting in `pane`'s folder, and opens it in a file tab there.
    pub(crate) fn pick_file(&mut self, pane: PaneId) -> Task<Message> {
        self.menu = None;
        let folder = self.pane_dir(pane);
        Task::perform(
            async move {
                let mut dialog = rfd::AsyncFileDialog::new().set_title("Open a file");
                if let Some(folder) = folder {
                    dialog = dialog.set_directory(folder);
                }
                dialog.pick_file().await.map(|f| f.path().to_path_buf())
            },
            move |file| match file {
                Some(path) => Message::OpenFile(pane, path),
                None => Message::Frame,
            },
        )
    }

    /// Opens a file tab for `path` in `pane` and reads it.
    pub(crate) fn open_file_tab(&mut self, pane: PaneId, path: PathBuf) -> Task<Message> {
        self.menu = None;
        let mut tasks = Vec::new();
        if self.model.pane(pane).is_some() {
            tasks.push(self.apply(Command::FocusPane(pane)));
        }
        tasks.push(self.apply(Command::OpenFile {
            path: path.to_string_lossy().into_owned(),
            split: None,
        }));
        let tab = self.model.focused_tab().id;
        tasks.push(self.file_update(tab, FileMessage::Refresh { force: true }));
        Task::batch(tasks)
    }

    /// The file tabs on screen, in any window.
    pub(crate) fn shown_file_tabs(&self) -> Vec<TabId> {
        self.model
            .all_workspaces()
            .iter()
            .flat_map(|ws| ws.shown_panes(macaroni_core::Rect::UNIT))
            .filter_map(|(pane, _)| self.model.pane(pane).map(|p| p.active))
            .filter(|tab| {
                self.model
                    .tab(*tab)
                    .is_some_and(|t| t.kind == TabKind::File)
                    && self.model.tab_visible(*tab)
            })
            .collect()
    }

    /// A key nothing captured while file tab `tab` has the keyboard: F5 reads the file again,
    /// Escape closes the folder's list, and Left and Right step through the folder unless the
    /// tab shows text (whose editor uses them).
    pub(crate) fn file_key(
        &mut self,
        tab: TabId,
        key: &iced::keyboard::Key,
    ) -> Option<Task<Message>> {
        use iced::keyboard::Key::Named;
        use iced::keyboard::key::Named as N;
        let state = self.file_tabs.get(&tab);
        let text = state.is_none_or(|s| {
            matches!(
                s.shown,
                None | Some(Shown::Text { .. } | Shown::Markdown { .. })
            )
        });
        match key {
            Named(N::F5) => Some(self.file_update(tab, FileMessage::Refresh { force: true })),
            Named(N::Escape) if state.is_some_and(|s| s.menu) => {
                Some(self.file_update(tab, FileMessage::Menu))
            }
            Named(N::ArrowLeft) if !text => Some(self.file_update(tab, FileMessage::Step(false))),
            Named(N::ArrowRight) if !text => Some(self.file_update(tab, FileMessage::Step(true))),
            _ => None,
        }
    }

    /// The selected text of file tab `tab` (its text, or Markdown's source), if any.
    pub(crate) fn file_selection(&self, tab: TabId) -> Option<String> {
        match self.file_tabs.get(&tab)?.shown.as_ref()? {
            Shown::Text { content, .. } | Shown::Markdown { content, .. } => content.selection(),
            _ => None,
        }
    }

    /// Applies a file tab's message.
    pub(crate) fn file_update(&mut self, tab: TabId, message: FileMessage) -> Task<Message> {
        let Some(path) = self
            .model
            .tab(tab)
            .filter(|t| t.kind == TabKind::File)
            .and_then(|t| t.file.clone())
            .map(PathBuf::from)
        else {
            return Task::none();
        };
        let state = self.file_tabs.entry(tab).or_default();
        match message {
            FileMessage::Refresh { force } => {
                if state.loading {
                    return Task::none();
                }
                state.loading = true;
                state.reads += 1;
                let n = state.reads;
                // Nothing shown yet, or nothing was there: read whatever the stamp says.
                let known = state.stamp.filter(|_| !force && state.shown.is_some());
                // The tick doesn't list the folder: it can be large.
                let list = force || state.siblings.is_empty();
                let kind = Kind::of(&path);
                let file = path.clone();
                let render = crate::preview::html::Render {
                    dark: self.preview_look().dark,
                    installed: self.viewers.installed.iter().copied().collect(),
                };
                let read = Task::perform(
                    crate::vscode::off_thread(move || {
                        if known.is_some() && crate::file_view::stamp(&path) == known {
                            return None;
                        }
                        let (stamp, read) = crate::file_view::read(&path, kind);
                        Some(Box::new(Fresh {
                            body: page_body(&path, kind, &read, stamp, &render),
                            stamp,
                            read,
                            siblings: list.then(|| crate::file_view::siblings(&path)),
                        }))
                    }),
                    move |read| Message::File(tab, FileMessage::Loaded(n, read)),
                );
                Task::batch([read, self.tree_scan(tab, &file, force)])
            }
            FileMessage::Loaded(n, read) => {
                if n != state.reads {
                    return Task::none();
                }
                state.loading = false;
                let Some(fresh) = read else {
                    return Task::none();
                };
                let Fresh {
                    stamp,
                    read,
                    body,
                    siblings,
                } = *fresh;
                state.stamp = stamp;
                state.shown = Some(shown(&path, read, state.scrolled));
                state.go_to_line();
                if let Some(siblings) = siblings {
                    state.siblings = siblings;
                }
                if let Some(body) = body {
                    self.publish_page(tab, &path, body);
                }
                Task::none()
            }
            FileMessage::Menu => {
                state.menu = !state.menu;
                if !state.menu {
                    return Task::none();
                }
                Task::perform(
                    crate::vscode::off_thread(move || crate::file_view::siblings(&path)),
                    move |files| Message::File(tab, FileMessage::Siblings(files)),
                )
            }
            FileMessage::Siblings(files) => {
                state.siblings = files;
                Task::none()
            }
            FileMessage::Step(next) => match step(&state.siblings, &path, next) {
                Some(target) => self.file_update(tab, FileMessage::Go(target)),
                None => Task::none(),
            },
            FileMessage::Go(target) => {
                state.menu = false;
                if target == path {
                    return Task::none();
                }
                // What was shown and any read still running belong to the old file.
                state.shown = None;
                state.stamp = None;
                state.loading = false;
                state.source = false;
                state.scrolled = 0;
                state.tree.follow(&target);
                let task = self.apply(Command::SetFile(tab, target.to_string_lossy().into_owned()));
                Task::batch([
                    task,
                    self.file_update(tab, FileMessage::Refresh { force: true }),
                ])
            }
            FileMessage::Edit(action) => {
                if action.is_edit() {
                    return Task::none();
                }
                if let text_editor::Action::Scroll { lines } = action {
                    state.scrolled = (state.scrolled + lines).max(0);
                }
                if let Some(Shown::Text { content, .. } | Shown::Markdown { content, .. }) =
                    &mut state.shown
                {
                    content.perform(action);
                }
                Task::none()
            }
            FileMessage::Source(source) => {
                state.source = source;
                state.scrolled = 0;
                Task::none()
            }
            FileMessage::Line(line) => {
                state.line = Some(line);
                if matches!(Kind::of(&path), Kind::Markdown) {
                    state.source = true;
                }
                state.go_to_line();
                Task::none()
            }
            FileMessage::Link(link) => self.open_page_link(tab, &path, &link),
            FileMessage::Open => {
                match Kind::of(&path) {
                    Kind::Image | Kind::Svg | Kind::Media(_) => open_default(&path),
                    Kind::Text | Kind::Markdown | Kind::Diagram(_) => {
                        crate::open_text_file(&path, None, None);
                    }
                }
                Task::none()
            }
            FileMessage::Reveal => {
                super::agent_tab::reveal(&path);
                Task::none()
            }
            FileMessage::Tree(message) => self.tree_update(tab, &path, message),
        }
    }

    /// Follows a link clicked in `document`, shown in tab `tab`: a web address in the browser,
    /// another file in a new file tab in the same pane, a folder in the file manager, any other
    /// scheme (`mailto:`) through the system.
    pub(crate) fn open_page_link(
        &mut self,
        tab: TabId,
        document: &Path,
        link: &str,
    ) -> Task<Message> {
        if link.starts_with("http://") || link.starts_with("https://") {
            return self.open_link(link.to_owned());
        }
        match crate::file_view::linked_file(document, link) {
            Some(target) if target.is_dir() => {
                super::agent_tab::reveal(&target);
                Task::none()
            }
            Some(target) if target.is_file() => match self.model.pane_of_tab(tab) {
                Some(pane) => self.open_file_tab(pane, target),
                None => Task::none(),
            },
            Some(_) => Task::none(),
            None => {
                crate::open_url(link);
                Task::none()
            }
        }
    }

    /// The page tab `tab` shows right now: a file tab's, or an agent setup tab's rendered
    /// Markdown.
    pub(crate) fn tab_page_shown(&self, tab: TabId) -> Option<BrowserId> {
        self.file_page_shown(tab)
            .or_else(|| self.agent_page_shown(tab))
    }

    /// A link clicked in the page of tab `tab` (a file tab or an agent setup tab).
    pub(crate) fn page_link(&mut self, tab: TabId, link: String) -> Task<Message> {
        if self
            .model
            .tab(tab)
            .is_some_and(|t| t.kind == TabKind::AgentSetup)
        {
            self.agent_update(tab, super::agent_tab::AgentMessage::Link(link))
        } else {
            self.file_update(tab, FileMessage::Link(link))
        }
    }

    /// A file tab's content; right-clicking opens the tab's menu.
    pub(crate) fn file_body(&self, tab: TabId) -> Element<'_, Message> {
        crate::right_click::RightClick::new(self.file_content(tab), move |at| {
            Message::MenuOpen(crate::state::MenuTarget::Tab(tab), at)
        })
        .into()
    }

    /// What [`App::file_body`] shows: the bar, then the file.
    fn file_content(&self, tab: TabId) -> Element<'_, Message> {
        let c = self.chrome;
        let Some(path) = self.model.tab(tab).and_then(|t| t.file.as_deref()) else {
            return centered(c, "No file");
        };
        let path = Path::new(path);
        let state = self.file_tabs.get(&tab);
        let mut main = column![self.file_view(tab, path, state)];
        if let Some(steps) = state.and_then(|s| self.file_steps(tab, path, s)) {
            main = main.push(separator(c)).push(steps);
        }
        let main: Element<'_, Message> = if self.file_tree {
            let rule =
                container(Space::new().width(1).height(Fill)).style(move |_| container::Style {
                    background: Some(c.separator.into()),
                    ..container::Style::default()
                });
            row![self.file_tree_view(tab, path), rule, main].into()
        } else {
            main.into()
        };
        let page: Element<'_, Message> =
            column![self.file_bar(tab, path, state), separator(c), main].into();
        match state {
            Some(state) if state.menu => {
                let catcher = mouse_area(Space::new().width(Fill).height(Fill))
                    .on_press(Message::File(tab, FileMessage::Menu));
                stack![
                    page,
                    catcher,
                    pin(self.folder_menu(tab, path, state)).x(8.0).y(BAR_HEIGHT)
                ]
                .into()
            }
            _ => page,
        }
    }

    /// The file itself, between the bars: its page, or what iced draws.
    fn file_view<'a>(
        &'a self,
        tab: TabId,
        path: &'a Path,
        state: Option<&'a FileTab>,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let mut body = column![];
        let Some((state, shown)) = state.and_then(|s| Some((s, s.shown.as_ref()?))) else {
            return centered(c, "Reading the file...");
        };
        if let Shown::Text { cut: true, .. } | Shown::Markdown { cut: true, .. } = shown {
            body = body.push(
                container(one_line(
                    format!(
                        "Only the first {} of this file is shown.",
                        crate::file_view::size(crate::file_view::MAX_TEXT as u64)
                    ),
                    SMALL_SIZE,
                    c.muted,
                ))
                .padding([6, 14]),
            );
        }
        if let Some(error) = self.page_problem(tab) {
            body = body.push(
                container(one_line(
                    format!("Showing the simple view: {error}"),
                    SMALL_SIZE,
                    c.muted,
                ))
                .padding([6, 14]),
            );
        }
        if let Some(id) = self.file_page_shown(tab) {
            return body
                .push(crate::browser::BrowserHost::new(id, move |bounds| {
                    Message::BrowserBounds(id, bounds)
                }))
                .into();
        }
        let content: Element<'_, Message> = match shown {
            Shown::Text { content, .. } => {
                let syntax = crate::file_view::syntax(path);
                let plain = state_len(state) > HIGHLIGHT_MAX;
                self.file_editor(tab, content, (!plain).then_some(syntax))
            }
            Shown::Markdown { content, .. } if state.source => {
                self.file_editor(tab, content, Some("md".to_owned()))
            }
            Shown::Markdown { items, .. } => self.prose(tab, path, items),
            Shown::Image(handle) => {
                container(image(handle.clone()).content_fit(ContentFit::ScaleDown))
                    .padding(16)
                    .center(Fill)
                    .into()
            }
            Shown::Svg(handle) => container(svg(handle.clone()).content_fit(ContentFit::ScaleDown))
                .padding(16)
                .center(Fill)
                .into(),
            // Only while the page hides (the folder's list is open over it).
            Shown::Media => Space::new().width(Fill).height(Fill).into(),
            Shown::Note(note) => centered(c, note),
        };
        body.push(content).into()
    }

    /// The bottom bar: the previous file at the left, where this one is in the folder, the
    /// next file at the right. `None` when the folder has nothing else.
    fn file_steps<'a>(
        &self,
        tab: TabId,
        path: &Path,
        state: &FileTab,
    ) -> Option<Element<'a, Message>> {
        let c = self.chrome;
        let index = state.siblings.iter().position(|p| p == path)?;
        if state.siblings.len() < 2 {
            return None;
        }
        let name = |p: &Path| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let step = |next: bool| {
            let target = step(&state.siblings, path, next);
            let label = target.as_deref().map(name).unwrap_or_default();
            let glyph = icon(
                if next {
                    Icon::ChevronRight
                } else {
                    Icon::ChevronLeft
                },
                14.0,
                if target.is_some() { c.label } else { c.faint },
            );
            let text = one_line(label, SMALL_SIZE, c.muted);
            let content = if next {
                row![text, glyph]
            } else {
                row![glyph, text]
            };
            button(content.spacing(6).align_y(alignment::Vertical::Center))
                .on_press_maybe(
                    target
                        .is_some()
                        .then_some(Message::File(tab, FileMessage::Step(next))),
                )
                .padding([4, 8])
                .style(move |_, s| flat_button(c, s, None))
        };
        let position = one_line(
            format!("{} of {}", index + 1, state.siblings.len()),
            SMALL_SIZE,
            c.faint,
        );
        Some(
            container(
                row![
                    container(step(false)).width(Fill).clip(true),
                    position,
                    container(step(true))
                        .width(Fill)
                        .align_x(alignment::Horizontal::Right)
                        .clip(true),
                ]
                .spacing(10)
                .align_y(alignment::Vertical::Center),
            )
            .height(STEP_HEIGHT)
            .padding([0, 6])
            .align_y(alignment::Vertical::Center)
            .into(),
        )
    }

    /// The list of the folder's files under the file's name, this one marked; picking one
    /// shows it in this tab.
    fn folder_menu<'a>(&self, tab: TabId, path: &Path, state: &FileTab) -> Element<'a, Message> {
        let c = self.chrome;
        let mut list = column![].spacing(1);
        if state.siblings.is_empty() {
            list = list.push(container(one_line("No files", TEXT_SIZE, c.faint)).padding(8));
        }
        for file in &state.siblings {
            let current = file == path;
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            list = list.push(
                button(
                    row![
                        icon(kind_icon(Kind::of(file)), 13.0, c.label),
                        one_line(name, TEXT_SIZE, c.text),
                    ]
                    .spacing(8)
                    .align_y(alignment::Vertical::Center),
                )
                .on_press(Message::File(tab, FileMessage::Go(file.clone())))
                .width(Fill)
                .padding([5, 10])
                .style(move |_, s| flat_button(c, s, current.then_some(c.raised))),
            );
        }
        let folder = path
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "This folder".to_owned());
        popup(
            c,
            column![menu_title(c, folder), thin_scroll(c, list.padding([4, 0])),]
                .spacing(4)
                .into(),
        )
    }

    /// The top bar: the file's name and where it is, then its buttons.
    fn file_bar<'a>(
        &self,
        tab: TabId,
        path: &Path,
        state: Option<&FileTab>,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: FileMessage| Some(Message::File(tab, m));
        let name = path
            .file_name()
            .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy())
            .into_owned();
        let mut detail = path
            .parent()
            .map(super::agent_tab::short_path)
            .unwrap_or_default();
        if let Some(len) = state.and_then(|s| s.stamp).map(Stamp::bytes) {
            detail = format!("{detail} \u{b7} {}", crate::file_view::size(len));
        }
        let tree = tooltip_button(
            c,
            Icon::PanelLeft,
            if self.file_tree {
                "Hide the folder tree"
            } else {
                "Show the folder tree"
            },
            send(FileMessage::Tree(TreeMessage::Toggle)),
        );
        let mut buttons = row![].spacing(2).align_y(alignment::Vertical::Center);
        let kind = Kind::of(path);
        let open = state.is_some_and(|s| s.menu);
        if kind == Kind::Markdown || matches!(kind, Kind::Diagram(_)) && crate::preview::AVAILABLE {
            let source = state.is_some_and(|s| s.source);
            buttons = buttons.push(
                container(small_button(
                    c,
                    if source {
                        if Kind::of(path) == Kind::Markdown {
                            "Rendered"
                        } else {
                            "Drawing"
                        }
                    } else {
                        "Source"
                    },
                    false,
                    send(FileMessage::Source(!source)),
                ))
                .padding([0, 6]),
            );
        }
        buttons = buttons
            .push(tooltip_button(
                c,
                Icon::Code,
                "Open in an editor",
                send(FileMessage::Open),
            ))
            .push(tooltip_button(
                c,
                Icon::Folder,
                "Show in the file manager",
                send(FileMessage::Reveal),
            ))
            .push(icon_button(
                c,
                Icon::Reload,
                Message::File(tab, FileMessage::Refresh { force: true }),
            ));
        // The name opens the folder's list.
        let title = button(
            row![
                icon(kind_icon(kind), 16.0, c.label),
                column![
                    row![
                        one_line(name, TEXT_SIZE, c.text),
                        icon(Icon::ChevronDown, 12.0, c.muted),
                    ]
                    .spacing(4)
                    .align_y(alignment::Vertical::Center),
                    one_line(detail, SMALL_SIZE, c.muted),
                ]
                .spacing(1),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center),
        )
        .on_press(Message::File(tab, FileMessage::Menu))
        .padding([3, 6])
        .style(move |_, s| flat_button(c, s, open.then_some(c.raised)));
        container(
            row![tree, container(title).width(Fill).clip(true), buttons,]
                .spacing(10)
                .align_y(alignment::Vertical::Center),
        )
        .height(BAR_HEIGHT)
        .padding([0, 6])
        .align_y(alignment::Vertical::Center)
        .into()
    }

    /// Text in a read-only editor in the terminal font, highlighted as `syntax` (an extension
    /// or file name) when given. Long lines wrap: the editor doesn't scroll sideways.
    fn file_editor<'a>(
        &self,
        tab: TabId,
        content: &'a text_editor::Content,
        syntax: Option<String>,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let selection = crate::theme::to_iced(self.palette.selection, 1.0);
        let editor = text_editor(content)
            .on_action(move |a| Message::File(tab, FileMessage::Edit(a)))
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
            });
        match syntax {
            Some(syntax) => editor.highlight(&syntax, self.highlight_theme()).into(),
            None => editor.into(),
        }
    }

    /// Shows `body` (just built from `path`) in file tab `tab`'s page, making the page the
    /// first time. Where the preview server can't start, the tab keeps the simple view and
    /// says why.
    fn publish_page(&mut self, tab: TabId, path: &Path, body: String) {
        let look = self.preview_look();
        let installed = self.viewers.installed.iter().copied().collect();
        let server = crate::preview::server::get();
        let Some(state) = self.file_tabs.get_mut(&tab) else {
            return;
        };
        let server = match server {
            Ok(server) => server,
            Err(error) => {
                state.page_error = Some(error);
                return;
            }
        };
        state.dark = look.dark;
        let id = *state.page.get_or_insert_with(next_page);
        self.browsers
            .entry(id)
            .or_insert_with(|| crate::browser::Panel::for_file(tab));
        server.publish(
            id,
            crate::preview::Page {
                document: path.to_path_buf(),
                body,
                look,
                installed,
                whole: matches!(Kind::of(path), Kind::Diagram(_) | Kind::Media(_)),
            },
        );
    }

    /// The page file tab `tab` shows right now: a file shown as a page, rendered (not its
    /// source), with a page that hasn't failed to start, and the folder's list closed (nothing
    /// iced draws can cover the page).
    pub(crate) fn file_page_shown(&self, tab: TabId) -> Option<BrowserId> {
        let state = self.file_tabs.get(&tab)?;
        let path = self.model.tab(tab)?.file.as_deref()?;
        // A file that's gone or turned binary shows its note, not the last page.
        let readable = matches!(
            state.shown,
            Some(Shown::Text { .. } | Shown::Markdown { .. } | Shown::Media)
        );
        let paged = matches!(
            Kind::of(Path::new(path)),
            Kind::Markdown | Kind::Diagram(_) | Kind::Media(_)
        );
        let id = state
            .page
            .filter(|_| readable && paged && !state.source && !state.menu)?;
        self.browsers
            .get(&id)
            .is_some_and(|p| p.error.is_none())
            .then_some(id)
    }

    /// Why file tab `tab` shows the simple view though it would have a page.
    fn page_problem(&self, tab: TabId) -> Option<String> {
        let state = self.file_tabs.get(&tab)?;
        if state.source {
            return None;
        }
        state.page_error.clone().or_else(|| {
            let panel = self.browsers.get(&state.page?)?;
            panel.error.clone()
        })
    }

    /// The colors and fonts file tabs' pages use: the pane's background and the interface's
    /// text colors, the terminal's font for code.
    pub(crate) fn preview_look(&self) -> crate::preview::Look {
        let c = self.chrome;
        let background = crate::theme::to_iced(self.palette.background, 1.0);
        let mono = match self.metrics.font.family {
            iced::font::Family::Name(name) => name.to_owned(),
            _ => "monospace".to_owned(),
        };
        crate::preview::Look {
            dark: crate::accessibility::contrast::luminance(background) <= 0.4,
            text: c.text,
            muted: c.muted,
            background,
            sunken: c.sunken,
            border: c.border,
            link: c.tab_accent,
            size: (self.metrics.font_size * PROSE_SCALE).round(),
            code_size: (self.metrics.font_size * BODY_SCALE).round(),
            mono,
        }
    }

    /// Keeps the pages of file tabs on screen in step with the app's colors and fonts: a
    /// changed look restyles the page, and a switch between light and dark builds the body
    /// again (its code colors). Runs on the file tick.
    pub(crate) fn restyle_pages(&mut self, tabs: &[TabId]) -> Task<Message> {
        let Some(server) = crate::preview::server::running() else {
            return Task::none();
        };
        let look = self.preview_look();
        let mut tasks = Vec::new();
        for tab in tabs {
            let Some(state) = self.file_tabs.get(tab) else {
                continue;
            };
            let Some(id) = state.page else {
                continue;
            };
            if state.dark != look.dark {
                tasks.push(self.file_update(*tab, FileMessage::Refresh { force: true }));
            } else if server.look(id).is_some_and(|old| old != look) {
                server.restyle(id, look.clone());
            }
        }
        Task::batch(tasks)
    }

    /// Builds every file tab's page again (a viewer was installed or removed, so what its
    /// diagrams show changed).
    pub(crate) fn rebuild_pages(&mut self) -> Task<Message> {
        let tabs: Vec<TabId> = self
            .file_tabs
            .iter()
            .filter(|(_, state)| state.page.is_some())
            .map(|(tab, _)| *tab)
            .collect();
        Task::batch(
            tabs.into_iter()
                .map(|tab| self.file_update(tab, FileMessage::Refresh { force: true })),
        )
    }

    /// The pages of file and agent setup tabs that are gone, which the caller closes.
    pub(crate) fn orphaned_pages(&self) -> Vec<BrowserId> {
        self.browsers
            .iter()
            .filter(|(_, p)| {
                p.file.is_some_and(|tab| {
                    !self.file_tabs.contains_key(&tab) && !self.agent_tabs.contains_key(&tab)
                })
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// The highlighting colors that suit the pane's background.
    pub(super) fn highlight_theme(&self) -> iced::highlighter::Theme {
        let background = crate::theme::to_iced(self.palette.background, 1.0);
        if crate::accessibility::contrast::luminance(background) > 0.4 {
            iced::highlighter::Theme::InspiredGitHub
        } else {
            iced::highlighter::Theme::Base16Ocean
        }
    }

    /// Rendered Markdown, centered and at most [`PROSE_WIDTH`] wide.
    fn prose<'a>(
        &self,
        tab: TabId,
        path: &Path,
        items: &'a [markdown::Item],
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let size = (self.metrics.font_size * PROSE_SCALE).round();
        let mut settings = markdown::Settings::with_text_size(
            size,
            markdown::Style {
                font: iced::Font::DEFAULT,
                inline_code_highlight: markdown::Highlight {
                    background: c.sunken.into(),
                    border: border::rounded(4),
                },
                inline_code_padding: Padding::from([0, 3]),
                inline_code_color: c.text,
                inline_code_font: self.metrics.font,
                code_block_font: self.metrics.font,
                link_color: c.tab_accent,
            },
        );
        settings.code_size = (self.metrics.font_size * BODY_SCALE).round().into();
        let viewer = Prose {
            document: path.to_path_buf(),
            chrome: c,
        };
        let document = markdown::view_with(items, settings, &viewer)
            .map(move |link| Message::File(tab, FileMessage::Link(link)));
        thin_scroll(
            c,
            container(container(document).max_width(PROSE_WIDTH))
                .padding([20, 28])
                .center_x(Fill),
        )
        .height(Fill)
        .into()
    }
}

/// How rendered Markdown draws what iced's defaults don't suit: code blocks in the app's
/// colors, and images from files beside the document.
struct Prose {
    /// The document, whose folder relative image paths start from.
    document: PathBuf,
    /// The app's colors.
    chrome: crate::theme::Chrome,
}

impl<'a> markdown::Viewer<'a, markdown::Uri> for Prose {
    fn on_link_click(url: markdown::Uri) -> markdown::Uri {
        url
    }

    /// A local image (relative to the document, or absolute) drawn at its size or narrower;
    /// anything else (a web address) as its alt text.
    fn image(
        &self,
        settings: markdown::Settings,
        url: &'a markdown::Uri,
        _title: &'a str,
        alt: &markdown::Text,
    ) -> Element<'a, markdown::Uri> {
        let file = crate::file_view::linked_file(&self.document, url).filter(|file| file.is_file());
        match file {
            Some(file) if Kind::of(&file) == Kind::Svg => svg(svg::Handle::from_path(file))
                .width(iced::Shrink)
                .content_fit(ContentFit::ScaleDown)
                .into(),
            Some(file) => image(image::Handle::from_path(file))
                .content_fit(ContentFit::ScaleDown)
                .into(),
            None => {
                container(rich_text(alt.spans(settings.style)).on_link_click(Self::on_link_click))
                    .padding(settings.spacing.0 / 2.0)
                    .into()
            }
        }
    }

    /// A code block on the sunken color, scrolling sideways when its lines are long.
    fn code_block(
        &self,
        settings: markdown::Settings,
        _language: Option<&'a str>,
        _code: &'a str,
        lines: &'a [markdown::Text],
    ) -> Element<'a, markdown::Uri> {
        let c = self.chrome;
        let lines = column(lines.iter().map(|line| {
            rich_text(line.spans(settings.style))
                .on_link_click(Self::on_link_click)
                .font(settings.style.code_block_font)
                .size(settings.code_size)
                .into()
        }));
        container(
            scrollable(container(lines).padding([10, 12]))
                .direction(scrollable::Direction::Horizontal(thin_scrollbar()))
                .style(move |theme, status| thin_scroll_style(c, theme, status)),
        )
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(c.sunken.into()),
            border: border::rounded(6),
            ..container::Style::default()
        })
        .into()
    }
}

/// The size of what a file tab read, for deciding whether to highlight it.
fn state_len(state: &FileTab) -> usize {
    state
        .stamp
        .map_or(0, |s| usize::try_from(s.bytes()).unwrap_or(usize::MAX))
}

/// A new page's id, counting down from `u64::MAX` so it never meets the model's ids.
pub(super) fn next_page() -> BrowserId {
    BrowserId(NEXT_PAGE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed))
}

/// The page's body for what was read from `path`, when the file is shown as a page: Markdown,
/// diagram and media files where pages exist. Blocking (highlighting); runs with the read.
fn page_body(
    path: &Path,
    kind: Kind,
    read: &Read,
    stamp: Option<Stamp>,
    render: &crate::preview::html::Render,
) -> Option<String> {
    if !crate::preview::AVAILABLE {
        return None;
    }
    match (kind, read) {
        (Kind::Markdown, Read::Text { text, .. }) => {
            Some(crate::preview::html::markdown(text, render))
        }
        (Kind::Diagram(extension), Read::Text { text, .. }) => {
            Some(crate::preview::html::diagram(extension, text, render))
        }
        (Kind::Media(media), Read::Media) => Some(crate::preview::html::media(
            media,
            &path.file_name()?.to_string_lossy(),
            &stamp.map(Stamp::tag).unwrap_or_default(),
        )),
        _ => None,
    }
}

/// The file before (`next` false) or after `path` among `files`, if any.
fn step(files: &[PathBuf], path: &Path, next: bool) -> Option<PathBuf> {
    let index = files.iter().position(|p| p == path)?;
    let target = if next {
        index.checked_add(1)?
    } else {
        index.checked_sub(1)?
    };
    files.get(target).cloned()
}

/// The icon for a kind of file.
pub(super) fn kind_icon(kind: Kind) -> Icon {
    match kind {
        Kind::Media(Media::Audio) => Icon::Music,
        _ => Icon::FileText,
    }
}

impl FileTab {
    /// Selects the pending [`FileTab::line`] in the text shown, which scrolls the editor to it.
    /// It stays pending until there's text.
    fn go_to_line(&mut self) {
        let content = match &mut self.shown {
            Some(Shown::Text { content, .. } | Shown::Markdown { content, .. }) => content,
            _ => return,
        };
        let Some(line) = self.line.take() else {
            return;
        };
        let line = (line as usize)
            .saturating_sub(1)
            .min(content.line_count().saturating_sub(1));
        let end = content.line(line).map_or(0, |l| l.text.len());
        content.move_to(text_editor::Cursor {
            position: text_editor::Position { line, column: end },
            selection: Some(text_editor::Position { line, column: 0 }),
        });
    }
}

/// What to show for what was read from `path`, scrolled down `scrolled` lines (text kept in
/// place across reads).
fn shown(path: &Path, read: Read, scrolled: i32) -> Shown {
    let text = |text: &str| {
        let mut content = text_editor::Content::with_text(text);
        if scrolled > 0 {
            content.perform(text_editor::Action::Scroll { lines: scrolled });
        }
        content
    };
    match read {
        Read::Text { text: body, cut } => match Kind::of(path) {
            Kind::Markdown => Shown::Markdown {
                items: markdown::parse(&body).collect(),
                content: text(&body),
                cut,
            },
            _ => Shown::Text {
                content: text(&body),
                cut,
            },
        },
        Read::Media => match Kind::of(path) {
            _ if crate::preview::AVAILABLE => Shown::Media,
            Kind::Media(Media::Picture) => Shown::Image(image::Handle::from_path(path)),
            _ => {
                Shown::Note("Videos and sounds play in file tabs on Windows and macOS.".to_owned())
            }
        },
        Read::Bytes(bytes) => match Kind::of(path) {
            Kind::Svg => Shown::Svg(svg::Handle::from_memory(bytes)),
            _ => Shown::Image(image::Handle::from_bytes(bytes)),
        },
        Read::Binary => Shown::Note(
            "This file isn't text, so it isn't shown here. Open it in another app instead."
                .to_owned(),
        ),
        Read::Folder => Shown::Note("This is a folder, not a file.".to_owned()),
        Read::Missing => {
            Shown::Note("This file doesn't exist. It shows here again if it's made.".to_owned())
        }
        Read::Failed(error) => Shown::Note(format!("Couldn't read this file: {error}")),
    }
}

/// Opens `path` in the system's usual app for it. Spawns and doesn't wait; failures are
/// ignored.
fn open_default(path: &Path) {
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = command.arg(path).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_waits_for_text_then_is_selected() {
        let mut tab = FileTab {
            line: Some(3),
            ..FileTab::default()
        };
        tab.go_to_line();
        assert_eq!(tab.line, Some(3));
        tab.shown = Some(Shown::Text {
            content: text_editor::Content::with_text("one\ntwo\nthree\nfour"),
            cut: false,
        });
        tab.go_to_line();
        assert_eq!(tab.line, None);
        let Some(Shown::Text { content, .. }) = &tab.shown else {
            unreachable!()
        };
        assert_eq!(content.selection().as_deref(), Some("three"));

        tab.line = Some(99);
        tab.go_to_line();
        let Some(Shown::Text { content, .. }) = &tab.shown else {
            unreachable!()
        };
        assert_eq!(content.selection().as_deref(), Some("four"));
    }
}
