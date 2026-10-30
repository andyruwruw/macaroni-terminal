//! The structural invariants, checked after every command in debug builds.
//!
//! [`Model::check`] is the contract every command must keep, and the randomized test in
//! `tests.rs` asserts it after each step. It verifies:
//!
//! - Workspaces: there is at least one, the active one exists, and pinned ones come before
//!   unpinned ones.
//! - Groups: every group has at least one workspace, its workspaces are contiguous and all
//!   pinned or all unpinned, and every workspace's group exists.
//! - Layouts: every workspace focuses a leaf of its own layout, every leaf is a pane in the model
//!   and appears in only one layout, no pane is outside every layout, and every split ratio is
//!   strictly between 0 and 1.
//! - Panes: every pane has tabs, its active tab is one of them, every tab is in exactly one pane
//!   (no duplicates, none missing from the model, no orphans), and pinned tabs come first.

use std::collections::BTreeSet;

use super::Model;
use crate::layout::SplitNode;

impl Model {
    /// Verifies the structural invariants listed in the module docs. Used by tests and by debug
    /// builds after every command.
    ///
    /// Returns `Err` with a short description of the first violation found.
    pub fn check(&self) -> Result<(), String> {
        self.check_workspaces()?;
        self.check_groups()?;
        self.check_layouts()?;
        self.check_panes()
    }

    /// There are workspaces, the active one exists, and pinned ones come first.
    fn check_workspaces(&self) -> Result<(), String> {
        if self.workspaces.is_empty() {
            return Err("no workspaces".into());
        }
        if self.workspace(self.active).is_none() {
            return Err("active workspace missing".into());
        }
        // `false < true`, so sorting by "not pinned" means every pinned workspace comes first.
        if !self.workspaces.is_sorted_by_key(|w| !w.pinned) {
            return Err("a pinned workspace after an unpinned one".into());
        }
        Ok(())
    }

    /// Every group's workspaces are together and all pinned or all not, and every workspace's
    /// group exists.
    fn check_groups(&self) -> Result<(), String> {
        for group in &self.groups {
            let members: Vec<usize> = (0..self.workspaces.len())
                .filter(|&i| self.workspaces[i].group == Some(group.id))
                .collect();
            let (Some(&first), Some(&last)) = (members.first(), members.last()) else {
                return Err(format!("{:?} is empty", group.id));
            };
            // Contiguous exactly when the span from first to last member holds only members.
            if last - first + 1 != members.len() {
                return Err(format!("{:?} is split up", group.id));
            }
            if members
                .iter()
                .any(|&i| self.workspaces[i].pinned != self.workspaces[first].pinned)
            {
                return Err(format!("{:?} is partly pinned", group.id));
            }
        }
        if let Some(w) = self
            .workspaces
            .iter()
            .find(|w| w.group.is_some_and(|g| self.group(g).is_none()))
        {
            return Err(format!("{:?} is in a missing group", w.id));
        }
        Ok(())
    }

    /// Every pane is in exactly one layout, each layout focuses one of its own panes, and every
    /// ratio is in range.
    fn check_layouts(&self) -> Result<(), String> {
        let mut seen_panes = BTreeSet::new();
        for w in &self.workspaces {
            let leaves = w.layout.leaves();
            if !leaves.contains(&w.focused) {
                return Err(format!("{:?} focuses a pane outside its layout", w.id));
            }
            for pane in leaves {
                if !seen_panes.insert(pane) {
                    return Err(format!("{pane:?} appears twice"));
                }
                if !self.panes.contains_key(&pane) {
                    return Err(format!("{pane:?} is in a layout but not in the model"));
                }
            }
            check_ratios(&w.layout)?;
        }
        // Every leaf was found in the pane map above, so a size mismatch means panes the map
        // holds that no layout reaches.
        if seen_panes.len() != self.panes.len() {
            return Err("orphan panes".into());
        }
        Ok(())
    }

    /// Every tab is in exactly one pane, and every pane has tabs, activates one of its own and
    /// keeps pinned ones first.
    fn check_panes(&self) -> Result<(), String> {
        let mut seen_tabs = BTreeSet::new();
        for pane in self.panes.values() {
            if pane.tabs.is_empty() {
                return Err(format!("{:?} has no tabs", pane.id));
            }
            if !pane.tabs.contains(&pane.active) {
                return Err(format!("{:?} activates a tab it doesn't hold", pane.id));
            }
            for tab in &pane.tabs {
                if !seen_tabs.insert(*tab) || !self.tabs.contains_key(tab) {
                    return Err(format!("{tab:?} is duplicated or missing"));
                }
            }
            if !pane.tabs.is_sorted_by_key(|t| !self.tabs[t].pinned) {
                return Err(format!(
                    "{:?} has a pinned tab after an unpinned one",
                    pane.id
                ));
            }
        }
        // As for panes: every listed tab exists, so a size mismatch means unreachable tabs.
        if seen_tabs.len() != self.tabs.len() {
            return Err("orphan tabs".into());
        }
        Ok(())
    }
}

/// Every split ratio in the tree is strictly between 0 and 1.
fn check_ratios(node: &SplitNode) -> Result<(), String> {
    if let SplitNode::Split {
        ratio,
        first,
        second,
        ..
    } = node
    {
        // Written as a negated range test so NaN fails it too.
        if !(*ratio > 0.0 && *ratio < 1.0) {
            return Err(format!("ratio {ratio} out of range"));
        }
        check_ratios(first)?;
        check_ratios(second)?;
    }
    Ok(())
}
