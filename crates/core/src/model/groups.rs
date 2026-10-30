//! Group commands, and moving workspaces into and out of groups.
//!
//! Groups are pure sidebar structure: a [`Group`] record plus a `group` field on each member
//! workspace. The ordering rules (members contiguous, all pinned or all not, pinned first) are
//! kept by working on the sidebar as blocks ([`Model::take_blocks`]) and reinserting with
//! [`insert_block`], both from `workspaces`. Empty groups are not removed here but by
//! [`Model::drop_empty_groups`] at the end of every [`Model::apply`].

use super::workspaces::{insert_block, shift_block};
use super::{Group, Model, Shift};
use crate::ids::{GroupId, WorkspaceId};

impl Model {
    /// A new group holding just this workspace, taken out of any group it was in (landing just
    /// after it). The group is named "Group N" and starts expanded with no color.
    pub(super) fn new_group(&mut self, id: WorkspaceId) {
        if self.workspace(id).is_none() {
            return;
        }
        self.set_workspace_group(id, None);
        self.groups_created += 1;
        let group = GroupId(self.next_id());
        self.groups.push(Group {
            id: group,
            name: format!("Group {}", self.groups_created),
            collapsed: false,
            color: None,
            icon: None,
        });
        self.workspace_mut(id).group = Some(group);
    }

    /// Moves the workspace into `group` (or out of any) just before `before`; see
    /// [`super::Command::PlaceWorkspace`].
    pub(super) fn place_workspace(
        &mut self,
        id: WorkspaceId,
        before: Option<WorkspaceId>,
        group: Option<GroupId>,
    ) {
        if before == Some(id) || group.is_some_and(|g| self.group(g).is_none()) {
            return;
        }
        // `take_blocks` empties `self.workspaces`, so every early return below puts this back.
        let original = self.workspaces.clone();
        let mut blocks = self.take_blocks();
        let Some((b, i)) = Self::find_block(&blocks, id) else {
            self.workspaces = original;
            return;
        };
        let mut w = blocks[b].remove(i);
        if blocks[b].is_empty() {
            blocks.remove(b);
        }
        match group {
            Some(group) => {
                // The group's only member can't be placed into it: taking it out removed the
                // group's block, so there's nothing left to find.
                let Some(target) = blocks.iter_mut().find(|blk| blk[0].group == Some(group)) else {
                    self.workspaces = original;
                    return;
                };
                let at = before
                    .and_then(|before| target.iter().position(|w| w.id == before))
                    .unwrap_or(target.len());
                w.group = Some(group);
                w.pinned = target[0].pinned;
                target.insert(at, w);
            }
            None => {
                w.group = None;
                // Before a block's first workspace is a slot between blocks. Before any later
                // member is inside a group, which an ungrouped workspace can't split, so it goes
                // just after that group instead.
                let slot = match before.and_then(|before| Self::find_block(&blocks, before)) {
                    Some((b, 0)) => b,
                    Some((b, _)) => b + 1,
                    None => blocks.len(),
                };
                insert_block(&mut blocks, vec![w], slot);
            }
        }
        self.workspaces = blocks.concat();
    }

    /// Moves the workspace to the end of `group`, taking its pinned state, or with `None` out
    /// of its group to just after it. Ignored when it's already there or `group` is missing.
    pub(super) fn set_workspace_group(&mut self, id: WorkspaceId, group: Option<GroupId>) {
        let Some(current) = self.workspace(id).map(|w| w.group) else {
            return;
        };
        if current == group || group.is_some_and(|g| self.group(g).is_none()) {
            return;
        }
        let mut blocks = self.take_blocks();
        let (b, i) = Self::find_block(&blocks, id).expect("the workspace is in the model");
        let mut w = blocks[b].remove(i);
        let emptied = blocks[b].is_empty();
        match group {
            Some(group) => {
                if emptied {
                    blocks.remove(b);
                }
                // `group` exists and isn't the workspace's own, so its block is untouched and
                // non-empty.
                let target = blocks
                    .iter_mut()
                    .find(|blk| blk[0].group == Some(group))
                    .expect("every group has workspaces");
                w.group = Some(group);
                w.pinned = target[0].pinned;
                target.push(w);
            }
            None => {
                // Just after the group it left. If it was the only member, its now empty block
                // stays in place and holds it, ungrouped.
                w.group = None;
                if emptied {
                    blocks[b].push(w);
                } else {
                    blocks.insert(b + 1, vec![w]);
                }
            }
        }
        self.workspaces = blocks.concat();
    }

    /// Renames the group; a blank name is ignored.
    pub(super) fn rename_group(&mut self, id: GroupId, name: &str) {
        let name = name.trim();
        if let Some(g) = self.groups.iter_mut().find(|g| g.id == id)
            && !name.is_empty()
        {
            g.name = name.to_owned();
        }
    }

    /// Runs `edit` on the group, if it exists.
    pub(super) fn edit_group(&mut self, id: GroupId, edit: impl FnOnce(&mut Group)) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.id == id) {
            edit(g);
        }
    }

    /// Pins or unpins every workspace in the group, moving the group as
    /// [`Model::set_pinned_block`] does.
    pub(super) fn set_group_pinned(&mut self, id: GroupId, pinned: bool) {
        if let Some(first) = self.workspaces.iter().find(|w| w.group == Some(id)) {
            self.set_pinned_block(first.id, pinned);
        }
    }

    /// Moves the whole group a step among the blocks with its pinned state; see [`shift_block`].
    pub(super) fn shift_group(&mut self, id: GroupId, shift: Shift) {
        let mut blocks = self.take_blocks();
        if let Some(b) = blocks.iter().position(|b| b[0].group == Some(id)) {
            shift_block(&mut blocks, b, shift);
        }
        self.workspaces = blocks.concat();
    }

    /// Dissolves the group, leaving its workspaces where they are. The group record itself is
    /// dropped by [`Model::drop_empty_groups`] at the end of the command.
    pub(super) fn ungroup(&mut self, id: GroupId) {
        for w in &mut self.workspaces {
            if w.group == Some(id) {
                w.group = None;
            }
        }
    }

    /// [`Command::MergeGroups`](super::Command::MergeGroups): each of `from`'s workspaces, in
    /// order, joins the end of `into` the way [`Command::SetWorkspaceGroup`](super::Command::SetWorkspaceGroup) would.
    pub(super) fn merge_groups(&mut self, into: GroupId, from: GroupId) {
        if into == from || self.group(into).is_none() {
            return;
        }
        let members: Vec<WorkspaceId> = self
            .workspaces
            .iter()
            .filter(|w| w.group == Some(from))
            .map(|w| w.id)
            .collect();
        for id in members {
            self.set_workspace_group(id, Some(into));
        }
    }

    /// Drops groups with no workspaces left: a group lasts while it has workspaces.
    pub(super) fn drop_empty_groups(&mut self) {
        let workspaces = &self.workspaces;
        self.groups
            .retain(|g| workspaces.iter().any(|w| w.group == Some(g.id)));
    }
}
