//! Tab commands: creating, ordering, pinning, moving and closing tabs.
//!
//! Creating a tab is the only place [`Effect::SpawnSession`] and [`Effect::OpenBrowser`] are
//! emitted, and [`Model::remove_tab`] the only place their kill and close counterparts are (the
//! quit path in `workspaces.rs` aside). Moving a tab ([`Model::move_tab`]) only re-parents its
//! id, so the session or browser keeps running. Every insertion goes through
//! [`Model::insert_tab_at`], which keeps pinned tabs first.

use super::{Effect, Model, MoveTo, Tab, TabKind};
use crate::ids::{BrowserId, PaneId, SessionId, TabId, WorkspaceId};
use crate::layout::Direction;

impl Model {
    /// A new terminal tab, not yet in any pane, and the effect that spawns its session.
    pub(super) fn new_terminal_tab(&mut self, effects: &mut Vec<Effect>) -> TabId {
        let session = SessionId(self.next_id());
        let id = TabId(self.next_id());
        self.tabs
            .insert(id, blank_tab(id, TabKind::Terminal(session), None));
        effects.push(Effect::SpawnSession(session));
        id
    }

    /// A new browser tab showing `url`, not yet in any pane, and the effect that opens it.
    pub(super) fn new_browser_tab(&mut self, url: &str, effects: &mut Vec<Effect>) -> TabId {
        let browser = BrowserId(self.next_id());
        let id = TabId(self.next_id());
        self.tabs.insert(
            id,
            blank_tab(id, TabKind::Browser(browser), Some(url.to_owned())),
        );
        effects.push(Effect::OpenBrowser(browser));
        id
    }

    /// Adds a terminal tab to the focused pane, after its active tab, and activates it.
    pub(super) fn new_tab(&mut self, effects: &mut Vec<Effect>) {
        let pane = self.active_workspace().focused;
        let tab = self.new_terminal_tab(effects);
        self.insert_tab(pane, tab);
    }

    /// Opens a browser tab in the focused pane, or in a new pane on the given side of it.
    pub(super) fn open_browser(
        &mut self,
        url: &str,
        split: Option<Direction>,
        effects: &mut Vec<Effect>,
    ) {
        let tab = self.new_browser_tab(url, effects);
        match split {
            Some(direction) => self.split_with(direction, tab),
            None => self.insert_tab(self.active_workspace().focused, tab),
        }
    }

    /// Opens a browser tab right after `beside` in its pane, if `beside` exists. Nothing is
    /// created (and no effect emitted) otherwise.
    pub(super) fn open_browser_beside(
        &mut self,
        beside: TabId,
        url: &str,
        effects: &mut Vec<Effect>,
    ) {
        if let Some(pane) = self.pane_of_tab(beside) {
            let tab = self.new_browser_tab(url, effects);
            let at = self.panes[&pane]
                .tabs
                .iter()
                .position(|t| *t == beside)
                .map_or(usize::MAX, |i| i + 1);
            self.insert_tab_at(pane, tab, at);
        }
    }

    /// Puts `tab` after the active tab of `pane` and activates it, keeping pinned tabs first.
    ///
    /// # Panics
    ///
    /// If `pane` doesn't exist.
    pub(super) fn insert_tab(&mut self, pane: PaneId, tab: TabId) {
        let after = self.panes[&pane]
            .tabs
            .iter()
            .position(|t| *t == self.panes[&pane].active)
            .map_or(usize::MAX, |i| i + 1);
        self.insert_tab_at(pane, tab, after);
    }

    /// Puts `tab` at `index` in `pane`, moved as little as needed to keep pinned tabs first,
    /// and activates it. A pinned tab lands at most at the end of the pinned run, an unpinned
    /// one at least just after it; `usize::MAX` means the end of its run.
    ///
    /// # Panics
    ///
    /// If `tab` or `pane` doesn't exist.
    pub(super) fn insert_tab_at(&mut self, pane: PaneId, tab: TabId, index: usize) {
        let pinned_tab = self.tabs[&tab].pinned;
        let pane = self
            .panes
            .get_mut(&pane)
            .expect("the target pane is in the model");
        let pinned = pane
            .tabs
            .iter()
            .take_while(|t| self.tabs[*t].pinned)
            .count();
        let at = if pinned_tab {
            index.min(pinned)
        } else {
            index.clamp(pinned, pane.tabs.len())
        };
        pane.tabs.insert(at, tab);
        pane.active = tab;
    }

