//! What a tab's, workspace's or group's menu items do.
//!
//! `update/menus.rs` closes the menu and calls these; each action becomes a model command, a
//! rename, a clipboard write, or a `cd` typed into a shell.
//!
//! A workspace's folder (`Workspace.cwd`) is where its new terminals start; "going" there types
//! `cd "<folder>"` and Enter into a shell, so it's only done where a shell is likely at its
//! prompt (see [`App::cd_tab`]).

use std::path::Path;

use iced::Task;
use macaroni_core::{Command, GroupId, TabId, TabKind, WorkspaceId};

use crate::message::{GroupAction, Message, TabAction, WorkspaceAction};
use crate::state::Renaming;
use crate::{App, home_dir};

/// Which of a tab's neighbours in its pane a bulk close takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Beside {
    /// Every other tab.
    Others,
    /// The tabs before it.
    Left,
    /// The tabs after it.
    Right,
}

impl App {
    /// Carries out a tab menu item.
    pub(super) fn tab_action(&mut self, tab: TabId, action: TabAction) -> Task<Message> {
        match action {
            TabAction::Rename => self.start_rename(Renaming::Tab(tab)),
            TabAction::Move(to) => self.apply(Command::MoveTab { tab, to }),
            TabAction::Mute(muted) => self.apply(Command::SetTabMuted(tab, muted)),
            TabAction::Pin(pinned) => self.apply(Command::SetTabPinned(tab, pinned)),
            TabAction::CopyId => iced::clipboard::write(tab.0.to_string()),
            TabAction::Icon(icon) => self.apply(Command::SetTabIcon(tab, icon.map(str::to_owned))),
            TabAction::Color(color) => {
                self.apply(Command::SetTabColor(tab, color.map(str::to_owned)))
            }
            TabAction::Close => self.apply(Command::CloseTab(tab)),
            TabAction::CloseOthers => self.close_beside(tab, Beside::Others),
            TabAction::CloseLeft => self.close_beside(tab, Beside::Left),
            TabAction::CloseRight => self.close_beside(tab, Beside::Right),
            TabAction::UseAsFolder => {
                let cwd = self.model.tab(tab).and_then(|t| t.cwd.clone());
                match self.workspace_of_tab(tab) {
                    Some(ws) if cwd.is_some() => self.apply(Command::SetWorkspaceCwd(ws, cwd)),
                    _ => Task::none(),
                }
            }
            TabAction::GoToFolder => {
                if let Some(folder) = self
                    .workspace_of_tab(tab)
                    .map(|ws| self.workspace_folder(ws))
                {
                    self.cd_tab(tab, &folder);
                }
                Task::none()
            }
        }
    }

    /// Closes the unpinned tabs on `side` of `tab` in its pane. Pinned tabs stay: pinning is
    /// how a tab says it should outlive tidying up.
    fn close_beside(&mut self, tab: TabId, side: Beside) -> Task<Message> {
        let Some(pane) = self.model.pane_of_tab(tab).and_then(|p| self.model.pane(p)) else {
            return Task::none();
        };
        let at = pane.tabs.iter().position(|t| *t == tab).unwrap_or(0);
        let doomed: Vec<TabId> = pane
            .tabs
            .iter()
            .enumerate()
            .filter(|&(i, _)| match side {
                Beside::Others => i != at,
                Beside::Left => i < at,
                Beside::Right => i > at,
            })
            .map(|(_, t)| *t)
            .filter(|t| self.model.tab(*t).is_some_and(|t| !t.pinned))
            .collect();
        let tasks: Vec<_> = doomed
            .into_iter()
            .map(|t| self.apply(Command::CloseTab(t)))
            .collect();
        Task::batch(tasks)
    }

