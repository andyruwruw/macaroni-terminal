//! Context menus: opening them and their submenus, custom colors, Escape, and the new-tab menu.
//!
//! `App.menu` is the open menu; `view/menus.rs` builds its entries and lays it out. Picking an
//! item closes the menu and hands the action to `update/actions.rs`. The new-tab menu's items
//! (a shell, a browser, VS Code) and the Git Bash offer are handled here too.

use std::path::PathBuf;

use iced::{Point, Task};
use macaroni_core::{Command, GroupId, PaneId, TabId, WorkspaceId};
use macaroni_term::shell::{self, ShellOption};

use crate::message::{ColorTarget, GroupAction, Message, TabAction, WorkspaceAction};
use crate::state::{Menu, MenuPage, MenuTarget, Submenu};
use crate::{App, home_dir, open_vscode, theme, vscode};

/// What a terminal runs for an `[[ssh]]` profile.
pub(crate) fn ssh_shell(profile: &macaroni_config::SshProfile) -> ShellOption {
    ShellOption {
        name: profile.label().to_owned(),
        program: "ssh".to_owned(),
        args: profile.ssh_args(),
    }
}

/// What the Git Bash offer runs in PowerShell: winget installs Git for Windows, in a tab that
/// stays open (`-NoExit`) so its prompts, UAC and result stay with the user.
const INSTALL_GIT_BASH: &str = "winget install --id Git.Git -e --source winget; if ($LASTEXITCODE -eq 0) { Write-Host ''; Write-Host 'Git Bash is installed. New tabs will use it.' }";

impl App {
    /// Opens a context menu at `at`; the new-tab menu looks for shells and VS Code first, so a
    /// shell installed since launch shows up.
    pub(crate) fn menu_open(&mut self, target: MenuTarget, at: Point) -> Task<Message> {
        if let MenuTarget::NewTab(_) = target {
            self.shells = shell::available();
            self.vscode = shell::vscode();
        }
        self.menu = Some(Menu {
            target,
            at,
            sub: None,
        });
        Task::none()
    }

    /// The pointer entered a menu item: open its submenu, or close an unpinned one.
    pub(crate) fn menu_hover(&mut self, page: Option<MenuPage>) -> Task<Message> {
        if let Some(menu) = &mut self.menu {
            let pinned = menu.sub.is_some_and(|s| s.pinned);
            match page {
                Some(page) if menu.sub.is_none_or(|s| s.page != page) => {
                    self.open_submenu(page, false)
                }
                None if !pinned => menu.sub = None,
                _ => {}
            }
        }
        Task::none()
    }

    /// A submenu's item was clicked: keep it open.
    pub(crate) fn menu_pin(&mut self, page: MenuPage) -> Task<Message> {
        self.open_submenu(page, true);
        Task::none()
    }

    /// Opens `page` beside the main list (pinned if clicked), replacing any other submenu.
    fn open_submenu(&mut self, page: MenuPage, pinned: bool) {
        let Some(menu) = &mut self.menu else {
            return;
        };
        if menu.sub.is_none_or(|s| s.page != page) {
            // The custom field starts with the item's custom color, if it has one.
            let color = match menu.target {
                MenuTarget::Workspace(ws) => self.model.workspace(ws).and_then(|w| w.color.clone()),
                MenuTarget::Group(group) => self.model.group(group).and_then(|g| g.color.clone()),
                MenuTarget::Tab(tab) => self.model.tab(tab).and_then(|t| t.color.clone()),
                MenuTarget::NewTab(_) => None,
            };
            self.color_input = color.filter(|c| c.starts_with('#')).unwrap_or_default();
        }
        menu.sub = Some(Submenu { page, pinned });
    }

    /// Applies the color typed in a color menu's custom field, if it's a color; otherwise the
    /// text stays in the field.
    pub(crate) fn color_submit(&mut self, target: ColorTarget) -> Task<Message> {
        let Some(color) = theme::custom_color(&self.color_input) else {
            return Task::none();
        };
        match target {
            ColorTarget::Workspace(ws) => {
                self.menu = None;
                self.apply(Command::SetWorkspaceColor(ws, Some(color)))
            }
            ColorTarget::Group(group) => self.group_color(group, Some(color)),
            ColorTarget::Tab(tab) => {
                self.menu = None;
                self.apply(Command::SetTabColor(tab, Some(color)))
            }
        }
    }

