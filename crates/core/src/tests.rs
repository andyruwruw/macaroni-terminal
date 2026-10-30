//! Behaviour tests for [`Model::apply`], one or more per command, plus a randomized consistency
//! test.
//!
//! The unit tests drive a model through commands and assert the resulting layout, focus, tab
//! order and emitted effects: splitting and closing, tab insertion and closing order, workspace
//! navigation and closing (including quitting on the last one), attention and muting, agents,
//! browser tabs, pinning, groups and sidebar placement, moving and dragging tabs, and the
//! `spec` / `restore` round trip with clamping of bad input.
//!
//! `random_commands_keep_the_model_consistent` runs 500 seeds of 120 steps each. At each step
//! `random_command` picks a command that refers only to ids that currently exist (so it exercises
//! real behaviour rather than the ignore-missing paths), applies it, and then asserts that
//! [`Model::check`] passes, that no session or browser id is ever spawned twice, that every kill
//! or close matches something live, and that the live sets equal what the model's tabs refer to.
//! An [`Effect::Quit`] must come with every session and browser ended, and ends the run. The PRNG
//! is a tiny built-in xorshift so the crate needs no dev-dependencies; a failure message names
//! the seed, step and command to reproduce it (there's no shrinking).

use std::collections::BTreeSet;

use crate::*;

/// The sessions the effects spawn, in order.
fn spawned(effects: &[Effect]) -> Vec<SessionId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SpawnSession(s) => Some(*s),
            _ => None,
        })
        .collect()
}

/// The sessions the effects kill, in order.
fn killed(effects: &[Effect]) -> Vec<SessionId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::KillSession(s) => Some(*s),
            _ => None,
        })
        .collect()
}

/// The browsers the effects open, in order.
fn opened(effects: &[Effect]) -> Vec<BrowserId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::OpenBrowser(b) => Some(*b),
            _ => None,
        })
        .collect()
}

/// The browsers the effects close, in order.
fn closed(effects: &[Effect]) -> Vec<BrowserId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::CloseBrowser(b) => Some(*b),
            _ => None,
        })
        .collect()
}

/// The session a terminal tab runs; panics if the tab is missing or a browser.
fn session_of(model: &Model, tab: TabId) -> SessionId {
    match model.tab(tab).unwrap().kind {
        TabKind::Terminal(s) => s,
        TabKind::Browser(_) => panic!("{tab:?} is a browser"),
    }
}

#[test]
fn new_model_has_one_terminal() {
    let (model, effects) = Model::new();
    assert_eq!(model.workspaces().len(), 1);
    assert_eq!(model.active_workspace().name, "Workspace 1");
    assert_eq!(spawned(&effects).len(), 1);
    assert_eq!(model.check(), Ok(()));
}

#[test]
fn workspace_folder_is_set_and_cleared() {
    let (mut model, _) = Model::new();
    let ws = model.active_workspace().id;
    assert_eq!(model.active_workspace().cwd, None);
    model.apply(Command::SetWorkspaceCwd(ws, Some("C:/work".into())));
    assert_eq!(model.active_workspace().cwd.as_deref(), Some("C:/work"));
    model.apply(Command::SetWorkspaceCwd(ws, Some("  ".into())));
    assert_eq!(model.active_workspace().cwd, None);
}

#[test]
fn merging_groups_keeps_the_order() {
    let (mut model, _) = Model::new();
    let a = model.active_workspace().id;
    model.apply(Command::NewWorkspace);
    let b = model.active_workspace().id;
    model.apply(Command::NewWorkspace);
    let c = model.active_workspace().id;
    model.apply(Command::NewGroup(a));
    model.apply(Command::NewGroup(b));
    let into = model
        .workspace(a)
        .and_then(|w| w.group)
        .expect("a is grouped");
    let from = model
        .workspace(b)
        .and_then(|w| w.group)
        .expect("b is grouped");
    model.apply(Command::SetWorkspaceGroup(c, Some(from)));
    model.apply(Command::MergeGroups { into, from });
    assert!(model.group(from).is_none(), "the emptied group is dropped");
    let order: Vec<_> = model.workspaces().iter().map(|w| (w.id, w.group)).collect();
    assert_eq!(
        order,
        vec![(a, Some(into)), (b, Some(into)), (c, Some(into))]
    );
    model.apply(Command::MergeGroups { into, from: into });
    assert_eq!(model.check(), Ok(()));
}

