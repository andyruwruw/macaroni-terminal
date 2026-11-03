//! The workspace environment dialog: environment variables for the workspace's terminals, one
//! `NAME=value` per line, and a startup command typed into each new shell there.
//!
//! "Environment & Startup…" on a workspace's menu opens it ([`EnvMessage::Open`]) with the
//! current values. Save checks every line is `NAME=value` with a valid name (letters, digits and
//! `_`, not starting with a digit; blank lines and `#` comments are skipped) and applies
//! [`Command::SetWorkspaceEnv`] and [`Command::SetWorkspaceStartup`]. Both apply to terminals
//! started afterwards. Escape, Cancel or a click outside closes it.

use iced::widget::{
    Space, button, column, container, mouse_area, opaque, row, text, text_editor, text_input,
};
use iced::{Border, Color, Element, Fill, Task, alignment};
use macaroni_core::{Command, WorkspaceId};

use super::widgets::{flat_button, input_style, one_line, shadow};
use crate::icons::{Icon, icon};
use crate::theme::Chrome;
use crate::{App, Message, glass};

/// The widget id of the startup command field.
const STARTUP_INPUT: &str = "env-dialog-startup";

/// The dialog's width when the window has room.
const DIALOG_WIDTH: f32 = 520.0;

/// The least room the dialog leaves around it, across both sides together.
const DIALOG_MARGIN: f32 = 48.0;

/// Corner radius of the dialog.
const RADIUS: f32 = 12.0;

/// The title's text size.
const TITLE_SIZE: f32 = 16.0;

/// Everything else's text size.
const TEXT_SIZE: f32 = 12.5;

/// The variables editor's height.
const EDITOR_HEIGHT: f32 = 140.0;

/// The open dialog, kept in `App.env_dialog`.
pub struct EnvDialog {
    /// Whose environment it sets.
    workspace: WorkspaceId,
    /// The variables, one `NAME=value` per line.
    variables: text_editor::Content,
    /// The startup command field.
    startup: String,
    /// Why the last save failed, shown until something changes.
    error: Option<String>,
}

impl std::fmt::Debug for EnvDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvDialog")
            .field("workspace", &self.workspace)
            .finish_non_exhaustive()
    }
}

/// What the dialog's controls send, wrapped in [`Message::Env`].
#[derive(Debug, Clone)]
pub enum EnvMessage {
    /// Open it for this workspace (closing the menu it was opened from).
    Open(WorkspaceId),
    /// Close it without saving.
    Close,
    /// An edit in the variables editor.
    Edit(text_editor::Action),
    /// The startup command field changed.
    Startup(String),
    /// Check and apply both, and close.
    Save,
}

impl App {
    /// Applies a dialog message. Ignored while it's closed, except [`EnvMessage::Open`].
    pub(crate) fn env_update(&mut self, message: EnvMessage) -> Task<Message> {
        if let EnvMessage::Open(workspace) = message {
            self.menu = None;
            let ws = self.model.workspace(workspace);
            let lines: Vec<String> = ws
                .map(|w| w.env.iter().map(|(k, v)| format!("{k}={v}")).collect())
                .unwrap_or_default();
            self.env_dialog = Some(EnvDialog {
                workspace,
                variables: text_editor::Content::with_text(&lines.join("\n")),
                startup: ws.and_then(|w| w.startup.clone()).unwrap_or_default(),
                error: None,
            });
            return Task::none();
        }
        let Some(dialog) = &mut self.env_dialog else {
            return Task::none();
        };
        match message {
            EnvMessage::Open(_) => {}
            EnvMessage::Close => self.env_dialog = None,
            EnvMessage::Edit(action) => {
                dialog.variables.perform(action);
                dialog.error = None;
            }
            EnvMessage::Startup(value) => {
                dialog.startup = value;
                dialog.error = None;
            }
            EnvMessage::Save => match parse_env(&dialog.variables.text()) {
                Err(error) => dialog.error = Some(error),
                Ok(env) => {
                    let (ws, startup) = (dialog.workspace, dialog.startup.trim().to_owned());
                    self.env_dialog = None;
                    return Task::batch([
                        self.apply(Command::SetWorkspaceEnv(ws, env)),
                        self.apply(Command::SetWorkspaceStartup(ws, Some(startup))),
                    ]);
                }
            },
        }
        Task::none()
    }

