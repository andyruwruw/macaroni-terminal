//! A Git tab: a repository's changes and history, in the manner of GitHub Desktop.
//!
//! A top bar names the repository (a menu switches to another one open in macaroni), the branch
//! (a menu switches, or creates one from what's typed) and offers the one remote action that fits:
//! publish a branch without an upstream, pull when behind, push when ahead, else fetch. Under it,
//! Changes lists the working tree's files with a checkbox each (checked means staged, so the
//! next commit takes it), the selected file's diff against `HEAD`, and a commit box; History
//! lists the branch's commits, a commit's files, and a file's diff in it.
//!
//! The model only knows the tab's kind and folder (`TabKind::Git`, `Tab.cwd`); everything shown
//! is the app's, in `App.git_tabs` by tab, made on first use and dropped when the tab closes.
//! `git` runs off the UI thread (`git::repo`): a status load at a time per tab, and operations
//! one after another from a queue, each followed by a fresh status. A shown Git tab reloads on
//! every `GIT_TICK`, so edits made elsewhere (an agent's, say) show up on their own.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use iced::widget::{
    Space, button, column, container, mouse_area, pin, row, scrollable, stack, text, text_editor,
    text_input,
};
use iced::{Border, Color, Element, Fill, Length, Task, alignment};
use macaroni_core::{Command, TabId, TabKind};

use super::widgets::{
    flat_button, icon_button, input_style, one_line, shadow, thin_scroll, thin_scrollbar,
};
use crate::git::repo::{
    self, Change, ChangeKind, Commit, CommitFile, Diff, LineKind, Op, Staged, Status,
};
use crate::icons::{Icon, icon};
use crate::{App, Message};

/// The top bar's height.
const BAR_HEIGHT: f32 = 46.0;

/// The widest a top bar button gets.
const BAR_BUTTON_WIDTH: f32 = 220.0;

/// The left column's share of the tab, and its narrowest and widest.
const LIST_SHARE: f32 = 0.38;
/// See [`LIST_SHARE`].
const LIST_MIN: f32 = 200.0;
/// See [`LIST_SHARE`].
const LIST_MAX: f32 = 340.0;

/// The narrowest a dragged left column leaves the diff beside it.
const DETAIL_MIN: f32 = 240.0;

/// The room between a list's rows and its scrollbar, which sits beside them, not over them.
const SCROLLBAR_GAP: f32 = 2.0;

/// The side of a changed file's letter box.
const MARK_SIZE: f32 = 17.0;

/// How wide the grabbable edge of the left column is, centered on its line.
const RESIZE_HANDLE: f32 = 8.0;

/// The width of a commit's file list in History.
const FILES_WIDTH: f32 = 220.0;

/// A list row's height.
const ROW_HEIGHT: f32 = 28.0;

/// A commit row's height in History.
const COMMIT_HEIGHT: f32 = 48.0;

/// Main text size.
const TEXT_SIZE: f32 = 12.5;

/// Small text: labels, details.
const SMALL_SIZE: f32 = 11.0;

/// Icons in rows and buttons.
const ICON_SIZE: f32 = 15.0;

/// The commit description's height.
const DESCRIPTION_HEIGHT: f32 = 72.0;

/// The tallest a top bar menu gets.
const MENU_HEIGHT: f32 = 320.0;

/// How strongly added and removed lines are tinted.
const LINE_TINT: f32 = 0.16;

/// How strongly a hunk header is tinted.
const HUNK_TINT: f32 = 0.10;

/// What a diff's rows leave for its vertical scrollbar.
const SCROLLBAR_ROOM: f32 = 12.0;

/// The diff's text size relative to the terminal's font size.
const DIFF_SCALE: f32 = 0.9;

/// Which view a Git tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GitView {
    /// The working tree's changes and the commit box.
    #[default]
    Changes,
    /// The branch's commits.
    History,
}

/// Which diff a load was for, so a late answer for an earlier selection is dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKey {
    /// A working tree file, by path.
    Change(String),
    /// A file in a commit: the commit's id and the path.
    Commit(String, String),
}

/// An open top bar menu.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BarMenu {
    /// Repositories open in macaroni, by top folder.
    Repository(Vec<PathBuf>),
    /// The local branches; `None` while they load.
    Branch(Option<Vec<String>>),
}

/// The branch's commits, as read at a `HEAD` and ahead count; either changing reloads them.
struct History {
    /// `HEAD` when read.
    head: Option<String>,
    /// How far ahead of its upstream the branch was.
    ahead: u32,
    /// The commits, or why they couldn't be read.
    commits: Result<Vec<Commit>, String>,
}

/// Everything a Git tab shows beyond its folder.
#[derive(Default)]
pub struct GitTab {
    /// The last status, or why it couldn't be read; `None` before the first answer.
    status: Option<Result<Status, String>>,
    /// A status load is running.
    loading: bool,
    /// Changes or History.
    view: GitView,
    /// The selected changed file, by path.
    selected: Option<String>,
    /// The diff shown, and which it is.
    diff: Option<(DiffKey, Result<Diff, String>)>,
    /// The commit summary typed so far.
    summary: String,
    /// The commit description.
    description: text_editor::Content,
    /// The branch's commits.
    history: Option<History>,
    /// The selected commit, by id.
    commit: Option<String>,
    /// The selected commit's files.
    commit_files: Option<(String, Result<Vec<CommitFile>, String>)>,
    /// The selected file in the selected commit, by path.
    commit_file: Option<String>,
    /// Operations waiting their turn.
    queue: VecDeque<Op>,
    /// The operation running.
    running: Option<Op>,
    /// What the last operation's `git` said went wrong, until dismissed.
    error: Option<String>,
    /// A file whose discard waits for a yes.
    discard: Option<Change>,
    /// The open top bar menu.
    menu: Option<BarMenu>,
    /// The branch menu's field: a filter, and the name a new branch would get.
    branch_input: String,
    /// The left column's width, once dragged; `None` follows the tab's width.
    list_width: Option<f32>,
    /// The left column's edge is being dragged.
    resizing: bool,
    /// Diffs wrap long lines instead of scrolling sideways.
    wrap: bool,
}

/// What a Git tab sends, wrapped in [`Message::Git`] with the tab.
#[derive(Debug, Clone)]
pub enum GitMessage {
    /// Reload the status (the refresh button, or a tick).
    Refresh,
    /// A status load finished.
    Loaded(Result<Status, String>),
    /// Show Changes or History.
    Show(GitView),
    /// Select a changed file.
    Select(String),
    /// A diff load finished.
    Diff(DiffKey, Result<Diff, String>),
    /// A file's checkbox: stage it whole, or take it out of the commit.
    Toggle(String),
    /// The list's checkbox: stage everything, or nothing when all is staged.
    ToggleAll,
    /// The summary field changed.
    Summary(String),
    /// An edit in the description.
    Description(text_editor::Action),
    /// Commit what's staged.
    Commit,
    /// Ask before discarding the selected file's changes.
    Discard,
    /// Yes, discard them.
    ConfirmDiscard,
    /// Keep them.
    CancelDiscard,
    /// The running operation finished.
    Done(Result<(), String>),
    /// The history load finished: for this `HEAD` and ahead count.
    History(Option<String>, u32, Result<Vec<Commit>, String>),
    /// Select a commit.
    SelectCommit(String),
    /// A commit's files loaded.
    CommitFiles(String, Result<Vec<CommitFile>, String>),
    /// Select a file in the selected commit.
    SelectCommitFile(String),
    /// The remote button: publish, pull, push or fetch, whichever fits.
    Remote,
    /// Open or close the repository menu.
    RepositoryMenu,
    /// Show another repository.
    PickRepository(PathBuf),
    /// Open or close the branch menu.
    BranchMenu,
    /// The branch list loaded.
    Branches(Result<Vec<String>, String>),
    /// The branch menu's field changed.
    BranchInput(String),
    /// Check out a branch.
    Switch(String),
    /// Create a branch named as typed and check it out.
    CreateBranch,
    /// Close the error banner.
    DismissError,
    /// A press outside an open menu.
    CloseMenu,
    /// Wrap the diff's long lines, or stop.
    ToggleWrap,
    /// The left column's edge was pressed.
    ResizeStart,
    /// The cursor moved while dragging the left column's edge, relative to the columns' left.
    ResizeMove(iced::Point),
}