#[test]
fn combining_workspaces_moves_every_pane() {
    let (mut model, _) = Model::new();
    let into = model.active_workspace().id;
    model.apply(Command::NewWorkspace);
    model.apply(Command::SplitPane(Direction::Down));
    let from = model.active_workspace().id;
    let moved = model.active_workspace().layout.leaves();
    let effects = model.apply(Command::CombineWorkspaces { into, from });
    assert!(effects.is_empty(), "nothing is killed");
    assert_eq!(model.workspaces().len(), 1);
    assert_eq!(model.active_workspace().id, into);
    let leaves = model.active_workspace().layout.leaves();
    assert_eq!(leaves.len(), 3);
    assert!(moved.iter().all(|p| leaves.contains(p)));
    assert!(moved.contains(&model.active_workspace().focused));
    // Combining with itself or a missing workspace changes nothing.
    model.apply(Command::CombineWorkspaces { into, from: into });
    model.apply(Command::CombineWorkspaces { into, from });
    assert_eq!(model.active_workspace().layout.leaves().len(), 3);
    assert_eq!(model.check(), Ok(()));
}

#[test]
fn split_focuses_the_new_pane_and_spawns_a_shell() {
    let (mut model, _) = Model::new();
    let first = model.active_workspace().focused;
    let effects = model.apply(Command::SplitPane(Direction::Right));
    assert_eq!(spawned(&effects).len(), 1);
    let ws = model.active_workspace();
    assert_ne!(ws.focused, first);
    assert_eq!(ws.layout.leaves(), vec![first, ws.focused]);
}

#[test]
fn split_left_puts_the_new_pane_first() {
    let (mut model, _) = Model::new();
    let first = model.active_workspace().focused;
    model.apply(Command::SplitPane(Direction::Left));
    let ws = model.active_workspace();
    assert_eq!(ws.layout.leaves(), vec![ws.focused, first]);
}

#[test]
fn tabs_open_after_the_active_one_and_close_to_a_neighbour() {
    let (mut model, _) = Model::new();
    let pane = model.active_workspace().focused;
    let t1 = model.pane(pane).unwrap().active;
    model.apply(Command::NewTab);
    let t2 = model.pane(pane).unwrap().active;
    model.apply(Command::SelectTab(t1));
    model.apply(Command::NewTab);
    let t3 = model.pane(pane).unwrap().active;
    assert_eq!(model.pane(pane).unwrap().tabs, vec![t1, t3, t2]);

    let s3 = session_of(&model, t3);
    let effects = model.apply(Command::CloseActiveTab);
    assert_eq!(killed(&effects), vec![s3]);
    assert_eq!(model.pane(pane).unwrap().active, t2);
}

#[test]
fn closing_the_last_tab_closes_the_pane_and_refocuses_its_sibling() {
    let (mut model, _) = Model::new();
    let left = model.active_workspace().focused;
    model.apply(Command::SplitPane(Direction::Right));
    model.apply(Command::CloseActiveTab);
    let ws = model.active_workspace();
    assert_eq!(ws.layout, SplitNode::Leaf(left));
    assert_eq!(ws.focused, left);
}

#[test]
fn closing_the_last_workspace_quits() {
    let (mut model, start) = Model::new();
    let effects = model.apply(Command::CloseActiveTab);
    assert_eq!(effects.last(), Some(&Effect::Quit));
    assert_eq!(killed(&effects), spawned(&start));
}

#[test]
fn closing_a_workspace_kills_all_of_its_sessions() {
    let (mut model, _) = Model::new();
    let first = model.active_workspace().id;
    let mut started = spawned(&model.apply(Command::NewWorkspace));
    started.extend(spawned(&model.apply(Command::SplitPane(Direction::Down))));
    started.extend(spawned(&model.apply(Command::NewTab)));
    let second = model.active_workspace().id;

    let mut gone = killed(&model.apply(Command::CloseWorkspace(second)));
    gone.sort();
    started.sort();
    assert_eq!(gone, started);
    assert_eq!(model.active_workspace().id, first);
}

#[test]
fn close_active_workspace_moves_to_a_neighbour() {
    let (mut model, _) = Model::new();
    let first = model.active_workspace().id;
    model.apply(Command::NewWorkspace);
    model.apply(Command::CloseActiveWorkspace);
    assert_eq!(model.workspaces().len(), 1);
    assert_eq!(model.active_workspace().id, first);
}

#[test]
fn notices_mark_tabs_until_cleared() {
    let (mut model, start) = Model::new();
    let first = spawned(&start)[0];
    let pane = model.active_workspace().focused;
    let first_tab = model.focused_tab().id;
    let second = spawned(&model.apply(Command::NewTab))[0];
    let ws = model.active_workspace().id;
    assert!(!model.pane_attention(pane));

    model.apply(Command::Notify {
        session: first,
        title: Some("build".into()),
        body: "done".into(),
    });
    model.apply(Command::Notify {
        session: second,
        title: None,
        body: "waiting".into(),
    });
    assert!(model.pane_attention(pane));
    assert_eq!(model.workspace_notice(ws).unwrap().body, "waiting");

    let second_tab = model.tab_for_session(second).unwrap();
    model.apply(Command::ClearAttention(second_tab));
    assert_eq!(model.workspace_notice(ws).unwrap().body, "done");
    model.apply(Command::ClearAttention(first_tab));
    assert!(!model.pane_attention(pane));
    assert_eq!(model.workspace_notice(ws), None);
}