    /// Sets a group's color from its menu, then asks whether its workspaces get it too, unless
    /// they all have it already.
    fn group_color(&mut self, group: GroupId, color: Option<String>) -> Task<Message> {
        let task = self.apply(Command::SetGroupColor(group, color.clone()));
        let ask = self
            .model
            .workspaces()
            .iter()
            .any(|w| w.group == Some(group) && w.color != color);
        match &mut self.menu {
            Some(menu) if ask => {
                menu.sub = Some(Submenu {
                    page: MenuPage::MembersColor,
                    pinned: true,
                });
            }
            _ => self.menu = None,
        }
        task
    }

    /// A tab's menu item was picked.
    pub(crate) fn tab_menu(&mut self, tab: TabId, action: TabAction) -> Task<Message> {
        self.menu = None;
        self.tab_action(tab, action)
    }

    /// A workspace's menu item was picked.
    pub(crate) fn workspace_menu(
        &mut self,
        ws: WorkspaceId,
        action: WorkspaceAction,
    ) -> Task<Message> {
        self.menu = None;
        self.workspace_action(ws, action)
    }

    /// A group's menu item was picked. Picking a color keeps the menu open, to offer it to the
    /// group's workspaces.
    pub(crate) fn group_menu(&mut self, group: GroupId, action: GroupAction) -> Task<Message> {
        if let GroupAction::Color(color) = action {
            return self.group_color(group, color);
        }
        self.menu = None;
        self.group_action(group, action)
    }

    /// Escape: lets go of the focused item, then ends an item drag, or else closes the menu,
    /// or else the settings dialog, or else cancels a rename. One thing per press.
    pub(crate) fn dismiss(&mut self) -> Task<Message> {
        self.item_focus = None;
        // The search field first: Escape empties it and gives the keyboard back.
        if self.sidebar_search_focused {
            return self.sidebar_search_end();
        }
        // An open color picker closes before the dialog it's over.
        if let Some(settings) = &mut self.settings
            && settings.close_picker()
        {
            return Task::none();
        }
        if self.item_drag.take().is_none()
            && self.menu.take().is_none()
            && self.icon_picker.take().is_none()
            && self.folder_dialog.take().is_none()
            && self.settings.take().is_none()
        {
            self.renaming = None;
        }
        Task::none()
    }

    /// The new-tab menu's shell at `index` was picked.
    pub(crate) fn new_tab_with(&mut self, pane: PaneId, index: usize) -> Task<Message> {
        self.menu = None;
        let Some(choice) = self.shells.get(index).cloned() else {
            return Task::none();
        };
        self.new_tab_running(Some(pane), choice)
    }

    /// The new-tab menu's SSH profile at `index` was picked: a tab running `ssh` to it, with the
    /// server icon.
    pub(crate) fn new_ssh_tab(&mut self, pane: PaneId, index: usize) -> Task<Message> {
        self.menu = None;
        let Some(profile) = self.config.ssh.get(index).cloned() else {
            return Task::none();
        };
        let task = self.new_tab_running(Some(pane), ssh_shell(&profile));
        let Some(session) = self.focused_session() else {
            return task;
        };
        self.ssh_tabs.insert(session, profile.label().to_owned());
        let tab = self.model.focused_tab().id;
        Task::batch([
            task,
            self.apply(Command::SetTabIcon(tab, Some("server".to_owned()))),
        ])
    }

    /// Opens a new tab that installs Git Bash with winget.
    pub(crate) fn install_git_bash(&mut self) -> Task<Message> {
        self.offer_git_bash = false;
        let install = ShellOption {
            name: "Install Git Bash".into(),
            program: "powershell.exe".into(),
            args: vec![
                "-NoLogo".into(),
                "-NoExit".into(),
                "-Command".into(),
                INSTALL_GIT_BASH.into(),
            ],
        };
        self.new_tab_running(None, install)
    }

    /// Opens a tab (in `pane`, or the focused pane) whose terminal runs `shell`.
    fn new_tab_running(&mut self, pane: Option<PaneId>, shell: ShellOption) -> Task<Message> {
        let mut task = Task::none();
        if let Some(pane) = pane {
            task = self.apply(Command::FocusPane(pane));
        }
        let task = Task::batch([task, self.apply(Command::NewTab)]);
        // The new tab is focused and its session only queued, so the choice is in place before
        // `spawn` looks for it.
        if let Some(session) = self.focused_session() {
            self.tab_shells.insert(session, shell);
        }
        task
    }

