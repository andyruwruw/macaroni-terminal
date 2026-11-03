//! Windows: each has its own sidebar (workspaces, groups, the active one).
//!
//! The focused window's sidebar lives in the model's own fields, so every sidebar command works
//! on it unchanged; the others wait in `Model::parked`. [`Model::apply`] finds the window a
//! command is about ([`Model::window_for`]) and swaps it in ([`Model::swap_to`]) before running
//! it, then swaps the focused one back, unless the command was about showing something there
//! (selecting a tab or workspace), in which case that window takes focus and the app is told
//! ([`Effect::FocusWindow`]). Panes and tabs live in shared maps, so moving a workspace or tab
//! between windows only moves sidebar entries.

use super::{Command, Effect, Model, MoveTo, Parked, WindowView, Workspace};
use crate::ids::{PaneId, TabId, WindowId, WorkspaceId};

impl Model {
    /// The focused window.
    pub fn focused_window(&self) -> WindowId {
        self.window
    }

    /// Every window, in the order they were opened.
    pub fn windows(&self) -> Vec<WindowId> {
        let mut windows: Vec<WindowId> = self.parked.keys().copied().collect();
        windows.push(self.window);
        windows.sort_unstable();
        windows
    }

    /// One window's sidebar, if the window exists.
    pub fn window(&self, id: WindowId) -> Option<WindowView<'_>> {
        if id == self.window {
            return Some(WindowView {
                id,
                workspaces: &self.workspaces,
                groups: &self.groups,
                active: self.active,
            });
        }
        self.parked.get(&id).map(|p| WindowView {
            id,
            workspaces: &p.workspaces,
            groups: &p.groups,
            active: p.active,
        })
    }

    /// Every window's sidebar, in the order they were opened.
    pub(super) fn window_views(&self) -> Vec<WindowView<'_>> {
        self.windows()
            .into_iter()
            .filter_map(|w| self.window(w))
            .collect()
    }

    /// The window whose sidebar holds the workspace, if it exists.
    pub fn window_of_workspace(&self, ws: WorkspaceId) -> Option<WindowId> {
        self.window_views()
            .into_iter()
            .find(|w| w.workspaces.iter().any(|x| x.id == ws))
            .map(|w| w.id)
    }

    /// The window holding the pane, if it exists.
    pub fn window_of_pane(&self, pane: PaneId) -> Option<WindowId> {
        self.window_of_workspace(self.workspace_of_pane(pane)?)
    }

    /// The window holding the tab, if it exists.
    pub fn window_of_tab(&self, tab: TabId) -> Option<WindowId> {
        self.window_of_pane(self.pane_of_tab(tab)?)
    }

    /// The window `command` is about, which must be swapped in to run it: the one holding the
    /// workspace, group, pane or tab it names, else the focused one. `None` when what it names
    /// is gone, or (placing a workspace) it names things in two windows, so the command is
    /// ignored.
    pub(super) fn window_for(&self, command: &Command) -> Option<WindowId> {
        let ws = |id: &WorkspaceId| self.window_of_workspace(*id);
        let group = |id| {
            self.window_views()
                .into_iter()
                .find(|w| w.groups.iter().any(|g| g.id == id))
                .map(|w| w.id)
        };
        let tab = |id: &TabId| self.window_of_tab(*id);
        match command {
            Command::CloseWorkspace(id)
            | Command::SelectWorkspace(id)
            | Command::RenameWorkspace(id, _)
            | Command::MoveWorkspace(id, _)
            | Command::SetWorkspaceMuted(id, _)
            | Command::SetWorkspacePinned(id, _)
            | Command::SetWorkspaceIcon(id, _)
            | Command::SetWorkspaceColor(id, _)
            | Command::SetWorkspaceCwd(id, _)
            | Command::SetWorkspaceEnv(id, _)
            | Command::SetWorkspaceStartup(id, _)
            | Command::ShiftWorkspace(id, _)
            | Command::NewGroup(id)
            | Command::CombineWorkspaces { into: id, .. }
            | Command::SetSplitRatio { workspace: id, .. } => ws(id),
            Command::SetWorkspaceGroup(id, g) | Command::PlaceWorkspace { id, group: g, .. } => {
                let window = ws(id)?;
                // A group in another window can't take the workspace.
                if g.is_some_and(|g| group(g) != Some(window)) {
                    return None;
                }
                Some(window)
            }
            Command::RenameGroup(id, _)
            | Command::SetGroupCollapsed(id, _)
            | Command::SetGroupColor(id, _)
            | Command::SetGroupIcon(id, _)
            | Command::SetGroupPinned(id, _)
            | Command::ShiftGroup(id, _)
            | Command::Ungroup(id)
            | Command::MergeGroups { into: id, .. } => group(*id),
            Command::FocusPane(pane) => self.window_of_pane(*pane),
            Command::CloseTab(id)
            | Command::SelectTab(id)
            | Command::SetTabPinned(id, _)
            | Command::MoveTab { tab: id, .. }
            | Command::OpenBrowserBeside { beside: id, .. } => tab(id),
            Command::SessionExited(session) => tab(&self.tab_for_session(*session)?),
            Command::BrowserClosed(browser) => tab(&self.tab_for_browser(*browser)?),
            Command::CloseWindow(id) | Command::FocusWindow(id) => self.window(*id).map(|w| w.id),
            Command::MoveWorkspaceToWindow { workspace, .. } => ws(workspace),
            _ => Some(self.window),
        }
    }

    /// Makes `to` the focused window: parks the current sidebar and takes out `to`'s. Does
    /// nothing for the focused window or a missing one.
    pub(super) fn swap_to(&mut self, to: WindowId) {
        if to == self.window {
            return;
        }
        let Some(next) = self.parked.remove(&to) else {
            return;
        };
        let current = Parked {
            workspaces: std::mem::replace(&mut self.workspaces, next.workspaces),
            groups: std::mem::replace(&mut self.groups, next.groups),
            active: std::mem::replace(&mut self.active, next.active),
        };
        self.parked.insert(self.window, current);
        self.window = to;
    }

    /// Leaves the focused window, whose sidebar is now empty, for `to` (another existing
    /// window), and tells the app to close it. `apply` decides which window ends up focused.
    fn drop_window_for(&mut self, to: WindowId, effects: &mut Vec<Effect>) {
        let Some(next) = self.parked.remove(&to) else {
            return;
        };
        effects.push(Effect::CloseWindow(self.window));
        self.workspaces = next.workspaces;
        self.groups = next.groups;
        self.active = next.active;
        self.window = to;
    }

    /// The window to focus when the focused one goes: the first other one.
    fn next_window(&self) -> Option<WindowId> {
        self.parked.keys().next().copied()
    }

    /// Parks the focused window and makes a new, empty one focused, telling the app to open
    /// it. The caller fills its sidebar before the command ends.
    fn open_empty_window(&mut self, effects: &mut Vec<Effect>) -> WindowId {
        let id = WindowId(self.next_id());
        let current = Parked {
            workspaces: std::mem::take(&mut self.workspaces),
            groups: std::mem::take(&mut self.groups),
            active: self.active,
        };
        self.parked.insert(self.window, current);
        self.window = id;
        effects.push(Effect::OpenWindow(id));
        id
    }

    /// [`Command::NewWindow`].
    pub(super) fn new_window(&mut self, effects: &mut Vec<Effect>) {
        self.open_empty_window(effects);
        self.add_workspace(effects);
    }

    /// [`Command::CloseWindow`], run with the window focused: everything in it closes, or the
    /// app quits if it's the only window.
    pub(super) fn close_window(&mut self, id: WindowId, effects: &mut Vec<Effect>) {
        if id != self.window {
            return;
        }
        let Some(next) = self.next_window() else {
            effects.extend(self.sessions().map(Effect::KillSession));
            effects.extend(self.browsers().map(Effect::CloseBrowser));
            effects.push(Effect::Quit);
            return;
        };
        let closing = std::mem::take(&mut self.workspaces);
        self.groups.clear();
        for ws in closing {
            for pane in ws.layout.leaves() {
                self.remove_pane(pane, effects);
            }
        }
        self.drop_window_for(next, effects);
    }

    /// Closes the focused window, whose last workspace `close_workspace` was asked to close,
    /// when another window exists. Returns false (doing nothing) when it's the only window.
    pub(super) fn close_last_workspace(&mut self, effects: &mut Vec<Effect>) -> bool {
        if self.parked.is_empty() {
            return false;
        }
        self.close_window(self.window, effects);
        true
    }

    /// [`Command::MoveWorkspaceToWindow`], run with the workspace's window focused.
    pub(super) fn move_workspace_to_window(
        &mut self,
        id: WorkspaceId,
        to: Option<WindowId>,
        effects: &mut Vec<Effect>,
    ) {
        let Some(index) = self.workspaces.iter().position(|w| w.id == id) else {
            return;
        };
        let alone = self.workspaces.len() == 1;
        match to {
            Some(w) if w == self.window || !self.parked.contains_key(&w) => return,
            None if alone => return,
            _ => {}
        }
        let mut ws = if alone {
            self.groups.clear();
            self.workspaces.remove(index)
        } else {
            self.remove_workspace(index)
        };
        ws.group = None;
        // The window it leaves is about to be parked, out of `apply`'s reach.
        self.tidy();
        match to {
            Some(target) if alone => self.drop_window_for(target, effects),
            Some(target) => {
                self.swap_to(target);
                effects.push(Effect::FocusWindow(target));
            }
            None => {
                self.open_empty_window(effects);
            }
        }
        self.add_moved_workspace(ws);
    }

    /// Puts a workspace from another window into the focused window's sidebar, after its last
    /// pinned or last unpinned workspace as it's pinned or not, and makes it active.
    fn add_moved_workspace(&mut self, ws: Workspace) {
        let at = if ws.pinned {
            self.workspaces.iter().take_while(|w| w.pinned).count()
        } else {
            self.workspaces.len()
        };
        self.active = ws.id;
        self.workspaces.insert(at, ws);
    }

    /// Moves a tab to a place in another window, or a new one ([`MoveTo::NewWindow`]), run with
    /// the tab's window focused; the target window takes focus. Returns false, doing nothing,
    /// when `to` is in the tab's own window (the ordinary move handles it).
    pub(super) fn move_tab_between_windows(
        &mut self,
        tab: TabId,
        to: MoveTo,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let target = match to {
            MoveTo::Workspace(ws) | MoveTo::SplitWorkspace { workspace: ws, .. } => {
                self.window_of_workspace(ws)
            }
            MoveTo::Pane { pane, .. } | MoveTo::SplitPane { pane, .. } => self.window_of_pane(pane),
            MoveTo::NewWindow => None,
            MoveTo::NewWorkspace => return false,
        };
        if target == Some(self.window) {
            return false;
        }
        // A target that's gone: nothing to do, but it's handled.
        if target.is_none() && !matches!(to, MoveTo::NewWindow) {
            return true;
        }
        let Some(from) = self.pane_of_tab(tab) else {
            return true;
        };
        let from_ws = self
            .workspace_of_pane(from)
            .expect("every pane is in a workspace");
        let alone = self.panes[&from].tabs.len() == 1;
        let empties_window =
            alone && self.is_only_pane(from_ws, from) && self.workspaces.len() == 1;
        // Alone in its window: a new window for it would be the same again.
        if empties_window && target.is_none() {
            return true;
        }
        self.take_tab(from, tab);
        if alone {
            if empties_window {
                self.panes.remove(&from);
                self.workspaces.clear();
                self.groups.clear();
            } else {
                self.detach_empty_pane(from_ws, from);
            }
        }
        // The window it leaves is about to be parked, out of `apply`'s reach.
        self.tidy();
        match target {
            Some(window) if empties_window => self.drop_window_for(window, effects),
            Some(window) => {
                self.swap_to(window);
                effects.push(Effect::FocusWindow(window));
            }
            None => {
                self.open_empty_window(effects);
            }
        }
        if matches!(to, MoveTo::NewWindow) {
            let pane = self.pane_with(tab);
            self.push_workspace(pane);
        } else {
            self.put_moved_tab(tab, to);
        }
        true
    }

    /// The tab `NextWaiting` shows: the waiting tab after the focused one when that's waiting
    /// too, else the oldest. Any window's.
    pub(super) fn next_waiting(&self) -> Option<TabId> {
        let waiting = self.waiting_tabs();
        let focused = self.focused_tab().id;
        let next = match waiting.iter().position(|t| *t == focused) {
            Some(at) => waiting.get(at + 1).or(waiting.first()),
            None => waiting.first(),
        };
        next.copied()
    }

    /// Sorts restored workspaces (`placed`: each one's id and window index, in spec order) into
    /// their windows, with each window's active workspace (`actives`, indices into `placed`)
    /// and the focused window. Windows with no workspaces are left out.
    pub(super) fn place_restored(
        &mut self,
        placed: &[(WorkspaceId, usize)],
        actives: &[usize],
        focused: usize,
    ) {
        let all = std::mem::take(&mut self.workspaces);
        let count = placed.iter().map(|(_, w)| w + 1).max().unwrap_or(1);
        let mut windows: Vec<(WindowId, Vec<Workspace>, Option<WorkspaceId>)> = (0..count)
            .map(|i| {
                let active = actives
                    .get(i)
                    .and_then(|&a| placed.get(a))
                    .filter(|(_, w)| *w == i)
                    .map(|(id, _)| *id);
                (WindowId(self.next_id()), Vec::new(), active)
            })
            .collect();
        for (ws, &(_, window)) in all.into_iter().zip(placed) {
            windows[window].1.push(ws);
        }
        let focused_index = focused.min(count - 1);
        let mut focus = None;
        for (index, (id, workspaces, active)) in windows.into_iter().enumerate() {
            let Some(first) = workspaces.first() else {
                continue;
            };
            let parked = Parked {
                active: active.unwrap_or(first.id),
                workspaces,
                groups: Vec::new(),
            };
            if index == focused_index || focus.is_none() {
                focus = Some(id);
            }
            self.parked.insert(id, parked);
        }
        // There's at least one window: `Model::restore` returns early without workspaces.
        let focus = focus.expect("a restored window");
        let first = self
            .parked
            .remove(&focus)
            .expect("the focused window is parked");
        self.workspaces = first.workspaces;
        self.active = first.active;
        self.window = focus;
    }
}