#[test]
fn visibility_follows_active_tabs_and_workspace() {
    let (mut model, _) = Model::new();
    let first = model.focused_tab().id;
    model.apply(Command::NewTab);
    let second = model.focused_tab().id;
    assert!(model.tab_visible(second) && !model.tab_visible(first));
    model.apply(Command::NewWorkspace);
    assert!(!model.tab_visible(second));
}

#[test]
fn agent_state_keeps_session_details_across_updates() {
    let (mut model, _) = Model::new();
    let tab = model.focused_tab().id;
    model.apply(Command::SetAgent {
        tab,
        state: AgentState::Idle,
        detail: None,
        session_id: Some("abc".into()),
        cwd: Some("C:/work".into()),
    });
    model.apply(Command::SetAgent {
        tab,
        state: AgentState::Working,
        detail: Some("Running cargo test".into()),
        session_id: None,
        cwd: None,
    });
    model.apply(Command::SetAgentSummary {
        tab,
        summary: Some("Opus".into()),
    });
    let agent = model.tab(tab).unwrap().agent.clone().unwrap();
    assert_eq!(agent.state, AgentState::Working);
    assert_eq!(agent.detail.as_deref(), Some("Running cargo test"));
    assert_eq!(agent.session_id.as_deref(), Some("abc"));
    assert_eq!(agent.cwd.as_deref(), Some("C:/work"));
    assert_eq!(agent.summary.as_deref(), Some("Opus"));
    model.apply(Command::ClearAgent(tab));
    assert_eq!(model.tab(tab).unwrap().agent, None);
}

#[test]
fn spec_and_restore_round_trip() {
    let (mut model, _) = Model::new();
    model.apply(Command::SplitPane(Direction::Right));
    model.apply(Command::NewTab);
    model.apply(Command::SplitPane(Direction::Down));
    let ws = model.active_workspace().id;
    model.apply(Command::SetSplitRatio {
        workspace: ws,
        path: vec![],
        ratio: 0.3,
    });
    model.apply(Command::RenameWorkspace(ws, "api".into()));
    model.apply(Command::NewWorkspace);
    model.apply(Command::SelectWorkspaceIndex(0));

    model.apply(Command::OpenBrowser {
        url: "https://example.com/".into(),
        split: None,
    });
    model.apply(Command::SelectWorkspaceIndex(0));

    let (spec, active, tabs) = model.spec();
    let (restored, effects, new_tabs) = Model::restore(spec.clone(), active);
    assert_eq!(restored.check(), Ok(()));
    assert_eq!(restored.spec().0, spec);
    assert_eq!(restored.spec().1, active);
    assert_eq!(new_tabs.len(), tabs.len());
    assert_eq!(
        spawned(&effects).len(),
        tabs.len() - 1,
        "a terminal per terminal tab"
    );
    let browser = opened(&effects)[0];
    let tab = restored
        .tab(restored.tab_for_browser(browser).unwrap())
        .unwrap();
    assert_eq!(tab.url.as_deref(), Some("https://example.com/"));
    assert_eq!(restored.active_workspace().name, "api");
}

#[test]
fn restore_clamps_bad_input() {
    let spec = vec![WorkspaceSpec {
        name: "  ".into(),
        layout: LayoutSpec::Split {
            axis: Axis::Row,
            ratio: f32::NAN,
            first: Box::new(LayoutSpec::Pane {
                tabs: vec![],
                active: 9,
            }),
            second: Box::new(LayoutSpec::Pane {
                tabs: vec![TabSpec::Terminal, TabSpec::Terminal],
                active: 5,
            }),
        },
        focused: 42,
    }];
    let (model, _, tabs) = Model::restore(spec, 7);
    assert_eq!(model.check(), Ok(()));
    assert_eq!(tabs.len(), 3);
    assert_eq!(model.active_workspace().name, "Workspace 1");
    let (empty, effects, tabs) = Model::restore(Vec::new(), 0);
    assert_eq!(empty.check(), Ok(()));
    assert_eq!((spawned(&effects).len(), tabs.len()), (1, 1));
}

