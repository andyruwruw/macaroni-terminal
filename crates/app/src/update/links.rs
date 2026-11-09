//! What a Ctrl+click (Cmd+click on macOS) in a terminal opens, and the Ctrl+right-click menu of
//! places to open it.
//!
//! [`App::clicked_at`] finds the link under a cell: an OSC 8 hyperlink or a URL in the text
//! (http and https only, see `macaroni_term::is_openable`), else a file named there that exists
//! (`macaroni_term::file_at`, resolved from the terminal's folder). A click opens a URL as
//! `[browser] open_links` says and a file as `[files] open` says ([`App::default_open`]). The
//! menu (`MenuTarget::Link`, its entries in `view/menus.rs`) offers each place, and
//! [`Message::OpenClicked`] opens it there.
//!
//! Files are never opened with the system's default handler: output is untrusted, and a path
//! could name a program or script. A program the user chose gets the path as an argument.

use std::path::{Path, PathBuf};

use iced::{Point, Task};
use macaroni_config::{OpenFiles, OpenLinks};
use macaroni_core::{Command, Direction, SessionId, TabKind};
use macaroni_term::{GridPoint, file_at, is_openable, url_at};

use crate::message::{Clicked, FileLink, Message, OpenIn};
use crate::state::MenuTarget;
use crate::view::file_tab::FileMessage;
use crate::vscode::Waiting;
use crate::{App, open_url};

/// The commands of VS Code and the editors built on it, which take `-g path:line:column`.
const GOTO_EDITORS: [&str; 5] = ["code", "code-insiders", "codium", "cursor", "windsurf"];

impl App {
    /// The link at a cell of terminal `id`: an OSC 8 hyperlink, else a URL in the text, else a
    /// file reference (`src/main.rs:12:5`, `README.md`) naming a file that exists.
    pub(crate) fn clicked_at(&self, id: SessionId, at: GridPoint) -> Option<Clicked> {
        let r = self.running.get(&id)?;
        let row = r.snapshot.rows.get(at.row);
        let url = r
            .session
            .hyperlink_at(at)
            .filter(|u| is_openable(u))
            .or_else(|| url_at(row?, at.col).map(|link| link.url));
        if let Some(url) = url {
            return Some(Clicked::Url(url));
        }
        let file = file_at(row?, at.col)?;
        Some(Clicked::File(FileLink {
            path: self.resolve_file(id, &file.path)?,
            line: file.line,
            column: file.column,
        }))
    }

    /// `path` as written in a terminal's output, as an existing file: Git Bash's `/c/x` read as
    /// `C:/x`, and a relative path taken from the tab's directory (else its workspace's).
    /// `None` when there's no such file.
    fn resolve_file(&self, id: SessionId, path: &str) -> Option<PathBuf> {
        let mut written = path.to_owned();
        if cfg!(windows) {
            let bytes = path.as_bytes();
            if bytes.len() >= 3
                && bytes[0] == b'/'
                && bytes[1].is_ascii_alphabetic()
                && bytes[2] == b'/'
            {
                written = format!("{}:{}", (bytes[1] as char).to_ascii_uppercase(), &path[2..]);
            }
        }
        let path = PathBuf::from(&written);
        let full = if path.is_absolute() {
            path
        } else {
            let tab = self.model.tab_for_session(id)?;
            let base = self
                .model
                .tab(tab)
                .and_then(|t| t.cwd.clone())
                .or_else(|| {
                    let ws = self.workspace_of_tab(tab)?;
                    self.workspace_dir(self.model.workspace(ws)?)
                })?;
            Path::new(&base).join(path)
        };
        full.is_file().then_some(full)
    }

    /// The link modifier was held while clicking a cell: open what's there where the settings
    /// say.
    pub(crate) fn open_link_at(&mut self, id: SessionId, at: GridPoint) -> Task<Message> {
        match self.clicked_at(id, at) {
            Some(clicked) => {
                let how = self.default_open(&clicked);
                self.open_clicked(clicked, how)
            }
            None => Task::none(),
        }
    }

    /// The link modifier was held while right-clicking a cell: the menu of places to open
    /// what's there, at `point` in the window. Nothing when there's no link.
    pub(crate) fn link_menu(
        &mut self,
        id: SessionId,
        at: GridPoint,
        point: Point,
    ) -> Task<Message> {
        let Some(clicked) = self.clicked_at(id, at) else {
            return Task::none();
        };
        self.clicked = Some(clicked);
        self.menu_open(MenuTarget::Link, point)
    }

