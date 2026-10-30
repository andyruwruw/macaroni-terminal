//! Pane commands: splitting, closing, focus and divider ratios.
//!
//! Panes live in the model's pane map and are placed by being leaves of a workspace's
//! [`SplitNode`]. These methods keep the two in step: a pane is created with its tab
//! ([`Model::pane_with`]) before it's spliced into a layout, and removed from the layout
//! ([`Model::remove_leaf`]) together with the map. Tree surgery and geometry are in
//! [`crate::layout`]; closing tabs (which can close a pane) is in `tabs.rs`.

use super::{Effect, Model, Pane};
use crate::ids::{PaneId, TabId, WorkspaceId};
use crate::layout::{Axis, Direction, MIN_RATIO, Rect, Side, SplitNode, neighbour};

impl Model {
    /// A new pane holding a new terminal, not yet in any layout.
    pub(super) fn new_pane(&mut self, effects: &mut Vec<Effect>) -> PaneId {
        let tab = self.new_terminal_tab(effects);
        self.pane_with(tab)
    }

    /// A new pane holding only `tab`, not yet in any layout.
    pub(super) fn pane_with(&mut self, tab: TabId) -> PaneId {
        let id = PaneId(self.next_id());
        self.panes.insert(
            id,
            Pane {
                id,
                tabs: vec![tab],
                active: tab,
            },
        );
        id
    }

    /// Makes `ws` the active workspace and focuses `pane` in it. The caller guarantees `pane`
    /// is in `ws`'s layout.
    ///
    /// # Panics
    ///
    /// If `ws` doesn't exist.
    pub(super) fn focus(&mut self, ws: WorkspaceId, pane: PaneId) {
        self.active = ws;
        self.workspace_mut(ws).focused = pane;
    }

    /// Focuses the pane and activates its workspace, if the pane exists.
    pub(super) fn focus_pane(&mut self, pane: PaneId) {
        if let Some(ws) = self.workspace_of_pane(pane) {
            self.focus(ws, pane);
        }
    }

    /// Focuses the nearest pane in a direction from the focused one, if there is one.
    ///
    /// Works in the unit square: ratios alone decide adjacency, so the result doesn't depend on
    /// the window's size.
    pub(super) fn focus_direction(&mut self, direction: Direction) {
        let ws = self.active_workspace();
        let panes = ws.layout.panes(Rect::UNIT);
        if let Some(target) = neighbour(&panes, ws.focused, direction) {
            self.workspace_mut(self.active).focused = target;
        }
    }