#[test]
fn browser_tabs_open_split_and_close() {
    let (mut model, _) = Model::new();
    let first = model.active_workspace().focused;
    let effects = model.apply(Command::OpenBrowser {
        url: "https://example.com/".into(),
        split: Some(Direction::Right),
    });
    let browser = opened(&effects)[0];
    assert!(
        spawned(&effects).is_empty(),
        "no terminal in a browser split"
    );
    let ws = model.active_workspace();
    assert_ne!(ws.focused, first);
    let pane = model.pane(ws.focused).unwrap();
    assert_eq!(pane.tabs.len(), 1);
    let tab = model.tab_for_browser(browser).unwrap();
    assert_eq!(pane.active, tab);

    model.apply(Command::SetBrowserUrl(
        browser,
        "https://example.org/".into(),
    ));
    model.apply(Command::SetBrowserTitle(browser, Some("Example".into())));
    let t = model.tab(tab).unwrap();
    assert_eq!(t.url.as_deref(), Some("https://example.org/"));
    assert_eq!(t.title.as_deref(), Some("Example"));

    let effects = model.apply(Command::OpenBrowserBeside {
        beside: tab,
        url: "https://popup.example/".into(),
    });
    let popup = model.tab_for_browser(opened(&effects)[0]).unwrap();
    assert_eq!(
        model.pane(ws_focused(&model)).unwrap().tabs,
        vec![tab, popup]
    );

    let effects = model.apply(Command::BrowserClosed(browser));
    assert_eq!(closed(&effects), vec![browser]);
    assert!(model.tab(tab).is_none());
    let effects = model.apply(Command::CloseTab(popup));
    assert_eq!(closed(&effects).len(), 1);
    assert_eq!(model.active_workspace().layout.leaves(), vec![first]);
}

/// The active workspace's focused pane.
fn ws_focused(model: &Model) -> PaneId {
    model.active_workspace().focused
}

#[test]
fn session_exit_closes_its_tab() {
    let (mut model, start) = Model::new();
    model.apply(Command::NewTab);
    let first_session = spawned(&start)[0];
    let effects = model.apply(Command::SessionExited(first_session));
    assert_eq!(killed(&effects), vec![first_session]);
    assert_eq!(model.tab_for_session(first_session), None);
}

#[test]
fn workspace_navigation_wraps() {
    let (mut model, _) = Model::new();
    let a = model.active_workspace().id;
    model.apply(Command::NewWorkspace);
    let b = model.active_workspace().id;
    model.apply(Command::NextWorkspace);
    assert_eq!(model.active_workspace().id, a);
    model.apply(Command::PreviousWorkspace);
    assert_eq!(model.active_workspace().id, b);
    model.apply(Command::SelectWorkspaceIndex(0));
    assert_eq!(model.active_workspace().id, a);
}

#[test]
fn rename_and_move_workspaces() {
    let (mut model, _) = Model::new();
    let a = model.active_workspace().id;
    model.apply(Command::NewWorkspace);
    model.apply(Command::RenameWorkspace(a, "  api  ".into()));
    model.apply(Command::RenameWorkspace(a, "   ".into()));
    assert_eq!(model.workspace(a).unwrap().name, "api");
    model.apply(Command::MoveWorkspace(a, 9));
    assert_eq!(model.workspaces().last().unwrap().id, a);
}

#[test]
fn split_ratio_is_clamped() {
    let (mut model, _) = Model::new();
    model.apply(Command::SplitPane(Direction::Right));
    let workspace = model.active_workspace().id;
    model.apply(Command::SetSplitRatio {
        workspace,
        path: vec![],
        ratio: 2.0,
    });
    let SplitNode::Split { ratio, .. } = model.active_workspace().layout else {
        panic!("expected a split");
    };
    assert_eq!(ratio, 1.0 - MIN_RATIO);
}

#[test]
fn directional_focus_moves_between_panes() {
    let (mut model, _) = Model::new();
    let left = model.active_workspace().focused;
    model.apply(Command::SplitPane(Direction::Right));
    let right = model.active_workspace().focused;
    model.apply(Command::FocusDirection(Direction::Left));
    assert_eq!(model.active_workspace().focused, left);
    model.apply(Command::FocusDirection(Direction::Left));
    assert_eq!(model.active_workspace().focused, left);
    model.apply(Command::FocusDirection(Direction::Right));
    assert_eq!(model.active_workspace().focused, right);
}

#[test]
fn tab_titles_ignore_blank_values() {
    let (mut model, start) = Model::new();
    let session = spawned(&start)[0];
    model.apply(Command::SetTabTitle(session, Some("vim".into())));
    assert_eq!(model.focused_tab().title.as_deref(), Some("vim"));
    model.apply(Command::SetTabTitle(session, Some(" ".into())));
    assert_eq!(model.focused_tab().title, None);
}