    /// Where a click opens `clicked`: a URL by `[browser] open_links` (the system browser where
    /// there are no browser tabs), a file by `[files] open` (the viewer when VS Code tabs
    /// were picked but VS Code isn't downloaded).
    pub(crate) fn default_open(&self, clicked: &Clicked) -> OpenIn {
        match clicked {
            Clicked::Url(_) => {
                let tabs = cfg!(any(windows, target_os = "macos"));
                if self.config.browser.open_links == OpenLinks::System || !tabs {
                    OpenIn::SystemBrowser
                } else {
                    OpenIn::BrowserPanel
                }
            }
            Clicked::File(_) => match self.config.files.open {
                OpenFiles::Vscode if self.vscode_web.installed => OpenIn::VsCode,
                OpenFiles::Viewer | OpenFiles::Vscode => OpenIn::Viewer,
                OpenFiles::App => OpenIn::App,
            },
        }
    }

    /// The name of `[files] app` for menus: the program's file name without its extension, or
    /// when it's empty, VS Code if its `code` command is installed, else the text editor.
    pub(crate) fn app_name(&self) -> String {
        let program = self.config.files.app.trim();
        if program.is_empty() {
            let name = if self.vscode.is_some() {
                "VS Code"
            } else if cfg!(target_os = "macos") {
                "TextEdit"
            } else {
                "Notepad"
            };
            return name.to_owned();
        }
        Path::new(program.trim_end_matches(['/', '\\']))
            .file_stem()
            .map_or_else(|| program.to_owned(), |s| s.to_string_lossy().into_owned())
    }

    /// Opens `clicked` in `how` (a menu item, or the click's default), closing the menu.
    pub(crate) fn open_clicked(&mut self, clicked: Clicked, how: OpenIn) -> Task<Message> {
        self.menu = None;
        match (clicked, how) {
            (Clicked::Url(url), OpenIn::BrowserPanel) => self.open_in_browser_pane(url),
            (Clicked::Url(url), OpenIn::Copy) => iced::clipboard::write(url),
            (Clicked::Url(url), _) => {
                open_url(&url);
                Task::none()
            }
            (Clicked::File(file), OpenIn::VsCode) => self.open_in_vscode(file),
            (Clicked::File(file), OpenIn::App) => {
                open_with(&self.config.files.app, &file);
                Task::none()
            }
            (Clicked::File(file), OpenIn::Program(program)) => {
                open_with(&program.to_string_lossy(), &file);
                Task::none()
            }
            (Clicked::File(file), OpenIn::PickApp) => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .set_title("Open with")
                        .pick_file()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                move |program| match program {
                    Some(program) => {
                        Message::OpenClicked(Clicked::File(file.clone()), OpenIn::Program(program))
                    }
                    None => Message::Frame,
                },
            ),
            (Clicked::File(file), OpenIn::Folder) => {
                crate::view::agent_tab::reveal(&file.path);
                Task::none()
            }
            (Clicked::File(file), OpenIn::Copy) => {
                iced::clipboard::write(file.path.to_string_lossy().into_owned())
            }
            (Clicked::File(file), _) => self.open_in_viewer(file),
        }
    }

    /// Opens `file` in a file tab and goes to its line: the tab already showing it in the
    /// active workspace if there is one, else a new tab in the pane a file tab shows in, else
    /// a new split to the right of the focused pane.
    fn open_in_viewer(&mut self, file: FileLink) -> Task<Message> {
        let path = file.path.to_string_lossy().into_owned();
        let mut same = None;
        let mut viewer = None;
        for pane in self.model.active_workspace().layout.leaves() {
            let Some(p) = self.model.pane(pane) else {
                continue;
            };
            let is_file = |tab| {
                self.model
                    .tab(tab)
                    .filter(|t| t.kind == TabKind::File)
                    .map(|t| t.file.as_deref() == Some(path.as_str()))
            };
            if let Some(tab) = p.tabs.iter().copied().find(|t| is_file(*t) == Some(true)) {
                same = Some(tab);
                break;
            }
            if viewer.is_none() && is_file(p.active).is_some() {
                viewer = Some(pane);
            }
        }
        let mut tasks = Vec::new();
        let tab = match same {
            Some(tab) => {
                tasks.push(self.apply(Command::SelectTab(tab)));
                tab
            }
            None => {
                let split = match viewer {
                    Some(pane) => {
                        tasks.push(self.apply(Command::FocusPane(pane)));
                        None
                    }
                    None => Some(Direction::Right),
                };
                tasks.push(self.apply(Command::OpenFile { path, split }));
                let tab = self.model.focused_tab().id;
                tasks.push(self.file_update(tab, FileMessage::Refresh { force: true }));
                tab
            }
        };
        if let Some(line) = file.line {
            tasks.push(self.file_update(tab, FileMessage::Line(line)));
        }
        Task::batch(tasks)
    }

    /// Opens `file` in a VS Code tab at its line, once the server listens (starting it first
    /// if needed). VS Code opens the terminal's folder when the file is inside it, else the
    /// file's own folder. Falls back to the viewer when VS Code isn't downloaded.
    fn open_in_vscode(&mut self, file: FileLink) -> Task<Message> {
        if !self.vscode_web.installed {
            return self.open_in_viewer(file);
        }
        let folder = self
            .model
            .pane_of_tab(self.model.focused_tab().id)
            .and_then(|pane| self.pane_dir(pane))
            .filter(|dir| file.path.starts_with(dir))
            .or_else(|| file.path.parent().map(Path::to_path_buf));
        self.vscode_web.error = None;
        if self.vscode_web.ensure_started().is_some() {
            // Opened by `vscode_ready`.
            self.vscode_web.waiting.push(Waiting::File(folder, file));
            return Task::none();
        }
        if self.vscode_web.server.is_none() {
            let error = self.vscode_web.error.clone().unwrap_or_default();
            self.config_errors.push(error);
            return Task::none();
        }
        self.open_vscode_file(folder.as_deref(), &file)
    }

    /// A VS Code tab at `folder` showing `file`, placed like a link's page: in the browser pane,
    /// else a split to the right. The server is running.
    pub(crate) fn open_vscode_file(
        &mut self,
        folder: Option<&Path>,
        file: &FileLink,
    ) -> Task<Message> {
        let Some(url) = self
            .vscode_web
            .server
            .as_ref()
            .map(|s| s.url(folder, Some(file)))
        else {
            return Task::none();
        };
        let task = self.open_in_browser_pane(url);
        let tab = self.model.focused_tab().id;
        Task::batch([
            task,
            self.apply(Command::SetTabIcon(tab, Some("code".to_owned()))),
        ])
    }
}