impl App {
    /// Opens a Git tab for the pane's folder, after its active tab. Nothing happens outside a
    /// repository (the buttons are grayed out there).
    pub(crate) fn open_git_tab(&mut self, pane: macaroni_core::PaneId) -> Task<Message> {
        self.menu = None;
        if !self.pane_in_repository(pane) {
            return Task::none();
        }
        let folder = self
            .pane_dir(pane)
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut tasks = Vec::new();
        if self.model.pane(pane).is_some() {
            tasks.push(self.apply(Command::FocusPane(pane)));
        }
        tasks.push(self.apply(Command::OpenGit {
            folder,
            split: None,
        }));
        let tab = self.model.focused_tab().id;
        tasks.push(self.git_update(tab, GitMessage::Refresh));
        Task::batch(tasks)
    }

    /// A key nothing captured while Git tab `tab` has the keyboard: Up and Down move through
    /// the changed files (or commits; with Alt, a commit's files), Space checks or unchecks the
    /// selected file, Delete asks to discard it, Ctrl+Enter (Cmd+Enter) commits, F5 reloads.
    /// `None` when the key isn't one of these.
    pub(crate) fn git_key(
        &mut self,
        tab: TabId,
        key: &iced::keyboard::Key,
        mods: iced::keyboard::Modifiers,
    ) -> Option<Task<Message>> {
        use iced::keyboard::Key::Named;
        use iced::keyboard::key::Named as N;
        let state = self.git_tabs.get(&tab)?;
        let message = match key.as_ref() {
            Named(N::ArrowUp | N::ArrowDown) => {
                let down = key.as_ref() == Named(N::ArrowDown);
                // The next path or id after (or before) the selected one in `list`; the first
                // (or last) when none is.
                let step = |list: Vec<&String>, at: Option<&String>| -> Option<String> {
                    let i = match at.and_then(|a| list.iter().position(|p| *p == a)) {
                        Some(i) if down => (i + 1).min(list.len().checked_sub(1)?),
                        Some(i) => i.saturating_sub(1),
                        None if down => 0,
                        None => list.len().checked_sub(1)?,
                    };
                    list.get(i).map(|s| (*s).clone())
                };
                match state.view {
                    GitView::Changes => {
                        let files = state.status.as_ref()?.as_ref().ok()?;
                        let paths = files.files.iter().map(|c| &c.path).collect();
                        GitMessage::Select(step(paths, state.selected.as_ref())?)
                    }
                    GitView::History if mods.alt() => {
                        let (_, files) = state.commit_files.as_ref()?;
                        let paths = files.as_ref().ok()?.iter().map(|f| &f.path).collect();
                        GitMessage::SelectCommitFile(step(paths, state.commit_file.as_ref())?)
                    }
                    GitView::History => {
                        let commits = state.history.as_ref()?.commits.as_ref().ok()?;
                        let ids = commits.iter().map(|c| &c.id).collect();
                        GitMessage::SelectCommit(step(ids, state.commit.as_ref())?)
                    }
                }
            }
            Named(N::Space) if state.view == GitView::Changes => {
                GitMessage::Toggle(state.selected.clone()?)
            }
            Named(N::Delete) if state.view == GitView::Changes && state.selected.is_some() => {
                GitMessage::Discard
            }
            Named(N::Enter) if mods.command() => GitMessage::Commit,
            Named(N::F5) => GitMessage::Refresh,
            _ => return None,
        };
        Some(self.git_update(tab, message))
    }

    /// Starts a Git tab over after its folder changed: what it showed was another repository's.
    pub(crate) fn git_folder_changed(&mut self, tab: TabId) -> Task<Message> {
        self.git_tabs.remove(&tab);
        self.git_update(tab, GitMessage::Refresh)
    }

    /// Whether the pane's directory is in a Git repository, for the Git buttons.
    pub(crate) fn pane_in_repository(&self, pane: macaroni_core::PaneId) -> bool {
        self.pane_dir(pane)
            .is_some_and(|dir| crate::git::top(&dir).is_some())
    }

    /// Every Git tab on screen, for the tick that reloads them.
    pub(crate) fn shown_git_tabs(&self) -> Vec<TabId> {
        self.model
            .all_workspaces()
            .iter()
            .flat_map(|ws| ws.shown_panes(macaroni_core::Rect::UNIT))
            .filter_map(|(pane, _)| self.model.pane(pane).map(|p| p.active))
            .filter(|tab| {
                self.model.tab(*tab).is_some_and(|t| t.kind == TabKind::Git)
                    && self.model.tab_visible(*tab)
            })
            .collect()
    }