#[test]
fn tabs_can_be_renamed_muted_and_pinned() {
    let (mut model, start) = Model::new();
    let session = spawned(&start)[0];
    let first = model.focused_tab().id;
    model.apply(Command::SetTabTitle(session, Some("vim".into())));
    model.apply(Command::RenameTab(first, Some("  editor ".into())));
    assert_eq!(model.focused_tab().label(), Some("editor"));
    model.apply(Command::RenameTab(first, Some(" ".into())));
    assert_eq!(model.focused_tab().label(), Some("vim"));

    model.apply(Command::SetTabIcon(first, Some("rocket".into())));
    model.apply(Command::SetTabColor(first, Some("red".into())));
    assert_eq!(model.focused_tab().icon.as_deref(), Some("rocket"));
    assert_eq!(model.focused_tab().color.as_deref(), Some("red"));

    model.apply(Command::SetTabMuted(first, true));
    model.apply(Command::Notify {
        session,
        title: None,
        body: "done".into(),
    });
    assert_eq!(model.focused_tab().attention, None);

    model.apply(Command::NewTab);
    model.apply(Command::NewTab);
    let pane = model.active_workspace().focused;
    let tabs = model.pane(pane).unwrap().tabs.clone();
    model.apply(Command::SetTabPinned(tabs[2], true));
    let order = model.pane(pane).unwrap().tabs.clone();
    assert_eq!(order, vec![tabs[2], tabs[0], tabs[1]]);
    assert_eq!(model.pane(pane).unwrap().active, tabs[2]);
    // New tabs never land among the pinned ones.
    model.apply(Command::SelectTab(tabs[2]));
    model.apply(Command::NewTab);
    let order = model.pane(pane).unwrap().tabs.clone();
    assert_eq!(order[0], tabs[2]);
    assert!(!model.tab(order[1]).unwrap().pinned);
    model.apply(Command::SetTabPinned(tabs[2], false));
    assert_eq!(model.pane(pane).unwrap().tabs[0], tabs[2]);
    // Unpinning a tab that isn't pinned leaves the order alone (restoring a session does it).
    let order = model.pane(pane).unwrap().tabs.clone();
    model.apply(Command::SetTabPinned(order[2], false));
    assert_eq!(model.pane(pane).unwrap().tabs, order);
}

#[test]
fn workspaces_can_be_muted_and_pinned() {
    let (mut model, start) = Model::new();
    let session = spawned(&start)[0];
    let first = model.active_workspace().id;
    model.apply(Command::NewWorkspace);
    model.apply(Command::NewWorkspace);
    let ids = |m: &Model| m.workspaces().iter().map(|w| w.id).collect::<Vec<_>>();
    let order = ids(&model);

    model.apply(Command::SetWorkspacePinned(order[2], true));
    assert_eq!(ids(&model), vec![order[2], order[0], order[1]]);
    // Unpinned workspaces can't be moved above pinned ones, nor pinned ones below.
    model.apply(Command::MoveWorkspace(order[1], 0));
    assert_eq!(ids(&model), vec![order[2], order[1], order[0]]);
    model.apply(Command::MoveWorkspace(order[2], 9));
    assert_eq!(ids(&model)[0], order[2]);
    model.apply(Command::SetWorkspacePinned(order[0], false));
    assert_eq!(ids(&model), vec![order[2], order[1], order[0]]);

    model.apply(Command::SetWorkspaceMuted(first, true));
    model.apply(Command::Notify {
        session,
        title: None,
        body: "done".into(),
    });
    assert_eq!(model.workspace_notice(first), None);
}

#[test]
fn groups_stay_together() {
    let (mut model, _) = Model::new();
    for _ in 0..4 {
        model.apply(Command::NewWorkspace);
    }
    let ids = |m: &Model| m.workspaces().iter().map(|w| w.id).collect::<Vec<_>>();
    let w = ids(&model);

    model.apply(Command::NewGroup(w[1]));
    let group = model.workspace(w[1]).unwrap().group.unwrap();
    model.apply(Command::SetWorkspaceGroup(w[3], Some(group)));
    assert_eq!(ids(&model), vec![w[0], w[1], w[3], w[2], w[4]]);
    model.apply(Command::RenameGroup(group, " api ".into()));
    assert_eq!(model.group(group).unwrap().name, "api");
    model.apply(Command::SetGroupColor(group, Some("blue".into())));
    model.apply(Command::SetGroupIcon(group, Some("rocket".into())));
    assert_eq!(
        model.group(group).and_then(|g| g.icon.as_deref()),
        Some("rocket")
    );
    assert_eq!(model.group(group).unwrap().color.as_deref(), Some("blue"));

    // Shifting an ungrouped workspace jumps over the whole group.
    model.apply(Command::ShiftWorkspace(w[0], Shift::Down));
    assert_eq!(ids(&model), vec![w[1], w[3], w[0], w[2], w[4]]);
    // Inside a group, workspaces move within it.
    model.apply(Command::ShiftWorkspace(w[3], Shift::Top));
    assert_eq!(ids(&model), vec![w[3], w[1], w[0], w[2], w[4]]);
    model.apply(Command::ShiftGroup(group, Shift::Bottom));
    assert_eq!(ids(&model), vec![w[0], w[2], w[4], w[3], w[1]]);

    // Pinning a member pins the group, at the top.
    model.apply(Command::SetWorkspacePinned(w[1], true));
    assert_eq!(ids(&model), vec![w[3], w[1], w[0], w[2], w[4]]);
    assert!(model.workspace(w[3]).unwrap().pinned);
    // Joining a pinned group pins the workspace too.
    model.apply(Command::SetWorkspaceGroup(w[4], Some(group)));
    assert!(model.workspace(w[4]).unwrap().pinned);
    assert_eq!(ids(&model), vec![w[3], w[1], w[4], w[0], w[2]]);

    model.apply(Command::SetWorkspaceGroup(w[3], None));
    assert_eq!(ids(&model), vec![w[1], w[4], w[3], w[0], w[2]]);
    model.apply(Command::Ungroup(group));
    assert!(model.groups().is_empty());
    // A group with no workspaces left disappears.
    model.apply(Command::NewGroup(w[2]));
    model.apply(Command::CloseWorkspace(w[2]));
    assert!(model.groups().is_empty());
}