    /// The dialog over a dimmed app, or nothing while it's closed.
    pub(crate) fn env_dialog_view(&self) -> Option<Element<'_, Message>> {
        let dialog = self.env_dialog.as_ref()?;
        let c = self.chrome;
        let name = self
            .model
            .workspace(dialog.workspace)
            .map(|w| w.name.as_str())
            .unwrap_or_default();
        let editor = text_editor(&dialog.variables)
            .placeholder("NAME=value, one per line")
            .on_action(|action| Message::Env(EnvMessage::Edit(action)))
            .size(TEXT_SIZE)
            .height(EDITOR_HEIGHT)
            .padding([7, 10]);
        let startup = text_input("A command, e.g. source .venv/bin/activate", &dialog.startup)
            .id(STARTUP_INPUT)
            .on_input(|value| Message::Env(EnvMessage::Startup(value)))
            .on_submit(Message::Env(EnvMessage::Save))
            .size(TEXT_SIZE)
            .padding([7, 10])
            .style(move |_, status| input_style(c, status));
        let mut content = column![
            header(c),
            one_line(
                format!("For terminals started in {name} from now on."),
                TEXT_SIZE,
                c.muted
            ),
            one_line("Environment variables", TEXT_SIZE, c.text),
            editor,
            one_line(
                "Startup command, typed into each new shell",
                TEXT_SIZE,
                c.text
            ),
            startup,
        ]
        .spacing(10);
        if let Some(error) = &dialog.error {
            content = content.push(text(error.clone()).size(TEXT_SIZE).color(c.danger));
        }
        content = content.push(
            row![
                Space::new().width(Fill),
                action(c, "Cancel", EnvMessage::Close, false),
                action(c, "Save", EnvMessage::Save, true),
            ]
            .spacing(8),
        );
        let card = container(content)
            .padding(20)
            .width((self.window_size().width - DIALOG_MARGIN).clamp(300.0, DIALOG_WIDTH))
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
        .on_press(Message::Env(EnvMessage::Close));
        Some(backdrop.into())
    }
}

/// The variables in `text`: one `NAME=value` per line, blank lines and `#` comments skipped.
/// The value is everything after the first `=`, untrimmed except for the line's end. A name
/// must be letters, digits and `_`, not starting with a digit; the error names the first line
/// that isn't valid.
fn parse_env(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut env = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let valid = |name: &str| {
            !name.is_empty()
                && !name.starts_with(|c: char| c.is_ascii_digit())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        match line.split_once('=') {
            Some((name, value)) if valid(name.trim()) => {
                env.push((name.trim().to_owned(), value.to_owned()));
            }
            _ => return Err(format!("Line {} isn't NAME=value: {line}", number + 1)),
        }
    }
    Ok(env)
}

/// The dialog's title and close button.
fn header<'a>(c: Chrome) -> Element<'a, Message> {
    row![
        one_line("Environment & startup", TITLE_SIZE, c.text),
        Space::new().width(Fill),
        button(icon(Icon::Close, 14.0, c.muted))
            .on_press(Message::Env(EnvMessage::Close))
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
    message: EnvMessage,
    primary: bool,
) -> Element<'a, Message> {
    button(one_line(label, TEXT_SIZE, c.text))
        .on_press(Message::Env(message))
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
    fn variables_parse_one_per_line() {
        let env = parse_env("A=1\n\n# a comment\nPATH_EXTRA = x=y \nEMPTY=").unwrap();
        assert_eq!(
            env,
            vec![
                ("A".to_owned(), "1".to_owned()),
                ("PATH_EXTRA".to_owned(), " x=y".to_owned()),
                ("EMPTY".to_owned(), String::new()),
            ]
        );
        assert!(parse_env("1BAD=x").is_err());
        assert!(parse_env("no equals").unwrap_err().contains("Line 1"));
    }
}