    /// Takes `tab` out of `pane`, activating the tab now in its place (or the last one) if it
    /// was active, and returns where it was. Leaves `pane` empty if `tab` was its only tab, for
    /// the caller to close or detach. The tab stays in the tab map.
    ///
    /// # Panics
    ///
    /// If `pane` doesn't exist or doesn't hold `tab`.
    fn take_tab(&mut self, pane: PaneId, tab: TabId) -> usize {
        let pane = self
            .panes
            .get_mut(&pane)
            .expect("the pane holding the tab is in the model");
        let index = pane
            .tabs
            .iter()
            .position(|t| *t == tab)
            .expect("the tab is in its pane");
        pane.tabs.remove(index);
        // An emptied pane keeps its stale `active`; it's about to be closed or detached.
        if pane.active == tab && !pane.tabs.is_empty() {
            pane.active = pane.tabs[index.min(pane.tabs.len() - 1)];
        }
        index
    }

    /// Activates the tab and focuses its pane and workspace.
    pub(super) fn select_tab(&mut self, tab: TabId) {
        if let Some(pane_id) = self.pane_of_tab(tab) {
            self.panes
                .get_mut(&pane_id)
                .expect("the pane holding the tab is in the model")
                .active = tab;
            self.focus_pane(pane_id);
        }
    }

    /// Runs `edit` on the tab, if it exists.
    pub(super) fn edit_tab(&mut self, tab: TabId, edit: impl FnOnce(&mut Tab)) {
        if let Some(tab) = self.tabs.get_mut(&tab) {
            edit(tab);
        }
    }

    /// Mutes or unmutes the tab; muting clears its attention.
    pub(super) fn set_tab_muted(&mut self, tab: TabId, muted: bool) {
        self.edit_tab(tab, |tab| {
            tab.muted = muted;
            if muted {
                tab.attention = None;
            }
        });
    }

    /// Pins or unpins the tab, moving it to the end of the pinned tabs or the start of the
    /// rest. The pane's active tab doesn't change. A no-op when the flag doesn't change, which
    /// restoring a session relies on (it replays pins onto tabs in their saved order).
    pub(super) fn set_pinned(&mut self, tab: TabId, pinned: bool) {
        let Some(pane) = self.pane_of_tab(tab) else {
            return;
        };
        if self.tabs[&tab].pinned == pinned {
            return;
        }
        // `take_tab` and `insert_tab_at` both move the active tab; pinning shouldn't.
        let active = self.panes[&pane].active;
        self.take_tab(pane, tab);
        self.tabs
            .get_mut(&tab)
            .expect("every tab in a pane is in the model")
            .pinned = pinned;
        self.insert_tab_at(pane, tab, if pinned { usize::MAX } else { 0 });
        self.panes
            .get_mut(&pane)
            .expect("the pane holding the tab is in the model")
            .active = active;
    }

    /// Activates the tab `step` places away in the focused pane, wrapping around (`rem_euclid`
    /// keeps a negative step in range).
    pub(super) fn cycle_tab(&mut self, step: isize) {
        let pane_id = self.active_workspace().focused;
        let pane = self
            .panes
            .get_mut(&pane_id)
            .expect("the focused pane is in the model");
        let n = pane.tabs.len() as isize;
        let i = pane
            .tabs
            .iter()
            .position(|t| *t == pane.active)
            .unwrap_or(0) as isize;
        pane.active = pane.tabs[(i + step).rem_euclid(n) as usize];
    }

    /// Moves a running tab to another place; see [`MoveTo`]. Pointless moves are ignored, as is
    /// a tab that doesn't exist. Emits nothing.
    pub(super) fn move_tab(&mut self, tab: TabId, to: MoveTo) {
        let Some(from) = self.pane_of_tab(tab) else {
            return;
        };
        let from_ws = self
            .workspace_of_pane(from)
            .expect("every pane is in a workspace");
        let alone = self.panes[&from].tabs.len() == 1;
        if self.move_is_pointless(from, from_ws, alone, to) {
            return;
        }
        if let MoveTo::Pane { pane, index } = to
            && pane == from
        {
            self.reorder_tab(from_ws, from, tab, index);
            return;
        }
        // Placed before the emptied pane is detached, so if that removes the source workspace,
        // the destination's workspace is already active and stays so.
        self.take_tab(from, tab);
        self.put_moved_tab(tab, to);
        if alone {
            self.detach_empty_pane(from_ws, from);
        }
    }