    /// Applies a Git tab's message.
    pub(crate) fn git_update(&mut self, tab: TabId, message: GitMessage) -> Task<Message> {
        let Some(folder) = self
            .model
            .tab(tab)
            .filter(|t| t.kind == TabKind::Git)
            .map(|t| t.cwd.clone().unwrap_or_default())
        else {
            return Task::none();
        };
        let state = self.git_tabs.entry(tab).or_default();
        let send = move |m: GitMessage| Message::Git(tab, m);
        match message {
            GitMessage::Refresh => {
                if state.loading || state.running.is_some() {
                    return Task::none();
                }
                state.loading = true;
                Task::future(crate::vscode::off_thread(move || {
                    repo::status(Path::new(&folder))
                }))
                .map(move |r| send(GitMessage::Loaded(r)))
            }
            GitMessage::Loaded(result) => {
                state.loading = false;
                // An answer from before an operation started is already stale.
                if state.running.is_some() {
                    return Task::none();
                }
                state.status = Some(result);
                let Some(Ok(status)) = &state.status else {
                    return Task::none();
                };
                if state
                    .selected
                    .as_ref()
                    .is_none_or(|s| !status.files.iter().any(|f| &f.path == s))
                {
                    state.selected = status.files.first().map(|f| f.path.clone());
                }
                if let Some(discard) = &state.discard
                    && !status.files.contains(discard)
                {
                    state.discard = None;
                }
                // History's diff is of a commit, which doesn't change.
                let diff = if state.view == GitView::Changes {
                    load_diff(tab, state)
                } else {
                    Task::none()
                };
                Task::batch([diff, load_history(tab, state)])
            }
            GitMessage::Show(view) => {
                state.view = view;
                state.menu = None;
                Task::batch([load_history(tab, state), load_diff(tab, state)])
            }
            GitMessage::Select(path) => {
                state.selected = Some(path);
                state.discard = None;
                load_diff(tab, state)
            }
            GitMessage::Diff(key, result) => {
                if current_diff(state).as_ref() == Some(&key) {
                    state.diff = Some((key, result));
                }
                Task::none()
            }
            GitMessage::Toggle(path) => {
                let Some(Ok(status)) = &mut state.status else {
                    return Task::none();
                };
                let Some(change) = status.files.iter_mut().find(|f| f.path == path) else {
                    return Task::none();
                };
                let op = if change.staged == Staged::All {
                    change.staged = Staged::None;
                    Op::Unstage(paths(change))
                } else {
                    change.staged = Staged::All;
                    Op::Stage(paths(change))
                };
                enqueue(tab, state, op)
            }
            GitMessage::ToggleAll => {
                let Some(Ok(status)) = &mut state.status else {
                    return Task::none();
                };
                let all = status.files.iter().all(|f| f.staged == Staged::All);
                let files: Vec<String> = status
                    .files
                    .iter()
                    .filter(|f| all || f.staged != Staged::All)
                    .flat_map(paths)
                    .collect();
                if files.is_empty() {
                    return Task::none();
                }
                for f in &mut status.files {
                    f.staged = if all { Staged::None } else { Staged::All };
                }
                enqueue(
                    tab,
                    state,
                    if all {
                        Op::Unstage(files)
                    } else {
                        Op::Stage(files)
                    },
                )
            }
            GitMessage::Summary(summary) => {
                state.summary = summary;
                Task::none()
            }
            GitMessage::Description(action) => {
                state.description.perform(action);
                Task::none()
            }
            GitMessage::Commit => {
                if !can_commit(state) {
                    return Task::none();
                }
                let description = state.description.text();
                let mut message = state.summary.trim().to_owned();
                if !description.trim().is_empty() {
                    message.push_str("\n\n");
                    message.push_str(description.trim());
                }
                enqueue(tab, state, Op::Commit(message))
            }
            GitMessage::Discard => {
                state.discard = selected_change(state).cloned();
                Task::none()
            }
            GitMessage::ConfirmDiscard => match state.discard.take() {
                Some(change) => enqueue(tab, state, Op::Discard(change)),
                None => Task::none(),
            },
            GitMessage::CancelDiscard => {
                state.discard = None;
                Task::none()
            }
            GitMessage::Done(result) => {
                let op = state.running.take();
                match result {
                    Err(error) => state.error = Some(error),
                    Ok(()) => match op {
                        Some(Op::Commit(_)) => {
                            state.summary.clear();
                            state.description = text_editor::Content::new();
                        }
                        Some(Op::Switch(_) | Op::CreateBranch(_)) => {
                            state.menu = None;
                            state.branch_input.clear();
                        }
                        _ => {}
                    },
                }
                match state.queue.pop_front() {
                    Some(next) => start(tab, state, next),
                    None => {
                        // Stale now: whatever the operation did, the next load shows.
                        state.loading = false;
                        self.git_update(tab, GitMessage::Refresh)
                    }
                }
            }
            GitMessage::History(head, ahead, result) => {
                state.history = Some(History {
                    head,
                    ahead,
                    commits: result,
                });
                if let Some(History {
                    commits: Ok(commits),
                    ..
                }) = &state.history
                    && state
                        .commit
                        .as_ref()
                        .is_none_or(|c| !commits.iter().any(|k| &k.id == c))
                {
                    state.commit = commits.first().map(|c| c.id.clone());
                    state.commit_files = None;
                    state.commit_file = None;
                }
                load_commit_files(tab, state)
            }
            GitMessage::SelectCommit(id) => {
                state.commit = Some(id);
                state.commit_files = None;
                state.commit_file = None;
                state.diff = None;
                load_commit_files(tab, state)
            }
            GitMessage::CommitFiles(id, result) => {
                if state.commit.as_ref() != Some(&id) {
                    return Task::none();
                }
                state.commit_file = result
                    .as_ref()
                    .ok()
                    .and_then(|files| files.first())
                    .map(|f| f.path.clone());
                state.commit_files = Some((id, result));
                load_diff(tab, state)
            }
            GitMessage::SelectCommitFile(path) => {
                state.commit_file = Some(path);
                load_diff(tab, state)
            }
            GitMessage::Remote => {
                let Some(Ok(status)) = &state.status else {
                    return Task::none();
                };
                match remote_action(status) {
                    Some((op, _, _)) => enqueue(tab, state, op),
                    None => Task::none(),
                }
            }
            GitMessage::RepositoryMenu => {
                state.menu = match state.menu {
                    Some(BarMenu::Repository(_)) => None,
                    _ => Some(BarMenu::Repository(Vec::new())),
                };
                if state.menu.is_some() {
                    let repos = self.open_repositories();
                    if let Some(state) = self.git_tabs.get_mut(&tab) {
                        state.menu = Some(BarMenu::Repository(repos));
                    }
                }
                Task::none()
            }
            GitMessage::PickRepository(top) => {
                state.menu = None;
                let previous = std::mem::take(state);
                let previous_top = match &previous.status {
                    Some(Ok(s)) => Some(s.top.clone()),
                    _ => None,
                };
                // The same repository keeps what's typed and selected.
                if previous_top.as_ref() == Some(&top) {
                    *state = previous;
                    return Task::none();
                }
                let task = self.apply(Command::SetGitFolder(
                    tab,
                    top.to_string_lossy().into_owned(),
                ));
                Task::batch([task, self.git_folder_changed(tab)])
            }
            GitMessage::BranchMenu => {
                if matches!(state.menu, Some(BarMenu::Branch(_))) {
                    state.menu = None;
                    return Task::none();
                }
                let Some(Ok(status)) = &state.status else {
                    return Task::none();
                };
                let top = status.top.clone();
                state.menu = Some(BarMenu::Branch(None));
                state.branch_input.clear();
                Task::batch([
                    Task::future(crate::vscode::off_thread(move || repo::branches(&top)))
                        .map(move |r| send(GitMessage::Branches(r))),
                    iced::widget::operation::focus(BRANCH_INPUT),
                ])
            }
            GitMessage::Branches(result) => {
                match result {
                    Ok(branches) if matches!(state.menu, Some(BarMenu::Branch(_))) => {
                        state.menu = Some(BarMenu::Branch(Some(branches)));
                    }
                    Ok(_) => {}
                    Err(error) => {
                        state.menu = None;
                        state.error = Some(error);
                    }
                }
                Task::none()
            }
            GitMessage::BranchInput(input) => {
                state.branch_input = input;
                Task::none()
            }
            GitMessage::Switch(branch) => {
                state.menu = None;
                enqueue(tab, state, Op::Switch(branch))
            }
            GitMessage::CreateBranch => {
                let name = state.branch_input.trim().to_owned();
                if name.is_empty() {
                    return Task::none();
                }
                // An existing name switches to it instead.
                if let Some(BarMenu::Branch(Some(branches))) = &state.menu
                    && branches.contains(&name)
                {
                    state.menu = None;
                    return enqueue(tab, state, Op::Switch(name));
                }
                enqueue(tab, state, Op::CreateBranch(name))
            }
            GitMessage::DismissError => {
                state.error = None;
                Task::none()
            }
            GitMessage::CloseMenu => {
                state.menu = None;
                Task::none()
            }
            GitMessage::ToggleWrap => {
                state.wrap = !state.wrap;
                Task::none()
            }
            GitMessage::ResizeStart => {
                state.resizing = true;
                Task::none()
            }
            GitMessage::ResizeMove(at) => {
                if state.resizing {
                    // Clamped again when drawn, against the tab's width.
                    state.list_width = Some(at.x.max(LIST_MIN));
                }
                Task::none()
            }
        }
    }

    /// Whether a Git tab's column edge is being dragged, which needs the button's release from
    /// anywhere in the window (`drag_events`).
    pub(crate) fn git_resizing(&self) -> bool {
        self.git_tabs.values().any(|g| g.resizing)
    }

    /// The left button was released: no Git tab's column edge is dragged any more.
    pub(crate) fn end_git_resize(&mut self) {
        for state in self.git_tabs.values_mut() {
            state.resizing = false;
        }
    }

    /// The top folder of every repository a tab or workspace in macaroni is in, sorted.
    fn open_repositories(&self) -> Vec<PathBuf> {
        let mut tops: Vec<PathBuf> = self
            .model
            .all_workspaces()
            .iter()
            .flat_map(|ws| {
                let tabs = ws
                    .layout
                    .leaves()
                    .into_iter()
                    .filter_map(|p| self.model.pane(p))
                    .flat_map(|p| p.tabs.clone())
                    .filter_map(|t| self.model.tab(t)?.cwd.clone());
                ws.cwd.clone().into_iter().chain(tabs).collect::<Vec<_>>()
            })
            .filter_map(|dir| crate::git::top(Path::new(&dir)))
            .collect();
        tops.sort();
        tops.dedup();
        tops
    }

