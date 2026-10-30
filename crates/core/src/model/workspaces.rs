//! Workspace commands, and the sidebar as blocks that keep groups together.
//!
//! Reordering never moves single workspaces through the flat list. Instead
//! [`Model::take_blocks`] splits the list into blocks (each group's members, or one ungrouped
//! workspace), the command moves a workspace within a block or a whole block among the others,
//! and the blocks are concatenated back. [`insert_block`] and [`shift_block`] keep pinned blocks
//! first, so the group and pinning invariants hold by construction. `groups.rs` builds on the
//! same helpers.

use super::{Effect, Model, Shift, Workspace};
use crate::ids::{PaneId, TabId, WorkspaceId};
use crate::layout::{Axis, SplitNode};

impl Model {
    /// Makes the workspace active, if it exists.
    pub(super) fn select_workspace(&mut self, id: WorkspaceId) {
        if self.workspace(id).is_some() {
            self.active = id;
        }
    }

    /// Makes the workspace at a sidebar position active, if there is one.
    pub(super) fn select_workspace_index(&mut self, index: usize) {
        if let Some(w) = self.workspaces.get(index) {
            self.active = w.id;
        }
    }

    /// Activates the workspace `step` places away in the sidebar, wrapping around.
    pub(super) fn cycle_workspace(&mut self, step: isize) {
        let n = self.workspaces.len() as isize;
        let i = self
            .workspaces
            .iter()
            .position(|w| w.id == self.active)
            .unwrap_or(0) as isize;
        self.active = self.workspaces[(i + step).rem_euclid(n) as usize].id;
    }

    /// Renames the workspace; a blank name is ignored.
    pub(super) fn rename_workspace(&mut self, id: WorkspaceId, name: &str) {
        let name = name.trim();
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id)
            && !name.is_empty()
        {
            w.name = name.to_owned();
        }
    }

    /// Runs `edit` on the workspace, if it exists.
    pub(super) fn edit_workspace(&mut self, id: WorkspaceId, edit: impl FnOnce(&mut Workspace)) {
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id) {
            edit(w);
        }
    }

    /// Mutes or unmutes the workspace; muting clears the attention of every tab in it.
    pub(super) fn set_workspace_muted(&mut self, id: WorkspaceId, muted: bool) {
        self.edit_workspace(id, |w| w.muted = muted);
        if muted && let Some(w) = self.workspace(id) {
            let tabs: Vec<TabId> = w
                .layout
                .leaves()
                .iter()
                .flat_map(|p| self.panes[p].tabs.clone())
                .collect();
            for tab in tabs {
                self.tabs
                    .get_mut(&tab)
                    .expect("every tab in a pane is in the model")
                    .attention = None;
            }
        }
    }

    /// A new, active workspace holding a new terminal.
    pub(super) fn add_workspace(&mut self, effects: &mut Vec<Effect>) -> WorkspaceId {
        let pane = self.new_pane(effects);
        self.push_workspace(pane)
    }

    /// A new, active workspace holding `pane`.
    pub(super) fn push_workspace(&mut self, pane: PaneId) -> WorkspaceId {
        let id = WorkspaceId(self.next_id());
        self.workspaces_created += 1;
        self.workspaces.push(Workspace {
            id,
            name: format!("Workspace {}", self.workspaces_created),
            layout: SplitNode::Leaf(pane),
            focused: pane,
            muted: false,
            pinned: false,
            icon: None,
            color: None,
            group: None,
            cwd: None,
        });
        self.active = id;
        id
    }

    /// Closes the workspace and everything in it, or quits if it's the last one.
    ///
    /// Quitting leaves the model untouched and emits a kill or close for every tab, then
    /// [`Effect::Quit`]. Ignored when the workspace doesn't exist.
    pub(super) fn close_workspace(&mut self, id: WorkspaceId, effects: &mut Vec<Effect>) {
        let Some(index) = self.workspaces.iter().position(|w| w.id == id) else {
            return;
        };
        if self.workspaces.len() == 1 {
            effects.extend(self.sessions().map(Effect::KillSession));
            effects.extend(self.browsers().map(Effect::CloseBrowser));
            effects.push(Effect::Quit);
            return;
        }
        let ws = self.remove_workspace(index);
        for pane in ws.layout.leaves() {
            self.remove_pane(pane, effects);
        }
    }

    /// [`Command::CombineWorkspaces`](super::Command::CombineWorkspaces): `from`'s layout joins `into`'s on the right.
    pub(super) fn combine_workspaces(&mut self, into: WorkspaceId, from: WorkspaceId) {
        if into == from || self.workspace(into).is_none() {
            return;
        }
        let Some(index) = self.workspaces.iter().position(|w| w.id == from) else {
            return;
        };
        // Two workspaces exist, so removing one never empties the sidebar.
        let taken = self.remove_workspace(index);
        let ws = self.workspace_mut(into);
        // The placeholder leaf is overwritten just below; it only lets the old tree move out.
        let own = std::mem::replace(&mut ws.layout, SplitNode::Leaf(taken.focused));
        ws.layout = SplitNode::Split {
            axis: Axis::Row,
            ratio: 0.5,
            first: Box::new(own),
            second: Box::new(taken.layout),
        };
        ws.focused = taken.focused;
        self.active = into;
    }

    /// Takes the workspace at `index` out of the sidebar, activating its neighbour (the one
    /// that took its place, else the new last one) if it was active. Its panes and tabs stay in
    /// the model for the caller to remove or reuse.
    ///
    /// # Panics
    ///
    /// If `index` is out of range or this is the last workspace.
    pub(super) fn remove_workspace(&mut self, index: usize) -> Workspace {
        let ws = self.workspaces.remove(index);
        if self.active == ws.id {
            self.active = self.workspaces[index.min(self.workspaces.len() - 1)].id;
        }
        ws
    }

    /// The sidebar as blocks: each group's workspaces together, and each ungrouped workspace
    /// alone. Moving whole blocks keeps groups together.
    ///
    /// Leaves `self.workspaces` empty; the caller must put the (concatenated) blocks back.
    /// Every block is non-empty.
    pub(super) fn take_blocks(&mut self) -> Vec<Vec<Workspace>> {
        let mut blocks: Vec<Vec<Workspace>> = Vec::new();
        for w in std::mem::take(&mut self.workspaces) {
            match blocks.last_mut() {
                Some(last) if w.group.is_some() && last[0].group == w.group => last.push(w),
                _ => blocks.push(vec![w]),
            }
        }
        blocks
    }

    /// The block holding the workspace, and its position within that block, or `None` if no
    /// block holds it.
    pub(super) fn find_block(blocks: &[Vec<Workspace>], id: WorkspaceId) -> Option<(usize, usize)> {
        blocks
            .iter()
            .enumerate()
            .find_map(|(b, block)| block.iter().position(|w| w.id == id).map(|i| (b, i)))
    }

    /// Moves a grouped workspace a step within its group, or an ungrouped one past the next block.
    /// Ignored when the workspace doesn't exist.
    pub(super) fn shift_workspace(&mut self, id: WorkspaceId, shift: Shift) {
        let mut blocks = self.take_blocks();
        if let Some((b, i)) = Self::find_block(&blocks, id) {
            if blocks[b][0].group.is_some() {
                let block = &mut blocks[b];
                let w = block.remove(i);
                // Positions are within the block after removal, so `len` is the last slot.
                let at = match shift {
                    Shift::Up => i.saturating_sub(1),
                    Shift::Down => (i + 1).min(block.len()),
                    Shift::Top => 0,
                    Shift::Bottom => block.len(),
                };
                block.insert(at, w);
            } else {
                shift_block(&mut blocks, b, shift);
            }
        }
        self.workspaces = blocks.concat();
    }

    /// To a position in the sidebar: within its group if it has one, else to the nearest place
    /// between blocks, and never across the pinned/unpinned line.
    pub(super) fn move_workspace(&mut self, id: WorkspaceId, to: usize) {
        let mut blocks = self.take_blocks();
        let Some((b, i)) = Self::find_block(&blocks, id) else {
            self.workspaces = blocks.concat();
            return;
        };
        if blocks[b][0].group.is_some() {
            let start: usize = blocks[..b].iter().map(Vec::len).sum();
            let block = &mut blocks[b];
            let w = block.remove(i);
            // `to` counts the whole list; make it relative to the group and keep it inside.
            let at = to.saturating_sub(start).min(block.len());
            block.insert(at, w);
        } else {
            let block = blocks.remove(b);
            // The first block that starts at or after `to` (counting workspaces, not blocks),
            // so the workspace lands at the nearest block boundary and never inside a group.
            let mut seen = 0;
            let slot = blocks
                .iter()
                .position(|blk| {
                    let start = seen;
                    seen += blk.len();
                    start >= to
                })
                .unwrap_or(blocks.len());
            insert_block(&mut blocks, block, slot);
        }
        self.workspaces = blocks.concat();
    }

    /// Pins or unpins the workspace's block (its whole group, if it has one): to the end of
    /// the pinned ones, or the start of the rest. A no-op when the flag doesn't change.
    pub(super) fn set_pinned_block(&mut self, id: WorkspaceId, pinned: bool) {
        let mut blocks = self.take_blocks();
        if let Some((b, _)) = Self::find_block(&blocks, id)
            && blocks[b][0].pinned != pinned
        {
            let mut block = blocks.remove(b);
            for w in &mut block {
                w.pinned = pinned;
            }
            insert_block(&mut blocks, block, if pinned { usize::MAX } else { 0 });
        }
        self.workspaces = blocks.concat();
    }
}