    /// Whether moving a tab out of `from` (in `from_ws`, and `alone` there) to `to` would do
    /// nothing useful: the target is missing, or it would leave the layout as it was.
    ///
    /// The "as it was" cases: into its own workspace; a lone tab of a lone pane to a new
    /// workspace or along its own workspace's edge; splitting a pane's only tab off itself.
    fn move_is_pointless(
        &self,
        from: PaneId,
        from_ws: WorkspaceId,
        alone: bool,
        to: MoveTo,
    ) -> bool {
        let only_pane = self.is_only_pane(from_ws, from);
        match to {
            MoveTo::Workspace(ws) => ws == from_ws || self.workspace(ws).is_none(),
            MoveTo::NewWorkspace => alone && only_pane,
            MoveTo::Pane { pane, .. } => !self.panes.contains_key(&pane),
            MoveTo::SplitPane { pane, .. } => {
                !self.panes.contains_key(&pane) || (pane == from && alone)
            }
            MoveTo::SplitWorkspace { workspace, .. } => {
                self.workspace(workspace).is_none() || (workspace == from_ws && alone && only_pane)
            }
        }
    }

    /// Moves the tab to `index` within its own pane and focuses that pane.
    fn reorder_tab(&mut self, ws: WorkspaceId, pane: PaneId, tab: TabId, index: usize) {
        let at = self.take_tab(pane, tab);
        // `index` counts the tab itself, which is no longer before what follows it.
        self.insert_tab_at(pane, tab, if index > at { index - 1 } else { index });
        self.focus(ws, pane);
    }

    /// Puts a tab already taken out of its pane at `to`, focusing it there and making its
    /// workspace active.
    ///
    /// # Panics
    ///
    /// If the target doesn't exist; [`Model::move_is_pointless`] rules that out first.
    fn put_moved_tab(&mut self, tab: TabId, to: MoveTo) {
        match to {
            MoveTo::Workspace(ws) => {
                let target = self
                    .workspace(ws)
                    .expect("the target workspace is in the model")
                    .focused;
                self.insert_tab(target, tab);
                self.active = ws;
            }
            MoveTo::NewWorkspace => {
                let pane = self.pane_with(tab);
                self.push_workspace(pane);
            }
            MoveTo::Pane { pane, index } => {
                self.insert_tab_at(pane, tab, index);
                let ws = self
                    .workspace_of_pane(pane)
                    .expect("every pane is in a workspace");
                self.focus(ws, pane);
            }
            MoveTo::SplitPane { pane, direction } => {
                let ws = self
                    .workspace_of_pane(pane)
                    .expect("every pane is in a workspace");
                // `split_with` splits the active workspace's focused pane, so aim it first.
                self.focus(ws, pane);
                self.split_with(direction, tab);
            }
            MoveTo::SplitWorkspace {
                workspace,
                direction,
            } => self.split_workspace_with(workspace, direction, tab),
        }
    }

    /// Closes the tab, and its pane if it was the pane's only tab (and so on up to the
    /// workspace, or quitting). Ignored when the tab doesn't exist.
    pub(super) fn close_tab(&mut self, tab: TabId, effects: &mut Vec<Effect>) {
        let Some(pane_id) = self.pane_of_tab(tab) else {
            return;
        };
        if self.panes[&pane_id].tabs.len() == 1 {
            let ws = self
                .workspace_of_pane(pane_id)
                .expect("every pane is in a workspace");
            self.close_pane(ws, pane_id, effects);
            return;
        }
        self.take_tab(pane_id, tab);
        self.remove_tab(tab, effects);
    }

    /// Drops the tab from the model, emitting [`Effect::KillSession`] or
    /// [`Effect::CloseBrowser`]. The caller has already taken it out of its pane.
    pub(super) fn remove_tab(&mut self, tab: TabId, effects: &mut Vec<Effect>) {
        if let Some(tab) = self.tabs.remove(&tab) {
            match tab.kind {
                TabKind::Terminal(session) => effects.push(Effect::KillSession(session)),
                TabKind::Browser(browser) => effects.push(Effect::CloseBrowser(browser)),
            }
        }
    }
}

/// A tab with nothing set beyond its kind and, for a browser, its address.
fn blank_tab(id: TabId, kind: TabKind, url: Option<String>) -> Tab {
    Tab {
        id,
        title: None,
        name: None,
        kind,
        attention: None,
        agent: None,
        cwd: None,
        url,
        muted: false,
        pinned: false,
        icon: None,
        color: None,
    }
}