    /// A Git tab's body, `width` wide; right-clicking it opens the tab's menu.
    pub(crate) fn git_body(&self, tab: TabId, width: f32) -> Element<'_, Message> {
        crate::right_click::RightClick::new(self.git_content(tab, width), move |at| {
            Message::MenuOpen(crate::state::MenuTarget::Tab(tab), at)
        })
        .into()
    }

    /// What [`App::git_body`] shows.
    fn git_content(&self, tab: TabId, width: f32) -> Element<'_, Message> {
        let c = self.chrome;
        let send = move |m: GitMessage| Message::Git(tab, m);
        let Some(state) = self.git_tabs.get(&tab) else {
            return centered(c, "Loading the repository...");
        };
        let status = match &state.status {
            None => return centered(c, "Loading the repository..."),
            Some(Err(error)) => {
                let folder = self
                    .model
                    .tab(tab)
                    .and_then(|t| t.cwd.clone())
                    .unwrap_or_default();
                return column![
                    self.git_bar(tab, state, None, width),
                    centered(
                        c,
                        &format!(
                            "{folder} isn't in a Git repository, or git can't read it.\n\n{error}"
                        ),
                    ),
                ]
                .into();
            }
            Some(Ok(status)) => status,
        };
        // A dragged width keeps at least `DETAIL_MIN` for the diff while the tab allows it.
        let list_width = state.list_width.map_or_else(
            || (width * LIST_SHARE).clamp(LIST_MIN, LIST_MAX),
            |w| w.min(width - DETAIL_MIN).max(LIST_MIN),
        );
        let (left, right) = match state.view {
            GitView::Changes => (
                self.changes_list(tab, state, status),
                self.change_detail(tab, state, status),
            ),
            GitView::History => (
                self.history_list(tab, state),
                self.commit_detail(tab, state),
            ),
        };
        let list_background = sidebar_background(c);
        let left = container(column![self.view_switch(tab, state, status), left])
            .width(list_width)
            .height(Fill)
            .style(move |_| container::Style {
                background: Some(list_background.into()),
                ..container::Style::default()
            });
        let rule = container(Space::new().width(1).height(Fill)).style(move |_| container::Style {
            background: Some(c.separator.into()),
            ..container::Style::default()
        });
        // The column's edge is grabbable a few pixels either side of its line.
        let handle = mouse_area(Space::new().width(RESIZE_HANDLE).height(Fill))
            .on_press(send(GitMessage::ResizeStart))
            .interaction(iced::mouse::Interaction::ResizingHorizontally);
        let columns: Element<'_, Message> = stack![
            row![left, rule, right].height(Fill),
            pin(handle).x(list_width - RESIZE_HANDLE / 2.0),
        ]
        .height(Fill)
        .into();
        // Moves are read relative to the columns while dragging, so the cursor's x is the width.
        // Always wrapped, so starting a drag doesn't rebuild (and scroll back) what's inside.
        let mut columns = mouse_area(columns);
        if state.resizing {
            columns = columns
                .on_move(move |at| send(GitMessage::ResizeMove(at)))
                .interaction(iced::mouse::Interaction::ResizingHorizontally);
        }
        let mut body = column![self.git_bar(tab, state, Some(status), width)];
        if let Some(error) = &state.error {
            body = body.push(error_banner(c, error, send(GitMessage::DismissError)));
        }
        body = body.push(columns);
        let body: Element<'_, Message> = body.into();
        match &state.menu {
            None => body,
            Some(menu) => {
                let button_width = bar_button_width(width);
                let (x, list) = match menu {
                    BarMenu::Repository(repos) => (0.0, self.repository_menu(tab, repos, status)),
                    BarMenu::Branch(branches) => (
                        button_width,
                        self.branch_menu(tab, state, branches.as_deref(), status),
                    ),
                };
                let catcher = mouse_area(Space::new().width(Fill).height(Fill))
                    .on_press(send(GitMessage::CloseMenu));
                stack![body, catcher, pin(list).x(x + 6.0).y(BAR_HEIGHT)].into()
            }
        }
    }

    /// The top bar: repository, branch and remote buttons, and refresh.
    fn git_bar<'a>(
        &self,
        tab: TabId,
        state: &'a GitTab,
        status: Option<&'a Status>,
        width: f32,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: GitMessage| Message::Git(tab, m);
        let button_width = bar_button_width(width);
        let repository = status
            .and_then(|s| s.top.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "None".to_owned());
        let mut bar = row![bar_button(
            c,
            Icon::Repository,
            "Current repository",
            repository,
            true,
            Some(send(GitMessage::RepositoryMenu)),
            button_width,
        )]
        .height(BAR_HEIGHT)
        .align_y(alignment::Vertical::Center);
        if let Some(status) = status {
            let branch = status.branch.clone().unwrap_or_else(|| {
                let head = status.head.as_deref().unwrap_or_default();
                format!("Detached at {}", &head[..head.len().min(7)])
            });
            bar = bar.push(bar_button(
                c,
                Icon::GitBranch,
                "Current branch",
                branch,
                true,
                (state.running.is_none()).then(|| send(GitMessage::BranchMenu)),
                button_width,
            ));
            let remote = match &state.running {
                Some(op @ (Op::Fetch | Op::Pull | Op::Push | Op::Publish(_))) => Some((
                    Icon::Reload,
                    format!("{}...", op.doing()),
                    String::new(),
                    false,
                )),
                _ => remote_action(status).map(|(_, glyph, title)| {
                    let detail = match status.upstream.as_deref() {
                        Some(_) if status.ahead + status.behind > 0 => {
                            sync_counts(status.ahead, status.behind)
                        }
                        Some(upstream) => format!("Up to date with {upstream}"),
                        None => "This branch isn't on the remote yet".to_owned(),
                    };
                    (glyph, title, detail, state.running.is_none())
                }),
            };
            if let Some((glyph, title, detail, enabled)) = remote {
                bar = bar.push(bar_button(
                    c,
                    glyph,
                    &detail,
                    title,
                    false,
                    enabled.then(|| send(GitMessage::Remote)),
                    button_width,
                ));
            }
        }
        bar = bar.push(Space::new().width(Fill));
        if let Some(op) = state.running.as_ref().filter(|op| !is_remote(op)) {
            bar = bar.push(one_line(format!("{}...", op.doing()), SMALL_SIZE, c.muted));
        }
        bar = bar.push(icon_button(c, Icon::Reload, send(GitMessage::Refresh)));
        let line = container(Space::new().width(Fill).height(1)).style(move |_| container::Style {
            background: Some(c.separator.into()),
            ..container::Style::default()
        });
        column![container(bar.spacing(2)).padding([0, 6]), line].into()
    }

    /// The Changes / History switch over the left column.
    fn view_switch<'a>(
        &self,
        tab: TabId,
        state: &'a GitTab,
        status: &'a Status,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let item = |glyph: Icon, label: String, view: GitView| {
            let on = state.view == view;
            let color = if on { c.text } else { c.muted };
            button(
                container(
                    row![icon(glyph, 13.0, color), one_line(label, TEXT_SIZE, color)]
                        .spacing(6)
                        .align_y(alignment::Vertical::Center),
                )
                .center_x(Fill),
            )
            .on_press(Message::Git(tab, GitMessage::Show(view)))
            .width(Fill)
            .padding([7, 8])
            .style(move |_, s| {
                let mut style = flat_button(c, s, on.then_some(c.raised));
                style.border.radius = 0.0.into();
                style
            })
        };
        let count = status.files.len();
        let changes = if count == 0 {
            "Changes".to_owned()
        } else {
            format!("Changes {count}")
        };
        column![
            row![
                item(Icon::GitCompare, changes, GitView::Changes),
                item(Icon::History, "History".to_owned(), GitView::History),
            ],
            separator(c),
        ]
        .into()
    }

    /// Changes: the files with their checkboxes, and the commit box.
    fn changes_list<'a>(
        &self,
        tab: TabId,
        state: &'a GitTab,
        status: &'a Status,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: GitMessage| Message::Git(tab, m);
        let staged = status
            .files
            .iter()
            .filter(|f| f.staged != Staged::None)
            .count();
        let all = if status.files.is_empty() || staged == 0 {
            Icon::Unchecked
        } else if staged == status.files.len()
            && status.files.iter().all(|f| f.staged == Staged::All)
        {
            Icon::Checked
        } else {
            Icon::PartlyChecked
        };
        let header = row![
            check_button(c, all, send(GitMessage::ToggleAll)),
            one_line(
                match status.files.len() {
                    1 => "1 changed file".to_owned(),
                    n => format!("{n} changed files"),
                },
                SMALL_SIZE,
                c.muted
            ),
        ]
        .spacing(4)
        .padding([2, 6])
        .align_y(alignment::Vertical::Center);
        let rows = status.files.iter().map(|change| {
            let selected = state.selected.as_ref() == Some(&change.path);
            let check = match change.staged {
                Staged::All => Icon::Checked,
                Staged::Some => Icon::PartlyChecked,
                Staged::None => Icon::Unchecked,
            };
            let (folder, name) = split_path(&change.path);
            let label = row![
                one_line(name, TEXT_SIZE, c.text),
                one_line(folder, SMALL_SIZE, c.faint),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center);
            let item = button(
                row![
                    container(label).width(Fill).clip(true),
                    kind_mark(self, change.kind),
                ]
                .spacing(6)
                // The button's fixed height would otherwise leave the row at its top.
                .height(Fill)
                .align_y(alignment::Vertical::Center),
            )
            .on_press(send(GitMessage::Select(change.path.clone())))
            .width(Fill)
            .height(ROW_HEIGHT)
            .padding([0, 8])
            .style(move |_, s| flat_button(c, s, selected.then_some(c.raised)));
            row![
                check_button(c, check, send(GitMessage::Toggle(change.path.clone()))),
                item
            ]
            .spacing(2)
            .padding([0, 6])
            .align_y(alignment::Vertical::Center)
            .into()
        });
        let list: Element<'_, Message> = if status.files.is_empty() {
            container(one_line("No local changes", TEXT_SIZE, c.faint))
                .center(Fill)
                .into()
        } else {
            thin_scroll(c, column(rows).spacing(1).padding([4, 0]))
                .spacing(SCROLLBAR_GAP)
                .height(Fill)
                .into()
        };
        column![
            header,
            separator(c),
            list,
            separator(c),
            self.commit_box(tab, state, status)
        ]
        .height(Fill)
        .into()
    }

    /// The summary, description and Commit button.
    fn commit_box<'a>(
        &self,
        tab: TabId,
        state: &'a GitTab,
        status: &'a Status,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: GitMessage| Message::Git(tab, m);
        let staged = status
            .files
            .iter()
            .filter(|f| f.staged != Staged::None)
            .count();
        let summary = text_input("Summary (required)", &state.summary)
            .on_input(move |s| send(GitMessage::Summary(s)))
            .on_submit(send(GitMessage::Commit))
            .size(TEXT_SIZE)
            .padding([6, 8])
            .style(move |_, s| {
                let focused = matches!(s, text_input::Status::Focused { .. });
                text_input::Style {
                    background: c.card.into(),
                    border: field_border(c, focused),
                    ..input_style(c, s)
                }
            });
        let description = text_editor(&state.description)
            .placeholder("Description")
            .on_action(move |a| send(GitMessage::Description(a)))
            .size(TEXT_SIZE)
            .height(DESCRIPTION_HEIGHT)
            .padding([6, 8])
            .style(move |theme, s| {
                let mut style = text_editor::default(theme, s);
                style.background = c.card.into();
                style.border = field_border(c, matches!(s, text_editor::Status::Focused { .. }));
                style.placeholder = c.faint;
                style.value = c.text;
                style
            });
        let branch = status.branch.as_deref().unwrap_or("HEAD");
        let label = match (&state.running, staged) {
            (Some(Op::Commit(_)), _) => "Committing...".to_owned(),
            (_, 0) => format!("Commit to {branch}"),
            (_, 1) => format!("Commit 1 file to {branch}"),
            (_, n) => format!("Commit {n} files to {branch}"),
        };
        let enabled = can_commit(state);
        let commit = button(container(one_line(label, TEXT_SIZE, c.text)).center_x(Fill))
            .on_press_maybe(enabled.then(|| send(GitMessage::Commit)))
            .width(Fill)
            .padding([7, 10])
            .style(move |_, s| {
                let mut style = flat_button(c, s, Some(c.raised));
                if !enabled {
                    style.text_color = c.faint;
                    style.background = Some(c.sunken.into());
                }
                style.border = Border {
                    color: c.separator,
                    width: c.s.controls.border_width,
                    radius: c.s.controls.radius.into(),
                };
                style
            });
        column![summary, description, commit]
            .spacing(6)
            .padding(8)
            .into()
    }

    /// Changes' right side: the selected file's header (or the discard question) and its diff.
    fn change_detail<'a>(
        &self,
        tab: TabId,
        state: &'a GitTab,
        status: &'a Status,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: GitMessage| Message::Git(tab, m);
        let Some(change) = selected_change(state) else {
            let hint = match remote_action(status) {
                Some((Op::Push, _, _)) => "No local changes. Push your commits to share them.",
                Some((Op::Pull, _, _)) => "No local changes. The remote has commits to pull.",
                Some((Op::Publish(_), _, _)) => "No local changes. Publish the branch to share it.",
                _ => "No local changes.",
            };
            return centered(c, hint);
        };
        let header: Element<'_, Message> = if state.discard.as_ref() == Some(change) {
            row![
                one_line(
                    format!(
                        "Discard the changes to {}? This can't be undone.",
                        change.path
                    ),
                    TEXT_SIZE,
                    c.text
                )
                .width(Fill),
                small_button(c, "Discard", true, Some(send(GitMessage::ConfirmDiscard))),
                small_button(c, "Cancel", false, Some(send(GitMessage::CancelDiscard))),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .into()
        } else {
            let path = match &change.from {
                Some(from) => format!("{from} \u{2192} {}", change.path),
                None => change.path.clone(),
            };
            row![
                container(one_line(path, TEXT_SIZE, c.text))
                    .width(Fill)
                    .clip(true),
                wrap_toggle(c, tab, state.wrap),
                tooltip_button(
                    c,
                    Icon::Undo,
                    "Discard changes",
                    state.running.is_none().then(|| send(GitMessage::Discard)),
                ),
            ]
            .spacing(6)
            .align_y(alignment::Vertical::Center)
            .into()
        };
        column![
            container(header)
                .padding([4, 10])
                .height(36)
                .align_y(alignment::Vertical::Center),
            separator(c),
            self.diff_view(state.diff.as_ref().map(|(_, d)| d), state.wrap),
        ]
        .width(Fill)
        .into()
    }

    /// History: the branch's commits.
    fn history_list<'a>(&self, tab: TabId, state: &'a GitTab) -> Element<'a, Message> {
        let c = self.chrome;
        let commits = match &state.history {
            None => return centered(c, "Loading history..."),
            Some(History {
                commits: Err(error),
                ..
            }) => return centered(c, error),
            Some(History {
                commits: Ok(commits),
                ..
            }) if commits.is_empty() => {
                return centered(c, "No commits yet.");
            }
            Some(History {
                commits: Ok(commits),
                ..
            }) => commits,
        };
        let now = now();
        let rows = commits.iter().map(|commit| {
            let selected = state.commit.as_ref() == Some(&commit.id);
            let mut detail = row![
                one_line(
                    format!("{} \u{b7} {}", commit.author, ago(now - commit.time)),
                    SMALL_SIZE,
                    c.muted
                )
                .width(Fill)
            ]
            .align_y(alignment::Vertical::Center);
            if commit.unpushed {
                detail = detail.push(icon(Icon::Up, 12.0, c.muted));
            }
            let content = column![one_line(commit.summary.as_str(), TEXT_SIZE, c.text), detail]
                .spacing(3)
                .width(Fill);
            button(container(content).width(Fill).center_y(Fill).clip(true))
                .on_press(Message::Git(
                    tab,
                    GitMessage::SelectCommit(commit.id.clone()),
                ))
                .width(Fill)
                .height(COMMIT_HEIGHT)
                .padding([6, 10])
                .style(move |_, s| flat_button(c, s, selected.then_some(c.raised)))
                .into()
        });
        thin_scroll(c, column(rows).spacing(1).padding([4, 6]))
            .spacing(SCROLLBAR_GAP)
            .height(Fill)
            .into()
    }

    /// History's right side: the selected commit's message, its files and a file's diff.
    fn commit_detail<'a>(&self, tab: TabId, state: &'a GitTab) -> Element<'a, Message> {
        let c = self.chrome;
        let Some(History {
            commits: Ok(commits),
            ..
        }) = &state.history
        else {
            return Space::new().into();
        };
        let Some(commit) = commits
            .iter()
            .find(|k| Some(&k.id) == state.commit.as_ref())
        else {
            return centered(c, "Select a commit.");
        };
        let mut header = column![
            one_line(commit.summary.as_str(), TEXT_SIZE + 1.0, c.text),
            one_line(
                format!(
                    "{} \u{b7} {} \u{b7} {}",
                    commit.author,
                    &commit.id[..commit.id.len().min(7)],
                    ago(now() - commit.time)
                ),
                SMALL_SIZE,
                c.muted
            ),
        ]
        .spacing(4);
        if !commit.body.is_empty() {
            header = header.push(text(commit.body.as_str()).size(SMALL_SIZE).color(c.label));
        }
        let files: Element<'_, Message> = match &state.commit_files {
            Some((id, Ok(files))) if id == &commit.id => {
                let rows = files.iter().map(|file| {
                    let selected = state.commit_file.as_ref() == Some(&file.path);
                    let (folder, name) = split_path(&file.path);
                    button(
                        row![
                            column![
                                one_line(name, TEXT_SIZE, c.text),
                                one_line(folder, SMALL_SIZE, c.faint)
                            ]
                            .width(Fill)
                            .clip(true),
                            kind_mark(self, file.kind),
                        ]
                        .spacing(6)
                        .align_y(alignment::Vertical::Center),
                    )
                    .on_press(Message::Git(
                        tab,
                        GitMessage::SelectCommitFile(file.path.clone()),
                    ))
                    .width(Fill)
                    .padding([6, 10])
                    .style(move |_, s| flat_button(c, s, selected.then_some(c.raised)))
                    .into()
                });
                thin_scroll(c, column(rows).spacing(1).padding([4, 6]))
                    .spacing(SCROLLBAR_GAP)
                    .height(Fill)
                    .into()
            }
            Some((id, Err(error))) if id == &commit.id => centered(c, error),
            _ => centered(c, "Loading..."),
        };
        let rule = container(Space::new().width(1).height(Fill)).style(move |_| container::Style {
            background: Some(c.separator.into()),
            ..container::Style::default()
        });
        column![
            row![
                container(header).width(Fill).clip(true),
                wrap_toggle(c, tab, state.wrap)
            ]
            .spacing(6)
            .padding([8, 10]),
            separator(c),
            row![
                container(files).width(FILES_WIDTH),
                rule,
                self.diff_view(state.diff.as_ref().map(|(_, d)| d), state.wrap),
            ]
            .height(Fill),
        ]
        .width(Fill)
        .into()
    }

    /// A diff: each line with its old and new numbers, tinted by kind, in the terminal font,
    /// scrolling both ways, or with `wrap` only down, its long lines wrapped.
    fn diff_view<'a>(
        &self,
        diff: Option<&'a Result<Diff, String>>,
        wrap: bool,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let diff = match diff {
            None => return centered(c, "Loading the diff..."),
            Some(Err(error)) => return centered(c, error),
            Some(Ok(diff)) if diff.binary => return centered(c, "Binary file, not shown."),
            Some(Ok(diff)) if diff.lines.is_empty() => {
                return centered(
                    c,
                    "No changes to show (only the file's mode, or it's empty).",
                );
            }
            Some(Ok(diff)) => diff,
        };
        let size = (self.metrics.font_size * DIFF_SCALE).round();
        let char_width = self.metrics.cell_width / self.metrics.font_size * size;
        let digits = diff
            .lines
            .iter()
            .flat_map(|l| [l.old, l.new])
            .flatten()
            .max()
            .unwrap_or(1)
            .to_string()
            .len()
            .max(3);
        let number_width = digits as f32 * char_width + 12.0;
        let longest = diff
            .lines
            .iter()
            .map(|l| l.text.chars().count())
            .max()
            .unwrap_or(0);
        let text_width = number_width * 2.0 + 24.0 + longest as f32 * char_width;
        let added = rgb(self.palette.ansi[2]);
        let removed = rgb(self.palette.ansi[1]);
        let font = self.metrics.font;
        let number = move |n: Option<u32>, color: Color| {
            container(
                text(n.map(|n| n.to_string()).unwrap_or_default())
                    .size(size)
                    .font(font)
                    .color(color),
            )
            .width(number_width)
            .padding([0, 6])
            .align_x(alignment::Horizontal::Right)
        };
        // Rows can't fill a sideways scrollable, so each is as wide as the longest line, or the
        // space when that's wider, which only `responsive` knows.
        iced::widget::responsive(move |area| {
            let row_width = if wrap {
                area.width - SCROLLBAR_ROOM
            } else {
                text_width.max(area.width - SCROLLBAR_ROOM)
            };
            let mut lines = column(diff.lines.iter().map(|line| {
                let (tint, color, mark) = match line.kind {
                    LineKind::Added => (Some(added.scale_alpha(LINE_TINT)), c.text, "+"),
                    LineKind::Removed => (Some(removed.scale_alpha(LINE_TINT)), c.text, "-"),
                    LineKind::Hunk => (Some(c.attention.scale_alpha(HUNK_TINT)), c.muted, ""),
                    LineKind::Context => (None, c.label, " "),
                    LineKind::Note => (None, c.faint, ""),
                };
                container(
                    row![
                        number(line.old, c.faint),
                        number(line.new, c.faint),
                        container(text(mark).size(size).font(font).color(c.muted)).width(24),
                        text(line.text.as_str())
                            .size(size)
                            .font(font)
                            .color(color)
                            .width(Fill)
                            .wrapping(if wrap {
                                text::Wrapping::WordOrGlyph
                            } else {
                                text::Wrapping::None
                            }),
                    ]
                    // A wrapped line's numbers stay by its first row.
                    .align_y(if wrap {
                        alignment::Vertical::Top
                    } else {
                        alignment::Vertical::Center
                    }),
                )
                .width(row_width)
                .style(move |_| container::Style {
                    background: tint.map(Into::into),
                    ..container::Style::default()
                })
                .into()
            }));
            if diff.cut > 0 {
                lines = lines.push(
                    container(one_line(
                        format!("{} more lines not shown.", diff.cut),
                        SMALL_SIZE,
                        c.faint,
                    ))
                    .padding([8, 12]),
                );
            }
            let direction = if wrap {
                scrollable::Direction::Vertical(thin_scrollbar())
            } else {
                scrollable::Direction::Both {
                    vertical: thin_scrollbar(),
                    horizontal: thin_scrollbar(),
                }
            };
            thin_scroll(c, lines.padding([4, 0]))
                .direction(direction)
                .width(Fill)
                .height(Fill)
                .into()
        })
        .into()
    }

    /// The repository menu: every repository open in macaroni.
    fn repository_menu<'a>(
        &self,
        tab: TabId,
        repos: &'a [PathBuf],
        status: &'a Status,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let items = repos.iter().map(|top| {
            let current = top == &status.top;
            let name = top
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            button(
                column![
                    one_line(name, TEXT_SIZE, c.text),
                    one_line(top.to_string_lossy().into_owned(), SMALL_SIZE, c.faint),
                ]
                .spacing(2),
            )
            .on_press(Message::Git(tab, GitMessage::PickRepository(top.clone())))
            .width(Fill)
            .padding([5, 10])
            .style(move |_, s| flat_button(c, s, current.then_some(c.raised)))
            .into()
        });
        let list: Element<'_, Message> = if repos.is_empty() {
            container(one_line(
                "No other repositories are open.",
                TEXT_SIZE,
                c.faint,
            ))
            .padding(10)
            .into()
        } else {
            thin_scroll(c, column(items).spacing(1)).into()
        };
        popup(
            c,
            column![menu_title(c, "Repositories open in macaroni"), list].into(),
        )
    }

    /// The branch menu: a field that filters the branches or names a new one, and the list.
    fn branch_menu<'a>(
        &self,
        tab: TabId,
        state: &'a GitTab,
        branches: Option<&'a [String]>,
        status: &'a Status,
    ) -> Element<'a, Message> {
        let c = self.chrome;
        let send = move |m: GitMessage| Message::Git(tab, m);
        let input = text_input("Filter, or name a new branch", &state.branch_input)
            .id(BRANCH_INPUT)
            .on_input(move |s| send(GitMessage::BranchInput(s)))
            .on_submit(send(GitMessage::CreateBranch))
            .size(TEXT_SIZE)
            .padding([6, 8])
            .style(move |_, s| input_style(c, s));
        let typed = state.branch_input.trim();
        let mut list = column![].spacing(1);
        match branches {
            None => {
                list = list.push(container(one_line("Loading...", TEXT_SIZE, c.faint)).padding(8))
            }
            Some(branches) => {
                let lower = typed.to_lowercase();
                for branch in branches
                    .iter()
                    .filter(|b| b.to_lowercase().contains(&lower))
                {
                    let current = status.branch.as_ref() == Some(branch);
                    list = list.push(
                        button(
                            row![
                                container(if current {
                                    Element::from(icon(Icon::GitBranch, 13.0, c.text))
                                } else {
                                    Space::new().width(13).into()
                                }),
                                one_line(branch.as_str(), TEXT_SIZE, c.text),
                            ]
                            .spacing(8)
                            .align_y(alignment::Vertical::Center),
                        )
                        .on_press_maybe(
                            (!current).then(|| send(GitMessage::Switch(branch.clone()))),
                        )
                        .width(Fill)
                        .padding([5, 10])
                        .style(move |_, s| flat_button(c, s, current.then_some(c.raised))),
                    );
                }
                if !typed.is_empty() && !branches.iter().any(|b| b == typed) {
                    list = list.push(
                        button(
                            row![
                                icon(Icon::Plus, 13.0, c.text),
                                one_line(
                                    format!(
                                        "Create branch {typed} from {}",
                                        status.branch.as_deref().unwrap_or("HEAD")
                                    ),
                                    TEXT_SIZE,
                                    c.text
                                ),
                            ]
                            .spacing(8)
                            .align_y(alignment::Vertical::Center),
                        )
                        .on_press(send(GitMessage::CreateBranch))
                        .width(Fill)
                        .padding([5, 10])
                        .style(move |_, s| flat_button(c, s, None)),
                    );
                }
            }
        }
        popup(
            c,
            column![
                menu_title(c, "Branches"),
                container(input).padding([0, 8]),
                thin_scroll(c, list.padding([4, 0])),
            ]
            .spacing(4)
            .into(),
        )
    }
}