    /// Opens the directory of the pane's active tab (else the home directory) in VS Code.
    /// Spawns `code`.
    pub(crate) fn open_vscode_in(&mut self, pane: PaneId) -> Task<Message> {
        self.menu = None;
        if let Some(code) = &self.vscode {
            let dir = self
                .model
                .pane(pane)
                .and_then(|p| self.model.tab(p.active))
                .and_then(|t| t.cwd.clone())
                .map(PathBuf::from)
                .or_else(home_dir);
            open_vscode(code, dir);
        }
        Task::none()
    }

    /// The directory of the pane's active tab, else its workspace's folder, else home.
    fn pane_dir(&self, pane: PaneId) -> Option<PathBuf> {
        let tab = self.model.pane(pane).and_then(|p| self.model.tab(p.active));
        tab.and_then(|t| t.cwd.clone())
            .or_else(|| {
                let ws = self.workspace_of_tab(tab?.id)?;
                self.model.workspace(ws)?.cwd.clone()
            })
            .map(PathBuf::from)
            .or_else(home_dir)
    }

    /// Opens VS Code for the web at the pane's directory in a new browser tab there, once its
    /// server listens (starting it first if needed).
    pub(crate) fn vscode_tab(&mut self, pane: PaneId) -> Task<Message> {
        self.menu = None;
        if !self.vscode_web.installed {
            return Task::none();
        }
        let folder = self.pane_dir(pane);
        self.vscode_web.error = None;
        if self.vscode_web.ensure_started().is_some() {
            // Opened by `vscode_ready`.
            self.vscode_web.waiting.push((pane, folder));
            return Task::none();
        }
        if self.vscode_web.server.is_none() {
            let error = self.vscode_web.error.clone().unwrap_or_default();
            self.config_errors.push(error);
            return Task::none();
        }
        self.open_vscode_tab(pane, folder.as_deref())
    }

    /// A browser tab in `pane` showing VS Code at `folder`, with the code icon. The server is
    /// running.
    fn open_vscode_tab(&mut self, pane: PaneId, folder: Option<&std::path::Path>) -> Task<Message> {
        let Some(url) = self.vscode_web.server.as_ref().map(|s| s.url(folder)) else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        if self.model.pane(pane).is_some() {
            tasks.push(self.apply(Command::FocusPane(pane)));
        }
        tasks.push(self.apply(Command::OpenBrowser { url, split: None }));
        let tab = self.model.focused_tab().id;
        tasks.push(self.apply(Command::SetTabIcon(tab, Some("code".to_owned()))));
        Task::batch(tasks)
    }

    /// The server listens (or gave up): open the tabs that waited for it.
    pub(crate) fn vscode_ready(&mut self, listening: bool) -> Task<Message> {
        if !listening {
            self.vscode_web.stop();
            let log = vscode::dir().join("server.log");
            let error = format!("VS Code didn't start. Its log: {}", log.display());
            self.vscode_web.error = Some(error.clone());
            self.config_errors.push(error);
            return Task::none();
        }
        self.vscode_web.ready = true;
        let waiting = std::mem::take(&mut self.vscode_web.waiting);
        Task::batch(
            waiting
                .into_iter()
                .map(|(pane, folder)| self.open_vscode_tab(pane, folder.as_deref()))
                .collect::<Vec<_>>(),
        )
    }

    /// Downloads (`true`) or removes VS Code on a thread; [`Message::VsCodeInstalled`] reports.
    /// Removing stops the server first, so no file is in use.
    pub(crate) fn vscode_install(&mut self, install: bool) -> Task<Message> {
        if self.vscode_web.busy {
            return Task::none();
        }
        self.vscode_web.busy = true;
        self.vscode_web.error = None;
        if !install {
            self.vscode_web.stop();
        }
        Task::future(vscode::off_thread(move || {
            if install {
                vscode::install().map(|()| true)
            } else {
                vscode::uninstall().map(|()| false)
            }
        }))
        .map(Message::VsCodeInstalled)
    }

    /// A download or removal finished.
    pub(crate) fn vscode_installed(&mut self, result: Result<bool, String>) -> Task<Message> {
        self.vscode_web.busy = false;
        if let Err(error) = &result {
            self.vscode_web.error = Some(error.clone());
        }
        self.vscode_web.installed = vscode::installed();
        Task::none()
    }

    /// Opens the home page in a new browser tab in `pane`.
    pub(crate) fn new_browser_in(&mut self, pane: PaneId) -> Task<Message> {
        self.menu = None;
        let home = self.config.browser.home.clone();
        Task::batch([
            self.apply(Command::FocusPane(pane)),
            self.apply(Command::OpenBrowser {
                url: home,
                split: None,
            }),
        ])
    }
}