/// Moves block `b` a step among the blocks with the same pinned state.
///
/// The target is picked by position within that section, then the block is inserted at the
/// section member's index in the full list. Since the pinned section is a prefix, the block
/// never crosses the pinned/unpinned line.
///
/// # Panics
///
/// If `b` is out of range.
pub(super) fn shift_block(blocks: &mut Vec<Vec<Workspace>>, b: usize, shift: Shift) {
    let pinned = blocks[b][0].pinned;
    let section: Vec<usize> = (0..blocks.len())
        .filter(|&i| blocks[i][0].pinned == pinned)
        .collect();
    let here = section
        .iter()
        .position(|&i| i == b)
        .expect("the block is in its pinned state's section");
    let target = match shift {
        Shift::Up => here.saturating_sub(1),
        Shift::Down => (here + 1).min(section.len() - 1),
        Shift::Top => 0,
        Shift::Bottom => section.len() - 1,
    };
    let block = blocks.remove(b);
    blocks.insert(section[target], block);
}

/// Inserts a block at position `slot` among the blocks, moved as little as needed to keep
/// pinned blocks first: a pinned block is clamped to at most the pinned count, an unpinned one
/// to at least it. `usize::MAX` means the end of its section and 0 the start.
///
/// # Panics
///
/// If `block` is empty.
pub(super) fn insert_block(blocks: &mut Vec<Vec<Workspace>>, block: Vec<Workspace>, slot: usize) {
    let pinned = blocks.iter().take_while(|b| b[0].pinned).count();
    let at = if block[0].pinned {
        slot.min(pinned)
    } else {
        slot.clamp(pinned, blocks.len())
    };
    blocks.insert(at, block);
}
