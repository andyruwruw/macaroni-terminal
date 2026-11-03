//! The new worktree dialog: a branch name for a new workspace in its own git worktree.
//!
//! "New Worktree…" on a workspace's menu, or the `new_worktree` action for the active
//! workspace, opens it ([`WorktreeMessage::Open`]). Create runs [`git::add_worktree`] off the UI
//! thread on the workspace's folder ([`App::workspace_dir`]): a sibling folder named after the
//! repository and branch, checking out the branch or creating it. When that works, a new
//! workspace named after the branch opens in the worktree, its first terminal typing `claude`
//! if asked. A failure (not a repository, the folder exists) stays in the dialog.

use std::path::PathBuf;

use iced::widget::{Space, button, column, container, mouse_area, opaque, row, text, text_input};
use iced::{Border, Color, Element, Fill, Task, alignment};
use macaroni_core::{Command, WorkspaceId};

use super::widgets::{flat_button, input_style, one_line, shadow};
use crate::icons::{Icon, icon};
use crate::theme::Chrome;
use crate::{App, Message, git, glass};

/// The widget id of the branch field, focused when the dialog opens.
const BRANCH_INPUT: &str = "worktree-dialog-input";

/// The dialog's width when the window has room.
const DIALOG_WIDTH: f32 = 460.0;

/// The least room the dialog leaves around it, across both sides together.
const DIALOG_MARGIN: f32 = 48.0;

/// Corner radius of the dialog.
const RADIUS: f32 = 12.0;

/// The title's text size.
const TITLE_SIZE: f32 = 16.0;

/// Everything else's text size.
const TEXT_SIZE: f32 = 12.5;

/// The open dialog, kept in `App.worktree_dialog`.
#[derive(Debug)]
pub struct WorktreeDialog {
    /// The folder whose repository gets the worktree.
    dir: Option<String>,
    /// The branch field's text.
    branch: String,
    /// Type `claude` into the new workspace's terminal.
    claude: bool,
    /// `git worktree add` is running.
    busy: bool,
    /// Why the last try failed, shown until the text changes.
    error: Option<String>,
}

/// What the dialog's controls send, wrapped in [`Message::Worktree`].
#[derive(Debug, Clone)]
pub enum WorktreeMessage {
    /// Open it for this workspace's repository (closing the menu it was opened from).
    Open(WorkspaceId),
    /// Close it.
    Close,
    /// The branch field changed.
    Input(String),
    /// Flip "Start Claude Code".
    ToggleClaude,
    /// Make the worktree.
    Create,
    /// `git worktree add` finished: the branch and the new folder, or why it failed.
    Created(String, Result<PathBuf, String>),
}

impl App {
    /// Applies a dialog message. Ignored while it's closed, except [`WorktreeMessage::Open`].
    pub(crate) fn worktree_update(&mut self, message: WorktreeMessage) -> Task<Message> {
        if let WorktreeMessage::Open(workspace) = message {
            self.menu = None;
            let dir = self
                .model
                .workspace(workspace)
                .and_then(|ws| self.workspace_dir(ws));
            let error = match &dir {
                Some(d) if git::branch(std::path::Path::new(d)).is_some() => None,
                _ => Some("This workspace's folder isn't in a git repository.".to_owned()),
            };
            self.worktree_dialog = Some(WorktreeDialog {
                dir,
                branch: String::new(),
                claude: true,
                busy: false,
                error,
            });
            return iced::widget::operation::focus(BRANCH_INPUT);
        }
        let Some(dialog) = &mut self.worktree_dialog else {
            return Task::none();
        };
        match message {
            WorktreeMessage::Open(_) => {}
            WorktreeMessage::Close => self.worktree_dialog = None,
            WorktreeMessage::Input(value) => {
                dialog.branch = value;
                dialog.error = None;
            }
            WorktreeMessage::ToggleClaude => dialog.claude = !dialog.claude,
            WorktreeMessage::Create => {
                let (Some(dir), false) = (dialog.dir.clone(), dialog.busy) else {
                    return Task::none();
                };
                dialog.busy = true;
                dialog.error = None;
                let branch = dialog.branch.trim().to_owned();
                return Task::future(crate::vscode::off_thread(move || {
                    let made = git::add_worktree(std::path::Path::new(&dir), &branch);
                    (branch, made)
                }))
                .map(|(branch, made)| Message::Worktree(WorktreeMessage::Created(branch, made)));
            }
            WorktreeMessage::Created(_, Err(error)) => {
                dialog.busy = false;
                dialog.error = Some(error);
            }
            WorktreeMessage::Created(branch, Ok(path)) => {
                let claude = dialog.claude;
                self.worktree_dialog = None;
                return self.open_worktree_workspace(&branch, &path, claude);
            }
        }
        Task::none()
    }