/// Opens `file` with `program` (`[files] app`, or one just picked): a path or a command on
/// `PATH`, or on macOS an `.app` bundle (through `open -a`). Empty means VS Code, else the
/// text editor (`open_text_file`). VS Code and the editors built on it ([`GOTO_EDITORS`]) get
/// `-g path:line:column`; anything else gets just the path. Spawns and doesn't wait; failures
/// are ignored.
fn open_with(program: &str, file: &FileLink) {
    let program = program.trim();
    if program.is_empty() {
        crate::open_text_file(&file.path, file.line, file.column);
        return;
    }
    let bundle = program.trim_end_matches('/');
    let mut command = if cfg!(target_os = "macos") && bundle.ends_with(".app") {
        let mut open = std::process::Command::new("open");
        open.arg("-a").arg(bundle).arg(&file.path);
        open
    } else {
        let mut command = std::process::Command::new(program);
        let stem = Path::new(program)
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase());
        if stem.is_some_and(|s| GOTO_EDITORS.contains(&s.as_str())) {
            command.arg("-g").arg(goto_target(file));
        } else {
            command.arg(&file.path);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // A batch file (`code.cmd`) would flash a console window up.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let ext = Path::new(program)
                .extension()
                .map(|e| e.to_ascii_lowercase());
            if ext.is_some_and(|e| e == "cmd" || e == "bat") {
                command.creation_flags(CREATE_NO_WINDOW);
            }
        }
        command
    };
    let _ = command.spawn();
}

/// `path:line:column` as `code -g` takes it, with what the reference didn't give left off.
fn goto_target(file: &FileLink) -> String {
    let mut target = file.path.to_string_lossy().into_owned();
    if let Some(line) = file.line {
        target = format!("{target}:{line}");
        if let Some(column) = file.column {
            target = format!("{target}:{column}");
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goto_targets_leave_off_what_is_missing() {
        let file = |line, column| FileLink {
            path: PathBuf::from("a.rs"),
            line,
            column,
        };
        assert_eq!(goto_target(&file(None, None)), "a.rs");
        assert_eq!(goto_target(&file(Some(3), None)), "a.rs:3");
        assert_eq!(goto_target(&file(Some(3), Some(9))), "a.rs:3:9");
    }
}