/// The branch menu's field, focused when the menu opens.
const BRANCH_INPUT: &str = "git-branch-input";

/// Starts `op` if nothing runs, else queues it.
fn enqueue(tab: TabId, state: &mut GitTab, op: Op) -> Task<Message> {
    state.error = None;
    if state.running.is_some() {
        state.queue.push_back(op);
        return Task::none();
    }
    start(tab, state, op)
}

/// Runs `op` off the UI thread; [`GitMessage::Done`] reports.
fn start(tab: TabId, state: &mut GitTab, op: Op) -> Task<Message> {
    let Some(Ok(status)) = &state.status else {
        return Task::none();
    };
    let top = status.top.clone();
    let has_head = status.head.is_some();
    state.running = Some(op.clone());
    Task::future(crate::vscode::off_thread(move || {
        repo::perform(&top, &op, has_head)
    }))
    .map(move |r| Message::Git(tab, GitMessage::Done(r)))
}

/// Which diff the tab should show now: the selected file's in Changes, the selected commit
/// file's in History.
fn current_diff(state: &GitTab) -> Option<DiffKey> {
    match state.view {
        GitView::Changes => state.selected.clone().map(DiffKey::Change),
        GitView::History => Some(DiffKey::Commit(
            state.commit.clone()?,
            state.commit_file.clone()?,
        )),
    }
}