#[test]
fn placing_workspaces_in_and_out_of_groups() {
    let (mut model, _) = Model::new();
    for _ in 0..4 {
        model.apply(Command::NewWorkspace);
    }
    let ids = |m: &Model| m.workspaces().iter().map(|w| w.id).collect::<Vec<_>>();
    let w = ids(&model);
    model.apply(Command::NewGroup(w[1]));
    let group = model.workspace(w[1]).unwrap().group.unwrap();
    model.apply(Command::SetWorkspaceGroup(w[2], Some(group)));
    assert_eq!(ids(&model), vec![w[0], w[1], w[2], w[3], w[4]]);
    let place = |id, before, group| Command::PlaceWorkspace { id, before, group };

    // Into the group, before a member, and at its end.
    model.apply(place(w[0], Some(w[2]), Some(group)));
    assert_eq!(ids(&model), vec![w[1], w[0], w[2], w[3], w[4]]);
    model.apply(place(w[4], None, Some(group)));
    assert_eq!(ids(&model), vec![w[1], w[0], w[2], w[4], w[3]]);
    assert_eq!(model.workspace(w[4]).unwrap().group, Some(group));

    // Out of it: before the group, after it, and before a later member means after it.
    model.apply(place(w[0], Some(w[1]), None));
    assert_eq!(ids(&model), vec![w[0], w[1], w[2], w[4], w[3]]);
    assert_eq!(model.workspace(w[0]).unwrap().group, None);
    model.apply(place(w[4], Some(w[3]), None));
    assert_eq!(ids(&model), vec![w[0], w[1], w[2], w[4], w[3]]);
    assert_eq!(model.workspace(w[4]).unwrap().group, None);
    model.apply(place(w[0], Some(w[2]), None));
    assert_eq!(ids(&model), vec![w[1], w[2], w[0], w[4], w[3]]);
    model.apply(place(w[3], None, None));
    assert_eq!(ids(&model), vec![w[1], w[2], w[0], w[4], w[3]]);

    // A group's last member stays put rather than vanishing with its group.
    model.apply(place(w[2], None, None));
    model.apply(place(w[1], Some(w[0]), Some(group)));
    assert_eq!(model.workspace(w[1]).unwrap().group, Some(group));
    assert!(model.group(group).is_some());
}

#[test]
fn moving_tabs_keeps_them_running() {
    let (mut model, _) = Model::new();
    let first_ws = model.active_workspace().id;
    model.apply(Command::NewTab);
    let moved = model.focused_tab().id;

    let effects = model.apply(Command::MoveTab {
        tab: moved,
        to: MoveTo::NewWorkspace,
    });
    assert!(effects.is_empty());
    assert_eq!(model.workspaces().len(), 2);
    assert_eq!(model.focused_tab().id, moved);
    let second_ws = model.active_workspace().id;

    // Back again: the second workspace is left empty, so it closes.
    model.apply(Command::MoveTab {
        tab: moved,
        to: MoveTo::Workspace(first_ws),
    });
    assert_eq!(model.workspaces().len(), 1);
    assert!(model.workspace(second_ws).is_none());
    assert_eq!(model.focused_tab().id, moved);

    let home = model.active_workspace().focused;
    model.apply(Command::MoveTab {
        tab: moved,
        to: MoveTo::SplitPane {
            pane: home,
            direction: Direction::Right,
        },
    });
    assert_eq!(model.active_workspace().layout.leaves().len(), 2);
    assert_eq!(model.focused_tab().id, moved);

    // A pane's only tab can't be split off from itself.
    let before = model.clone().spec();
    let alone_in = model.active_workspace().focused;
    model.apply(Command::MoveTab {
        tab: moved,
        to: MoveTo::SplitPane {
            pane: alone_in,
            direction: Direction::Down,
        },
    });
    assert_eq!(model.spec(), before);
}