    /// The workspace's folder: its own, else `[terminal] default_dir`, else the home directory.
    pub(crate) fn workspace_folder(&self, ws: WorkspaceId) -> String {
        self.model
            .workspace(ws)
            .and_then(|w| w.cwd.clone())
            .or_else(|| self.default_folder())
            .or_else(|| home_dir().map(|h| h.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    /// `[terminal] default_dir` with a leading `~` expanded; `None` when it's blank.
    pub(crate) fn default_folder(&self) -> Option<String> {
        let dir = self.config.terminal.default_dir.trim();
        let rest = dir.strip_prefix('~');
        match (rest, home_dir()) {
            (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                Some(format!("{}{rest}", home.to_string_lossy()))
            }
            _ => Some(dir.to_owned()).filter(|d| !d.is_empty()),
        }
    }

    /// Whether `cd` can be typed into `tab` now: a terminal whose shell is likely at its prompt,
    /// so not one running a coding agent or a full-screen program, and not one already there.
    pub(crate) fn can_cd(&self, tab: TabId, folder: &str) -> bool {
        let Some(t) = self.model.tab(tab) else {
            return false;
        };
        let TabKind::Terminal(session) = t.kind else {
            return false;
        };
        let at_prompt = self
            .running
            .get(&session)
            .is_some_and(|r| !r.snapshot.modes.alt_screen);
        at_prompt
            && t.agent.is_none()
            && cd_command(folder).is_some()
            && !t.cwd.as_deref().is_some_and(|cwd| same_dir(cwd, folder))
    }

    /// Types `cd "<folder>"` and Enter into `tab`'s shell, if [`App::can_cd`].
    fn cd_tab(&self, tab: TabId, folder: &str) {
        if !self.can_cd(tab, folder) {
            return;
        }
        let (Some(TabKind::Terminal(session)), Some(command)) = (
            self.model.tab(tab).map(|t| t.kind.clone()),
            cd_command(folder),
        ) else {
            return;
        };
        if let Some(running) = self.running.get(&session) {
            running.session.write(command.as_bytes());
        }
    }

    /// Carries out a workspace menu item.
    pub(super) fn workspace_action(
        &mut self,
        ws: WorkspaceId,
        action: WorkspaceAction,
    ) -> Task<Message> {
        match action {
            WorkspaceAction::Rename => self.start_rename(Renaming::Workspace(ws)),
            WorkspaceAction::Shift(shift) => self.apply(Command::ShiftWorkspace(ws, shift)),
            WorkspaceAction::Mute(muted) => self.apply(Command::SetWorkspaceMuted(ws, muted)),
            WorkspaceAction::Pin(pinned) => self.apply(Command::SetWorkspacePinned(ws, pinned)),
            WorkspaceAction::CopyId => iced::clipboard::write(ws.0.to_string()),
            WorkspaceAction::Close => self.apply(Command::CloseWorkspace(ws)),
            WorkspaceAction::Icon(icon) => {
                self.apply(Command::SetWorkspaceIcon(ws, icon.map(str::to_owned)))
            }
            WorkspaceAction::Color(color) => {
                self.apply(Command::SetWorkspaceColor(ws, color.map(str::to_owned)))
            }
            WorkspaceAction::NewGroup => {
                let task = self.apply(Command::NewGroup(ws));
                // Name it right away.
                match self.model.workspace(ws).and_then(|w| w.group) {
                    Some(group) => {
                        Task::batch([task, self.group_action(group, GroupAction::Rename)])
                    }
                    None => task,
                }
            }
            WorkspaceAction::Group(group) => self.apply(Command::SetWorkspaceGroup(ws, group)),
            WorkspaceAction::Combine(from) => {
                self.apply(Command::CombineWorkspaces { into: ws, from })
            }
        }
    }

    /// Carries out a group menu item.
    pub(super) fn group_action(&mut self, group: GroupId, action: GroupAction) -> Task<Message> {
        match action {
            GroupAction::Rename => self.start_rename(Renaming::Group(group)),
            GroupAction::Color(color) => self.apply(Command::SetGroupColor(group, color)),
            GroupAction::ColorMembers => self.color_members(group),
            GroupAction::Icon(icon) => {
                self.apply(Command::SetGroupIcon(group, icon.map(str::to_owned)))
            }
            GroupAction::Collapse(collapsed) => {
                // Clicking a header sends this too, and a clicked item is the focused one: Enter
                // then renames it.
                self.item_focus = Some(Renaming::Group(group));
                self.apply(Command::SetGroupCollapsed(group, collapsed))
            }
            GroupAction::Shift(shift) => self.apply(Command::ShiftGroup(group, shift)),
            GroupAction::Pin(pinned) => self.apply(Command::SetGroupPinned(group, pinned)),
            GroupAction::Ungroup => self.apply(Command::Ungroup(group)),
            GroupAction::Merge(from) => self.apply(Command::MergeGroups { into: group, from }),
            GroupAction::Delete => self.delete_group(group),
        }
    }

    /// Closes every workspace in the group. When that's every workspace, a new one is opened
    /// first, since closing the last workspace quits.
    fn delete_group(&mut self, group: GroupId) -> Task<Message> {
        let members: Vec<WorkspaceId> = self
            .model
            .workspaces()
            .iter()
            .filter(|w| w.group == Some(group))
            .map(|w| w.id)
            .collect();
        let mut tasks = Vec::new();
        if members.len() == self.model.workspaces().len() {
            tasks.push(self.apply(Command::NewWorkspace));
        }
        for ws in members {
            tasks.push(self.apply(Command::CloseWorkspace(ws)));
        }
        Task::batch(tasks)
    }

    /// Gives every workspace in the group the group's color.
    fn color_members(&mut self, group: GroupId) -> Task<Message> {
        let color = self.model.group(group).and_then(|g| g.color.clone());
        let members: Vec<WorkspaceId> = self
            .model
            .workspaces()
            .iter()
            .filter(|w| w.group == Some(group))
            .map(|w| w.id)
            .collect();
        Task::batch(
            members
                .into_iter()
                .map(|ws| self.apply(Command::SetWorkspaceColor(ws, color.clone())))
                .collect::<Vec<_>>(),
        )
    }
}

/// The `cd` line for `folder`, or `None` when it has characters a shell would expand or that
/// would end the quotes (`"`, `$`, `` ` ``, `%`, newlines): typing those blind isn't safe. Double
/// quotes work the same in bash, zsh, PowerShell and cmd.
fn cd_command(folder: &str) -> Option<String> {
    let unsafe_char = |c: char| matches!(c, '"' | '$' | '`' | '%' | '\n' | '\r');
    (!folder.is_empty() && !folder.contains(unsafe_char)).then(|| format!("cd \"{folder}\"\r"))
}

/// Whether two directory paths name the same place, ignoring slash direction, a trailing
/// slash, and (on Windows) case.
pub(crate) fn same_dir(a: &str, b: &str) -> bool {
    let norm = |p: &str| {
        let p = p.replace('\\', "/");
        let p = p.trim_end_matches('/').to_owned();
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    norm(a) == norm(b) || Path::new(a) == Path::new(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cd_lines() {
        assert_eq!(cd_command("C:/work").as_deref(), Some("cd \"C:/work\"\r"));
        assert_eq!(cd_command("/tmp/a b").as_deref(), Some("cd \"/tmp/a b\"\r"));
        assert!(cd_command("/tmp/$HOME").is_none());
        assert!(cd_command("a\"; rm").is_none());
        assert!(cd_command("").is_none());
    }

    #[test]
    fn same_dirs() {
        assert!(same_dir("C:/Users/me/", "C:\\Users\\me"));
        assert!(!same_dir("/a", "/b"));
    }
}