    /// Sets the ratio of the split at `path`, clamped to `MIN_RATIO..=1 - MIN_RATIO`; ignored
    /// if there's no split there or `ratio` isn't finite (a NaN would pass `clamp` through and
    /// break the range invariant).
    pub(super) fn set_split_ratio(&mut self, workspace: WorkspaceId, path: &[Side], ratio: f32) {
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == workspace)
            && let Some(SplitNode::Split { ratio: r, .. }) = w.layout.node_at_mut(path)
            && ratio.is_finite()
        {
            *r = ratio.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
        }
    }

    /// Splits the focused pane, putting a new pane with a new terminal on the given side.
    pub(super) fn split(&mut self, direction: Direction, effects: &mut Vec<Effect>) {
        let tab = self.new_terminal_tab(effects);
        self.split_with(direction, tab);
    }

    /// Splits the focused pane of the active workspace 50/50, putting a new, focused pane
    /// holding only `tab` on the given side.
    pub(super) fn split_with(&mut self, direction: Direction, tab: TabId) {
        let (axis, side) = axis_side(direction);
        let target = self.active_workspace().focused;
        let new = self.pane_with(tab);
        let ws = self.workspace_mut(self.active);
        ws.layout.split_leaf(target, axis, new, side);
        ws.focused = new;
    }

    /// Wraps the whole layout of `workspace` in an even split with a new pane holding only
    /// `tab` on the given side, and focuses that pane there, making `workspace` active.
    ///
    /// # Panics
    ///
    /// If `workspace` doesn't exist.
    pub(super) fn split_workspace_with(
        &mut self,
        workspace: WorkspaceId,
        direction: Direction,
        tab: TabId,
    ) {
        let new = self.pane_with(tab);
        let (axis, side) = axis_side(direction);
        let ws = self.workspace_mut(workspace);
        // The placeholder leaf is overwritten just below; it only lets the old tree be moved out.
        let old = std::mem::replace(&mut ws.layout, SplitNode::Leaf(new));
        let (first, second) = match side {
            Side::First => (SplitNode::Leaf(new), old),
            Side::Second => (old, SplitNode::Leaf(new)),
        };
        ws.layout = SplitNode::Split {
            axis,
            ratio: 0.5,
            first: Box::new(first),
            second: Box::new(second),
        };
        ws.focused = new;
        self.active = workspace;
    }

    /// Closes the focused pane of the active workspace.
    pub(super) fn close_focused_pane(&mut self, effects: &mut Vec<Effect>) {
        let pane = self.active_workspace().focused;
        self.close_pane(self.active, pane, effects);
    }

    /// Closes the pane and its tabs, or the whole workspace if it's the only pane there (which
    /// quits if it's the last workspace). Emits a kill or close for each tab.
    pub(super) fn close_pane(
        &mut self,
        ws_id: WorkspaceId,
        pane_id: PaneId,
        effects: &mut Vec<Effect>,
    ) {
        if self.is_only_pane(ws_id, pane_id) {
            self.close_workspace(ws_id, effects);
            return;
        }
        self.remove_leaf(ws_id, pane_id);
        self.remove_pane(pane_id, effects);
    }

    /// Takes a pane whose tabs have moved elsewhere out of its workspace, closing the workspace
    /// if that was its only pane. Never the last workspace: the tabs went to another. Emits
    /// nothing, since no tab is closed.
    ///
    /// # Panics
    ///
    /// If `ws_id` doesn't exist.
    pub(super) fn detach_empty_pane(&mut self, ws_id: WorkspaceId, pane_id: PaneId) {
        self.panes.remove(&pane_id);
        if self.is_only_pane(ws_id, pane_id) {
            let index = self
                .workspaces
                .iter()
                .position(|w| w.id == ws_id)
                .expect("the pane's workspace is in the model");
            self.remove_workspace(index);
            return;
        }
        self.remove_leaf(ws_id, pane_id);
    }

    /// Whether `pane_id` is the whole layout of the workspace.
    ///
    /// # Panics
    ///
    /// If `ws_id` doesn't exist.
    pub(super) fn is_only_pane(&self, ws_id: WorkspaceId, pane_id: PaneId) -> bool {
        self.workspace(ws_id)
            .expect("the pane's workspace is in the model")
            .layout
            == SplitNode::Leaf(pane_id)
    }

    /// Takes `pane_id` out of a layout that has other panes, focusing its neighbour instead if
    /// it was focused. Leaves the pane map alone.
    ///
    /// # Panics
    ///
    /// If `ws_id` doesn't exist or `pane_id` is its whole layout.
    fn remove_leaf(&mut self, ws_id: WorkspaceId, pane_id: PaneId) {
        let ws = self.workspace_mut(ws_id);
        // Found before the removal, while the sibling relationship still exists.
        let refocus = ws.layout.neighbour_of(pane_id);
        let layout = std::mem::replace(&mut ws.layout, SplitNode::Leaf(pane_id));
        ws.layout = layout
            .without(pane_id)
            .expect("the layout has panes besides this one");
        if ws.focused == pane_id {
            ws.focused = refocus.unwrap_or_else(|| ws.layout.leaves()[0]);
        }
    }

    /// Drops the pane and its tabs from the model, ending their sessions and browsers.
    pub(super) fn remove_pane(&mut self, pane: PaneId, effects: &mut Vec<Effect>) {
        if let Some(pane) = self.panes.remove(&pane) {
            for tab in pane.tabs {
                self.remove_tab(tab, effects);
            }
        }
    }
}

/// The split that puts a new pane on the given side: left and right split a row, up and down a
/// column, and left/up put the new pane first.
fn axis_side(direction: Direction) -> (Axis, Side) {
    match direction {
        Direction::Left => (Axis::Row, Side::First),
        Direction::Right => (Axis::Row, Side::Second),
        Direction::Up => (Axis::Column, Side::First),
        Direction::Down => (Axis::Column, Side::Second),
    }
}