/// Loads the diff the tab should show, keeping the one shown until it arrives if it's the same.
fn load_diff(tab: TabId, state: &mut GitTab) -> Task<Message> {
    let Some(key) = current_diff(state) else {
        state.diff = None;
        return Task::none();
    };
    if state.diff.as_ref().is_some_and(|(k, _)| k != &key) {
        state.diff = None;
    }
    let Some(Ok(status)) = &state.status else {
        return Task::none();
    };
    let top = status.top.clone();
    let send = move |result| Message::Git(tab, GitMessage::Diff(key.clone(), result));
    match current_diff(state) {
        Some(DiffKey::Change(path)) => {
            let Some(change) = status.files.iter().find(|f| f.path == path).cloned() else {
                return Task::none();
            };
            let head = status.head.clone();
            Task::future(crate::vscode::off_thread(move || {
                repo::change_diff(&top, &change, head.as_deref())
            }))
            .map(send)
        }
        Some(DiffKey::Commit(id, path)) => {
            let commit = match &state.history {
                Some(History {
                    commits: Ok(commits),
                    ..
                }) => commits.iter().find(|k| k.id == id).cloned(),
                _ => None,
            };
            let file = match &state.commit_files {
                Some((files_of, Ok(files))) if files_of == &id => {
                    files.iter().find(|f| f.path == path).cloned()
                }
                _ => None,
            };
            let (Some(commit), Some(file)) = (commit, file) else {
                return Task::none();
            };
            Task::future(crate::vscode::off_thread(move || {
                repo::commit_diff(&top, &commit, &file)
            }))
            .map(send)
        }
        None => Task::none(),
    }
}

