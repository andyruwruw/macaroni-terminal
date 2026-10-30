//! The workspace folder dialog: a small modal with a text field for the folder new tabs in a
//! workspace open in.
//!
//! "Set Active Folder…" on a workspace's menu opens it ([`FolderMessage::Open`]) with the
//! current folder typed in and selected. Browse picks one with the system folder dialog (`rfd`,
//! run as a task so the UI keeps drawing). Enter or Save checks the path is a folder and applies
//! [`Command::SetWorkspaceCwd`]; a leading `~` means the home folder, and an empty field puts the
//! workspace back on the home folder. Escape, Cancel or a click outside closes it.

use std::path::PathBuf;

use iced::widget::{Space, button, column, container, mouse_area, opaque, row, text, text_input};
use iced::{Border, Color, Element, Fill, Task, alignment};
use macaroni_core::{Command, WorkspaceId};

use super::widgets::{flat_button, input_style, one_line, shadow};
use crate::icons::{Icon, icon};
use crate::theme::Chrome;
use crate::{App, Message, glass, home_dir};

/// The widget id of the folder field, focused when the dialog opens.
const FOLDER_INPUT: &str = "folder-dialog-input";

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

/// The open dialog, kept in `App.folder_dialog`.
#[derive(Debug)]
pub struct FolderDialog {
    /// Whose folder it sets.
    workspace: WorkspaceId,
    /// The field's text.
    value: String,
    /// Why the last save failed, shown until the text changes.
    error: Option<String>,
}

/// What the dialog's controls send, wrapped in [`Message::Folder`].
#[derive(Debug, Clone)]
pub enum FolderMessage {
    /// Open it for this workspace (closing the menu it was opened from).
    Open(WorkspaceId),
    /// Close it without saving.
    Close,
    /// The field changed.
    Input(String),
    /// Ask for a folder with the system dialog.
    Browse,
    /// The system dialog closed, with the folder picked, if any.
    Browsed(Option<PathBuf>),
    /// Use what's typed, and close.
    Save,
}

impl App {
    /// Applies a dialog message. Ignored while it's closed, except [`FolderMessage::Open`].
    pub(crate) fn folder_update(&mut self, message: FolderMessage) -> Task<Message> {
        if let FolderMessage::Open(workspace) = message {
            self.menu = None;
            self.folder_dialog = Some(FolderDialog {
                workspace,
                value: self.workspace_folder(workspace),
                error: None,
            });
            // Selected, so typing replaces it and the arrow keys keep it.
            return iced::widget::operation::focus(FOLDER_INPUT)
                .chain(iced::widget::operation::select_all(FOLDER_INPUT));
        }
        let Some(dialog) = &mut self.folder_dialog else {
            return Task::none();
        };
        match message {
            FolderMessage::Open(_) => {}
            FolderMessage::Close => self.folder_dialog = None,
            FolderMessage::Input(value) => {
                dialog.value = value;
                dialog.error = None;
            }
            FolderMessage::Browse => {
                let mut picker =
                    rfd::AsyncFileDialog::new().set_title("Choose the workspace folder");
                let start = PathBuf::from(expand_home(dialog.value.trim()));
                if start.is_dir() {
                    picker = picker.set_directory(start);
                }
                return Task::perform(
                    async move { picker.pick_folder().await.map(|f| f.path().to_path_buf()) },
                    |path| Message::Folder(FolderMessage::Browsed(path)),
                );
            }
            FolderMessage::Browsed(None) => {}
            FolderMessage::Browsed(Some(path)) => {
                dialog.value = path.to_string_lossy().into_owned();
                dialog.error = None;
            }
            FolderMessage::Save => {
                let typed = dialog.value.trim();
                if typed.is_empty() {
                    let ws = dialog.workspace;
                    self.folder_dialog = None;
                    return self.apply(Command::SetWorkspaceCwd(ws, None));
                }
                let path = expand_home(typed);
                if !PathBuf::from(&path).is_dir() {
                    dialog.error = Some(format!("{typed} isn't a folder."));
                    return Task::none();
                }
                let ws = dialog.workspace;
                self.folder_dialog = None;
                return self.apply(Command::SetWorkspaceCwd(ws, Some(path)));
            }
        }
        Task::none()
    }

    /// The dialog over a dimmed app, or nothing while it's closed.
    pub(crate) fn folder_dialog_view(&self) -> Option<Element<'_, Message>> {
        let dialog = self.folder_dialog.as_ref()?;
        let c = self.chrome;
        let name = self
            .model
            .workspace(dialog.workspace)
            .map(|w| w.name.as_str())
            .unwrap_or_default();
        let field = row![
            text_input("Folder (empty for your home folder)", &dialog.value)
                .id(FOLDER_INPUT)
                .on_input(|value| Message::Folder(FolderMessage::Input(value)))
                .on_submit(Message::Folder(FolderMessage::Save))
                .size(TEXT_SIZE)
                .padding([7, 10])
                .style(move |_, status| input_style(c, status)),
            action(c, "Browse\u{2026}", FolderMessage::Browse, false),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
        let mut content = column![
            header(c),
            one_line(format!("New tabs in {name} open here."), TEXT_SIZE, c.muted),
            field,
        ]
        .spacing(12);
        if let Some(error) = &dialog.error {
            content = content.push(text(error.clone()).size(TEXT_SIZE).color(c.danger));
        }
        content = content.push(
            row![
                Space::new().width(Fill),
                action(c, "Cancel", FolderMessage::Close, false),
                action(c, "Save", FolderMessage::Save, true),
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
        .on_press(Message::Folder(FolderMessage::Close));
        Some(backdrop.into())
    }
}

/// `path` with a leading `~` (alone, or before a slash) turned into the home folder.
fn expand_home(path: &str) -> String {
    let rest = match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => rest,
        _ => return path.to_owned(),
    };
    match home_dir() {
        Some(home) => format!("{}{rest}", home.to_string_lossy()),
        None => path.to_owned(),
    }
}

/// The dialog's title and close button.
fn header<'a>(c: Chrome) -> Element<'a, Message> {
    row![
        one_line("Workspace folder", TITLE_SIZE, c.text),
        Space::new().width(Fill),
        button(icon(Icon::Close, 14.0, c.muted))
            .on_press(Message::Folder(FolderMessage::Close))
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
    message: FolderMessage,
    primary: bool,
) -> Element<'a, Message> {
    button(one_line(label, TEXT_SIZE, c.text))
        .on_press(Message::Folder(message))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_means_home() {
        let home = home_dir().map(|h| h.to_string_lossy().into_owned());
        if let Some(home) = home {
            assert_eq!(expand_home("~"), home);
            assert_eq!(expand_home("~/src"), format!("{home}/src"));
        }
        assert_eq!(expand_home("~user/src"), "~user/src");
        assert_eq!(expand_home("C:/work"), "C:/work");
    }
}