    /// A new workspace named `branch`, its folder `path`, its first terminal typing `claude`
    /// when asked. The folder is set in the same update, so the terminal starts there.
    fn open_worktree_workspace(
        &mut self,
        branch: &str,
        path: &std::path::Path,
        claude: bool,
    ) -> Task<Message> {
        let created = self.apply(Command::NewWorkspace);
        let ws = self.model.active_workspace().id;
        let tab = self.model.focused_tab().id;
        if claude {
            self.startup_input.insert(tab, "claude\r".to_owned());
        }
        Task::batch([
            created,
            self.apply(Command::RenameWorkspace(ws, branch.to_owned())),
            self.apply(Command::SetWorkspaceCwd(
                ws,
                Some(path.to_string_lossy().into_owned()),
            )),
        ])
    }

    /// The dialog over a dimmed app, or nothing while it's closed.
    pub(crate) fn worktree_dialog_view(&self) -> Option<Element<'_, Message>> {
        let dialog = self.worktree_dialog.as_ref()?;
        let c = self.chrome;
        let repo = dialog
            .dir
            .as_deref()
            .map(|d| format!("A new workspace in its own worktree of the repository in {d}."))
            .unwrap_or_default();
        let field = text_input("Branch (an existing one, or a new one)", &dialog.branch)
            .id(BRANCH_INPUT)
            .on_input(|value| Message::Worktree(WorktreeMessage::Input(value)))
            .on_submit(Message::Worktree(WorktreeMessage::Create))
            .size(TEXT_SIZE)
            .padding([7, 10])
            .style(move |_, status| input_style(c, status));
        let claude = row![
            super::settings::toggle(
                c,
                dialog.claude,
                Message::Worktree(WorktreeMessage::ToggleClaude)
            ),
            one_line("Start Claude Code in it", TEXT_SIZE, c.text),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center);
        let mut content = column![
            header(c),
            text(repo).size(TEXT_SIZE).color(c.muted),
            field,
            claude
        ]
        .spacing(12);
        if let Some(error) = &dialog.error {
            content = content.push(text(error.clone()).size(TEXT_SIZE).color(c.danger));
        }
        let create = if dialog.busy {
            "Creating\u{2026}"
        } else {
            "Create"
        };
        content = content.push(
            row![
                Space::new().width(Fill),
                action(c, "Cancel", WorktreeMessage::Close, false),
                action(c, create, WorktreeMessage::Create, true),
            ]
            .spacing(8),
        );
        let card = container(content)
            .padding(20)
            .width((self.window_size.width - DIALOG_MARGIN).clamp(280.0, DIALOG_WIDTH))
            .style(move |_| container::Style {
                background: Some(
                    if c.glass.is_some() {
                        c.popup
                    } else {
                        c.background
                    }
                    .into(),
                ),
                border: Border {
                    color: if c.glass.is_some() {
                        Color::TRANSPARENT
                    } else {
                        c.item_edge
                    },
                    width: 1.0,
                    radius: RADIUS.into(),
                },
                shadow: shadow(c.menu_shadow, 12.0, 40.0),
                ..container::Style::default()
            });
        let backdrop = mouse_area(
            container(opaque(glass::sheet(c, card, RADIUS, 1.0)))
                .center(Fill)
                .style(move |_| container::Style {
                    background: Some(c.overlay.into()),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::Worktree(WorktreeMessage::Close));
        Some(backdrop.into())
    }
}

/// The dialog's title and close button.
fn header<'a>(c: Chrome) -> Element<'a, Message> {
    row![
        one_line("New worktree workspace", TITLE_SIZE, c.text),
        Space::new().width(Fill),
        button(icon(Icon::Close, 14.0, c.muted))
            .on_press(Message::Worktree(WorktreeMessage::Close))
            .style(move |_, status| flat_button(c, status, None))
            .padding(5),
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

/// A text button with an outline; `primary` is raised, the others sunken.
fn action<'a>(
    c: Chrome,
    label: &'a str,
    message: WorktreeMessage,
    primary: bool,
) -> Element<'a, Message> {
    button(one_line(label, TEXT_SIZE, c.text))
        .on_press(Message::Worktree(message))
        .padding([6, 14])
        .style(move |_, status| {
            let mut style = flat_button(c, status, Some(if primary { c.raised } else { c.sunken }));
            style.border = Border {
                color: c.separator,
                width: 1.0,
                radius: c.s.controls.radius.into(),
            };
            style
        })
        .into()
}