/// Loads the history while History shows, when `HEAD` or the ahead count changed since.
fn load_history(tab: TabId, state: &mut GitTab) -> Task<Message> {
    if state.view != GitView::History {
        return Task::none();
    }
    let Some(Ok(status)) = &state.status else {
        return Task::none();
    };
    if let Some(history) = &state.history
        && history.head == status.head
        && history.ahead == status.ahead
    {
        return Task::none();
    }
    let status = status.clone();
    Task::future(crate::vscode::off_thread(move || {
        let result = repo::history(&status.top, &status);
        (status.head, status.ahead, result)
    }))
    .map(move |(head, ahead, result)| Message::Git(tab, GitMessage::History(head, ahead, result)))
}

/// Loads the selected commit's files, unless they're loaded.
fn load_commit_files(tab: TabId, state: &mut GitTab) -> Task<Message> {
    let Some(id) = state.commit.clone() else {
        return Task::none();
    };
    if state.commit_files.as_ref().is_some_and(|(of, _)| of == &id) {
        return load_diff(tab, state);
    }
    let commit = match &state.history {
        Some(History {
            commits: Ok(commits),
            ..
        }) => commits.iter().find(|k| k.id == id).cloned(),
        _ => None,
    };
    let (Some(commit), Some(Ok(status))) = (commit, &state.status) else {
        return Task::none();
    };
    let top = status.top.clone();
    Task::future(crate::vscode::off_thread(move || {
        repo::commit_files(&top, &commit)
    }))
    .map(move |result| Message::Git(tab, GitMessage::CommitFiles(id.clone(), result)))
}

/// The selected changed file.
fn selected_change(state: &GitTab) -> Option<&Change> {
    let Some(Ok(status)) = &state.status else {
        return None;
    };
    status
        .files
        .iter()
        .find(|f| Some(&f.path) == state.selected.as_ref())
}

/// Whether Commit can run: a summary, something staged, and nothing running.
fn can_commit(state: &GitTab) -> bool {
    let staged = match &state.status {
        Some(Ok(status)) => status.files.iter().any(|f| f.staged != Staged::None),
        _ => false,
    };
    staged && !state.summary.trim().is_empty() && state.running.is_none()
}

/// The paths an operation on `change` names: a rename's old path too, so both sides move
/// together.
fn paths(change: &Change) -> Vec<String> {
    std::iter::once(change.path.clone())
        .chain(change.from.clone())
        .collect()
}

/// The remote button's operation, icon and title for `status`: publish without an upstream,
/// pull when behind, push when ahead, else fetch. `None` without a remote.
fn remote_action(status: &Status) -> Option<(Op, Icon, String)> {
    let remote = status.remote.clone()?;
    Some(if status.upstream.is_none() {
        if status.branch.is_none() {
            (Op::Fetch, Icon::Reload, format!("Fetch {remote}"))
        } else {
            (
                Op::Publish(remote.clone()),
                Icon::Up,
                "Publish branch".to_owned(),
            )
        }
    } else if status.behind > 0 {
        (Op::Pull, Icon::Down, format!("Pull {remote}"))
    } else if status.ahead > 0 {
        (Op::Push, Icon::Up, format!("Push {remote}"))
    } else {
        (Op::Fetch, Icon::Reload, format!("Fetch {remote}"))
    })
}

/// Whether `op` talks to a remote (the remote button shows it running).
fn is_remote(op: &Op) -> bool {
    matches!(op, Op::Fetch | Op::Pull | Op::Push | Op::Publish(_))
}

/// `2 to push, 1 to pull`, leaving out a part that's 0.
fn sync_counts(ahead: u32, behind: u32) -> String {
    let mut parts = Vec::new();
    if ahead > 0 {
        parts.push(format!("{ahead} to push"));
    }
    if behind > 0 {
        parts.push(format!("{behind} to pull"));
    }
    parts.join(", ")
}

/// A path's folder (with its trailing `/`, empty at the top) and file name.
fn split_path(path: &str) -> (String, String) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (format!("{folder}/"), name.to_owned()),
        None => (String::new(), path.to_owned()),
    }
}

/// Seconds since 1970, now.
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// How long ago, roughly: `just now`, `5m ago`, `3h ago`, `2d ago`, `4mo ago`, `2y ago`.
fn ago(seconds: i64) -> String {
    let s = seconds.max(0);
    match s {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", s / 60),
        3600..86_400 => format!("{}h ago", s / 3600),
        86_400..2_592_000 => format!("{}d ago", s / 86_400),
        2_592_000..31_536_000 => format!("{}mo ago", s / 2_592_000),
        _ => format!("{}y ago", s / 31_536_000),
    }
}

/// A terminal palette color as an iced one.
fn rgb(c: macaroni_term::Rgb) -> Color {
    Color::from_rgb8(c.r, c.g, c.b)
}