#[test]
fn dragging_tabs_reorders_and_snaps() {
    let (mut model, _) = Model::new();
    model.apply(Command::NewTab);
    model.apply(Command::NewTab);
    let pane = model.active_workspace().focused;
    let tabs = model.pane(pane).unwrap().tabs.clone();

    // Dragging the first tab to after the last: index counts the dragged tab.
    model.apply(Command::MoveTab {
        tab: tabs[0],
        to: MoveTo::Pane { pane, index: 3 },
    });
    assert_eq!(
        model.pane(pane).unwrap().tabs,
        vec![tabs[1], tabs[2], tabs[0]]
    );
    model.apply(Command::MoveTab {
        tab: tabs[0],
        to: MoveTo::Pane { pane, index: 0 },
    });
    assert_eq!(model.pane(pane).unwrap().tabs, tabs);

    // Snapped to the right half of its own pane, then into the left pane's strip.
    model.apply(Command::MoveTab {
        tab: tabs[2],
        to: MoveTo::SplitPane {
            pane,
            direction: Direction::Right,
        },
    });
    let right = model.active_workspace().focused;
    assert_ne!(right, pane);
    model.apply(Command::MoveTab {
        tab: tabs[2],
        to: MoveTo::Pane { pane, index: 1 },
    });
    // The right pane emptied and closed.
    assert_eq!(model.active_workspace().layout, SplitNode::Leaf(pane));
    assert_eq!(
        model.pane(pane).unwrap().tabs,
        vec![tabs[0], tabs[2], tabs[1]]
    );

    // Along the whole workspace's edge: as tall as the workspace, beside every pane.
    model.apply(Command::SplitPane(Direction::Down));
    let ws = model.active_workspace().id;
    model.apply(Command::MoveTab {
        tab: tabs[1],
        to: MoveTo::SplitWorkspace {
            workspace: ws,
            direction: Direction::Left,
        },
    });
    let layout = &model.active_workspace().layout;
    match layout {
        SplitNode::Split {
            axis: Axis::Row,
            first,
            second,
            ..
        } => {
            assert!(matches!(**first, SplitNode::Leaf(_)));
            assert_eq!(second.leaves().len(), 2);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(model.focused_tab().id, tabs[1]);
}

/// A tiny deterministic PRNG (xorshift64*), so the randomized test needs no dependencies.
struct Rng(u64);

impl Rng {
    /// The next pseudo-random value. The seed must be non-zero, or every value is 0.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A value in `0..n` (slightly biased, which doesn't matter here); panics if `n` is 0.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// A random element of `items`; panics if it's empty.
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }
}

/// Every direction, for picking one at random.
const DIRECTIONS: [Direction; 4] = [
    Direction::Left,
    Direction::Right,
    Direction::Up,
    Direction::Down,
];

/// A random command that refers only to things that currently exist.
///
/// Arms are weighted by how many of the 25 buckets map to them; the unlisted buckets (and 16
/// when there are no sessions) end a random tab's session or browser, so tabs churn steadily.
/// Ratios range over -1..2 to exercise clamping, and some picks (moves, placements) are
/// pointless, which covers the ignore paths.
fn random_command(rng: &mut Rng, model: &Model) -> Command {
    let workspaces: Vec<_> = model.workspaces().iter().map(|w| w.id).collect();
    let panes: Vec<_> = model
        .workspaces()
        .iter()
        .flat_map(|w| w.layout.leaves())
        .collect();
    let tabs: Vec<_> = panes
        .iter()
        .flat_map(|p| model.pane(*p).unwrap().tabs.clone())
        .collect();
    let sessions: Vec<_> = model.sessions().collect();
    match rng.below(25) {
        15 => Command::CloseActiveWorkspace,
        16 if !sessions.is_empty() => Command::Notify {
            session: rng.pick(&sessions),
            title: None,
            body: "x".into(),
        },
        17 => Command::ClearAttention(rng.pick(&tabs)),
        0 => Command::NewWorkspace,
        1 => Command::CloseWorkspace(rng.pick(&workspaces)),
        2 => Command::SelectWorkspace(rng.pick(&workspaces)),
        3 => Command::NextWorkspace,
        4 => Command::MoveWorkspace(rng.pick(&workspaces), rng.below(6)),
        5 => Command::SplitPane(rng.pick(&DIRECTIONS)),
        6 => Command::ClosePane,
        7 => Command::FocusPane(rng.pick(&panes)),
        8 => Command::FocusDirection(rng.pick(&DIRECTIONS)),
        9 => {
            let ws = model.active_workspace();
            let dividers = ws.layout.dividers(Rect::UNIT);
            let path = if dividers.is_empty() {
                vec![]
            } else {
                dividers[rng.below(dividers.len())].path.clone()
            };
            Command::SetSplitRatio {
                workspace: ws.id,
                path,
                ratio: rng.below(300) as f32 / 100.0 - 1.0,
            }
        }
        10 => Command::NewTab,
        11 => Command::CloseTab(rng.pick(&tabs)),
        12 => Command::SelectTab(rng.pick(&tabs)),
        13 => Command::NextTab,
        18 => Command::OpenBrowser {
            url: "https://example.com/".into(),
            split: [None, Some(rng.pick(&DIRECTIONS))][rng.below(2)],
        },
        19 => Command::OpenBrowserBeside {
            beside: rng.pick(&tabs),
            url: "https://example.com/".into(),
        },
        21 => Command::SetTabPinned(rng.pick(&tabs), rng.below(2) == 0),
        22 => Command::SetTabMuted(rng.pick(&tabs), rng.below(2) == 0),
        23 => {
            let groups: Vec<_> = model.groups().iter().map(|g| g.id).collect();
            let group = (!groups.is_empty()).then(|| rng.pick(&groups));
            let shift = rng.pick(&[Shift::Up, Shift::Down, Shift::Top, Shift::Bottom]);
            match (rng.below(9), group) {
                (0, _) => Command::RenameTab(rng.pick(&tabs), Some("x".into())),
                (1, _) => Command::SetWorkspacePinned(rng.pick(&workspaces), rng.below(2) == 0),
                (2, _) => Command::SetWorkspaceMuted(rng.pick(&workspaces), rng.below(2) == 0),
                (3, _) => Command::NewGroup(rng.pick(&workspaces)),
                (4, _) if rng.below(2) == 0 => {
                    Command::SetWorkspaceGroup(rng.pick(&workspaces), group)
                }
                (4, _) => Command::PlaceWorkspace {
                    id: rng.pick(&workspaces),
                    before: [None, Some(rng.pick(&workspaces))][rng.below(2)],
                    group,
                },
                (5, _) => Command::ShiftWorkspace(rng.pick(&workspaces), shift),
                (6, Some(g)) => Command::ShiftGroup(g, shift),
                (7, Some(g)) => Command::SetGroupPinned(g, rng.below(2) == 0),
                (_, Some(g)) => Command::Ungroup(g),
                (_, None) => Command::NewGroup(rng.pick(&workspaces)),
            }
        }
        24 => Command::MoveTab {
            tab: rng.pick(&tabs),
            to: match rng.below(5) {
                0 => MoveTo::Workspace(rng.pick(&workspaces)),
                1 => MoveTo::NewWorkspace,
                2 => MoveTo::Pane {
                    pane: rng.pick(&panes),
                    index: rng.below(5),
                },
                3 => MoveTo::SplitPane {
                    pane: rng.pick(&panes),
                    direction: rng.pick(&DIRECTIONS),
                },
                _ => MoveTo::SplitWorkspace {
                    workspace: rng.pick(&workspaces),
                    direction: rng.pick(&DIRECTIONS),
                },
            },
        },
        _ => {
            let tab = rng.pick(&tabs);
            match model.tab(tab).unwrap().kind {
                TabKind::Terminal(session) => Command::SessionExited(session),
                TabKind::Browser(browser) => Command::BrowserClosed(browser),
            }
        }
    }
}

#[test]
fn random_commands_keep_the_model_consistent() {
    for seed in 1..=500u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let (mut model, effects) = Model::new();
        let mut live: BTreeSet<SessionId> = spawned(&effects).into_iter().collect();
        let mut ever = live.clone();
        let mut browsers = BTreeSet::new();

        for step in 0..120 {
            let command = random_command(&mut rng, &model);
            let effects = model.apply(command.clone());
            let context = format!("seed {seed}, step {step}, after {command:?}");
            assert_eq!(model.check(), Ok(()), "{context}");

            for s in spawned(&effects) {
                assert!(ever.insert(s), "{context}: session id {s:?} reused");
                live.insert(s);
            }
            for s in killed(&effects) {
                assert!(live.remove(&s), "{context}: killed {s:?} which wasn't live");
            }
            for b in opened(&effects) {
                assert!(browsers.insert(b), "{context}: browser id {b:?} reused");
            }
            for b in closed(&effects) {
                assert!(
                    browsers.remove(&b),
                    "{context}: closed {b:?} which wasn't open"
                );
            }
            if effects.contains(&Effect::Quit) {
                assert!(live.is_empty(), "{context}: quit with live sessions");
                assert!(browsers.is_empty(), "{context}: quit with open browsers");
                break;
            }
            let in_model: BTreeSet<_> = model.sessions().collect();
            assert_eq!(in_model, live, "{context}");
            let in_model: BTreeSet<_> = model.browsers().collect();
            assert_eq!(in_model, browsers, "{context}");
        }
    }
}
