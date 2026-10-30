//! Rebuilding the model from the session saved at the last exit.
//!
//! `App::new` calls [`startup`] with what [`session::load`] read. The layout goes through
//! [`Model::restore`]; everything the layout spec doesn't carry (names, flags, icons, colors,
//! groups) is then set with ordinary commands, so the model's own invariants still hold. The
//! per-tab details (working directory, agent session) are handed back for `App::spawn` to use
//! when each terminal starts.

use std::collections::BTreeMap;

use macaroni_core::{Command, Effect, GroupId, Model, TabId, WorkspaceId};

use crate::session::{self, Saved, SavedGroup, SavedTab, SavedWorkspace};

/// The model to start with, the effects that start it, and where to start each restored tab.
/// The map is empty for a fresh model.
pub(crate) type Startup = (Model, Vec<Effect>, BTreeMap<TabId, SavedTab>);

/// The saved session as a model, or a fresh model when there's nothing to restore.
pub(crate) fn startup(saved: Option<Saved>) -> Startup {
    let Some(saved) = saved else {
        let (model, effects) = Model::new();
        return (model, effects, BTreeMap::new());
    };
    let workspaces = saved.workspaces.clone();
    let saved_groups = saved.groups.clone();
    let (specs, active, details) = session::into_spec(saved);
    let (mut model, effects, tabs) = Model::restore(specs, active);
    // `Model::restore` keeps the saved order of workspaces and tabs, so zipping by position
    // pairs each new id with its saved details.
    let ids: Vec<_> = model.workspaces().iter().map(|w| w.id).collect();
    restore_workspaces(&mut model, &ids, &workspaces, &saved_groups);
    restore_tabs(&mut model, &tabs, &details);
    (model, effects, tabs.into_iter().zip(details).collect())
}

/// Gives each restored workspace its saved flags, icon, color and group. None of these
/// commands has effects, so what `apply` returns is ignored.
fn restore_workspaces(
    model: &mut Model,
    ids: &[WorkspaceId],
    workspaces: &[SavedWorkspace],
    saved_groups: &[SavedGroup],
) {
    let mut groups: BTreeMap<usize, GroupId> = BTreeMap::new();
    for (&id, saved) in ids.iter().zip(workspaces) {
        model.apply(Command::SetWorkspaceMuted(id, saved.muted));
        model.apply(Command::SetWorkspacePinned(id, saved.pinned));
        model.apply(Command::SetWorkspaceIcon(id, saved.icon.clone()));
        model.apply(Command::SetWorkspaceColor(id, saved.color.clone()));
        model.apply(Command::SetWorkspaceCwd(id, saved.cwd.clone()));
        if let Some(index) = saved.group {
            join_saved_group(model, id, index, saved_groups, &mut groups);
        }
    }
}

/// Puts a workspace in the group saved at `index`, creating that group on first use. Saved
/// groups are already together, so joining in order keeps the order.
fn join_saved_group(
    model: &mut Model,
    id: WorkspaceId,
    index: usize,
    saved_groups: &[SavedGroup],
    groups: &mut BTreeMap<usize, GroupId>,
) {
    if let Some(&group) = groups.get(&index) {
        model.apply(Command::SetWorkspaceGroup(id, Some(group)));
    } else if let Some(spec) = saved_groups.get(index) {
        model.apply(Command::NewGroup(id));
        if let Some(group) = model.workspace(id).and_then(|w| w.group) {
            model.apply(Command::RenameGroup(group, spec.name.clone()));
            model.apply(Command::SetGroupCollapsed(group, spec.collapsed));
            model.apply(Command::SetGroupColor(group, spec.color.clone()));
            model.apply(Command::SetGroupIcon(group, spec.icon.clone()));
            groups.insert(index, group);
        }
    }
}

/// Gives each restored tab its saved name, flags, icon and color.
fn restore_tabs(model: &mut Model, tabs: &[TabId], details: &[SavedTab]) {
    for (&tab, saved) in tabs.iter().zip(details) {
        model.apply(Command::RenameTab(tab, saved.name.clone()));
        model.apply(Command::SetTabMuted(tab, saved.muted));
        model.apply(Command::SetTabPinned(tab, saved.pinned));
        model.apply(Command::SetTabIcon(tab, saved.icon.clone()));
        model.apply(Command::SetTabColor(tab, saved.color.clone()));
    }
}