/// How wide each top bar button is in a tab `width` wide.
fn bar_button_width(width: f32) -> f32 {
    ((width - 60.0) / 3.0).clamp(110.0, BAR_BUTTON_WIDTH)
}

/// A top bar button: an icon, a small label over a value, and a chevron when it opens a menu.
fn bar_button<'a>(
    c: crate::theme::Chrome,
    glyph: Icon,
    label: &str,
    value: String,
    menu: bool,
    message: Option<Message>,
    width: f32,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    let mut content = row![
        icon(glyph, ICON_SIZE, if enabled { c.label } else { c.faint }),
        column![
            one_line(label.to_owned(), SMALL_SIZE - 0.5, c.faint),
            one_line(value, TEXT_SIZE, if enabled { c.text } else { c.muted }),
        ]
        .spacing(1)
        .width(Fill)
        .clip(true),
    ]
    .spacing(8)
    .align_y(alignment::Vertical::Center);
    if menu {
        content = content.push(icon(Icon::ChevronDown, 12.0, c.muted));
    }
    button(content)
        .on_press_maybe(message)
        .width(width)
        .padding([5, 8])
        .style(move |_, s| flat_button(c, s, None))
        .into()
}

/// A checkbox icon as a button.
fn check_button<'a>(
    c: crate::theme::Chrome,
    glyph: Icon,
    message: Message,
) -> Element<'a, Message> {
    button(icon(glyph, ICON_SIZE, c.label))
        .on_press(message)
        .padding(4)
        .style(move |_, s| flat_button(c, s, None))
        .into()
}

/// An icon button named on hover.
pub(super) fn tooltip_button<'a>(
    c: crate::theme::Chrome,
    glyph: Icon,
    name: &str,
    message: Option<Message>,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    iced::widget::tooltip(
        button(icon(
            glyph,
            ICON_SIZE,
            if enabled { c.label } else { c.faint },
        ))
        .on_press_maybe(message)
        .padding(5)
        .style(move |_, s| flat_button(c, s, None)),
        super::sidebar::name_tip(c, name.to_owned()),
        iced::widget::tooltip::Position::Left,
    )
    .into()
}

/// The diff's wrap button, shown pressed while long lines wrap.
fn wrap_toggle<'a>(c: crate::theme::Chrome, tab: TabId, on: bool) -> Element<'a, Message> {
    iced::widget::tooltip(
        button(icon(
            Icon::TextWrap,
            ICON_SIZE,
            if on { c.text } else { c.label },
        ))
        .on_press(Message::Git(tab, GitMessage::ToggleWrap))
        .padding(5)
        .style(move |_, s| flat_button(c, s, on.then_some(c.raised))),
        super::sidebar::name_tip(
            c,
            if on { "Don't wrap lines" } else { "Wrap lines" }.to_owned(),
        ),
        iced::widget::tooltip::Position::Left,
    )
    .into()
}

/// A small text button; `danger` colors it as destructive.
pub(super) fn small_button<'a>(
    c: crate::theme::Chrome,
    label: &'a str,
    danger: bool,
    message: Option<Message>,
) -> Element<'a, Message> {
    button(one_line(
        label,
        SMALL_SIZE,
        if danger { c.danger } else { c.text },
    ))
    .on_press_maybe(message)
    .padding([4, 10])
    .style(move |_, s| {
        let mut style = flat_button(c, s, Some(c.sunken));
        style.border = Border {
            color: if danger { c.danger } else { c.separator },
            width: c.s.controls.border_width,
            radius: c.s.controls.radius.into(),
        };
        style
    })
    .into()
}

/// A changed file's letter in its kind's color.
fn kind_mark<'a>(app: &App, kind: ChangeKind) -> Element<'a, Message> {
    let color = match kind {
        ChangeKind::Added => rgb(app.palette.ansi[2]),
        ChangeKind::Deleted | ChangeKind::Conflicted => rgb(app.palette.ansi[1]),
        ChangeKind::Renamed => rgb(app.palette.ansi[4]),
        ChangeKind::Modified => rgb(app.palette.ansi[3]),
    };
    container(
        text(kind.letter())
            .size(SMALL_SIZE)
            .line_height(1.0)
            .color(color)
            .align_x(alignment::Horizontal::Center)
            .align_y(alignment::Vertical::Center)
            .width(Fill)
            .height(Fill),
    )
    .center(MARK_SIZE)
    .style(move |_| container::Style {
        border: Border {
            color: color.scale_alpha(0.6),
            width: 1.0,
            radius: 3.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// What the left column sits on: the sidebar's background, which is the window's where the
/// sidebar color is see-through.
fn sidebar_background(c: crate::theme::Chrome) -> Color {
    let a = c.sidebar.a;
    let mix = |top: f32, under: f32| top * a + under * (1.0 - a);
    Color::from_rgb(
        mix(c.sidebar.r, c.background.r),
        mix(c.sidebar.g, c.background.g),
        mix(c.sidebar.b, c.background.b),
    )
}

/// The commit box's fields' edge: the outline while focused, else a separator line, so they
/// stand out from the column behind them.
fn field_border(c: crate::theme::Chrome, focused: bool) -> Border {
    Border {
        color: if focused { c.outline } else { c.separator },
        width: c.s.controls.border_width.max(1.0),
        radius: c.s.controls.radius.into(),
    }
}

/// A full-width horizontal rule.
pub(super) fn separator<'a>(c: crate::theme::Chrome) -> Element<'a, Message> {
    container(Space::new().width(Fill).height(1))
        .style(move |_| container::Style {
            background: Some(c.separator.into()),
            ..container::Style::default()
        })
        .into()
}

/// `message` centered in the space, muted.
pub(super) fn centered<'a>(c: crate::theme::Chrome, message: &str) -> Element<'a, Message> {
    container(
        text(message.to_owned())
            .size(TEXT_SIZE)
            .color(c.muted)
            .align_x(alignment::Horizontal::Center),
    )
    .padding(16)
    .center(Fill)
    .into()
}

/// The error banner under the top bar, with a close button.
pub(super) fn error_banner<'a>(
    c: crate::theme::Chrome,
    error: &str,
    close: Message,
) -> Element<'a, Message> {
    container(
        row![
            text(error.to_owned())
                .size(SMALL_SIZE)
                .color(c.danger)
                .width(Fill),
            icon_button(c, Icon::Close, close),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center),
    )
    .padding(iced::Padding {
        top: 4.0,
        right: 6.0,
        bottom: 4.0,
        left: 12.0,
    })
    .width(Fill)
    .style(move |_| container::Style {
        background: Some(c.danger.scale_alpha(0.08).into()),
        ..container::Style::default()
    })
    .into()
}

/// A menu's title line.
pub(super) fn menu_title<'a>(
    c: crate::theme::Chrome,
    title: impl iced::widget::text::IntoFragment<'a>,
) -> Element<'a, Message> {
    container(one_line(title, SMALL_SIZE, c.faint))
        .padding([6, 10])
        .into()
}

/// A floating menu card.
pub(super) fn popup<'a>(
    c: crate::theme::Chrome,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fixed(320.0))
        .max_height(MENU_HEIGHT)
        .padding([4, 0])
        .style(move |_| container::Style {
            background: Some(c.popup.into()),
            border: Border {
                color: c.item_edge,
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: shadow(c.menu_shadow, 6.0, 20.0),
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_remote_action() {
        let mut status = Status {
            top: PathBuf::new(),
            branch: Some("main".into()),
            head: Some("abc".into()),
            upstream: Some("origin/main".into()),
            ahead: 0,
            behind: 0,
            remote: Some("origin".into()),
            files: Vec::new(),
        };
        let op = |s: &Status| remote_action(s).map(|(op, _, title)| (op, title));
        assert_eq!(op(&status), Some((Op::Fetch, "Fetch origin".into())));
        status.ahead = 2;
        assert_eq!(op(&status), Some((Op::Push, "Push origin".into())));
        status.behind = 1;
        assert_eq!(op(&status), Some((Op::Pull, "Pull origin".into())));
        status.upstream = None;
        assert_eq!(
            op(&status),
            Some((Op::Publish("origin".into()), "Publish branch".into()))
        );
        status.remote = None;
        assert_eq!(op(&status), None);
        assert_eq!(sync_counts(2, 1), "2 to push, 1 to pull");
        assert_eq!(ago(90), "1m ago");
        assert_eq!(split_path("a/b/c.rs"), ("a/b/".into(), "c.rs".into()));
    }
}
