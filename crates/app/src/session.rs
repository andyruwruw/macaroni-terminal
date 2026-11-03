//! The saved session (`session.json` next to the config): workspaces, splits, tabs, and for each
//! tab its working directory and the coding-agent session running in it (or, for a browser tab,
//! its address), so the next launch can rebuild the layout and resume the agents.
//!
//! [`capture`] turns the model into a [`Saved`] (written by [`save`] from
//! `update/effects.rs`), and [`into_spec`] turns a loaded one back into what [`Model::restore`]
//! takes; `restore.rs` applies the rest. The format is versioned ([`VERSION`]): a file with
//! another version, or no workspaces, is ignored and the app starts fresh.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use macaroni_core::{
    Axis, LayoutSpec, Model, SessionId, Spec, TabId, TabKind, TabSpec, WorkspaceSpec,
};
use serde::{Deserialize, Serialize};

/// The session file's name, in the config directory.
const FILE_NAME: &str = "session.json";
/// The format version written and accepted. Bump it when an old file can't be read as the new
/// format.
const VERSION: u32 = 1;

/// The whole saved session, as written to `session.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    /// [`VERSION`] when written; other versions are ignored on load.
    pub version: u32,
    /// The index in `workspaces` of the focused window's active workspace. Sessions from before
    /// windows have only this.
    pub active: usize,
    /// Every workspace: each window's in its sidebar order, windows in order.
    pub workspaces: Vec<SavedWorkspace>,
    /// Every window, in order; empty in sessions from before windows, which had one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<SavedWindow>,
    /// The index in `windows` of the focused one.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub focused_window: usize,
    /// Every group, in sidebar order; workspaces refer to them by index.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<SavedGroup>,
    /// The sidebar's dragged width and whether it's folded away.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar: Option<SidebarLayout>,
}

/// Whether `n` is 0, so a default is left out of the file.
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A window: its active workspace and where it was on screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
    /// The index in [`Saved::workspaces`] of its active workspace.
    pub active: usize,
    /// Its top-left corner on screen, in logical pixels, if it was known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<(f32, f32)>,
    /// Its size in logical pixels, if it was known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<(f32, f32)>,
    /// Whether it was maximized.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub maximized: bool,
}

/// The sidebar as the user left it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SidebarLayout {
    /// Its dragged width; `None` uses `style.window.sidebar_width`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// Folded away (Ctrl+B, or dragged shut).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

/// A sidebar group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedGroup {
    /// The group's name.
    pub name: String,
    /// Whether its workspaces are hidden in the sidebar.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
    /// A preset color's name or `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Its icon's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// A workspace: its name, split tree and sidebar look.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedWorkspace {
    /// The workspace's name.
    pub name: String,
    /// The index of the focused pane, in the layout's leaf order.
    pub focused: usize,
    /// The split tree, with each pane's tabs.
    pub layout: SavedLayout,
    /// Whether its notifications are muted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// Whether it's pinned.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// Its icon's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A preset color's name or `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Index into [`Saved::groups`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
    /// Its folder, where new terminals start; unset is the home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Environment variables for its terminals, as name and value pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
    /// The command typed into its new shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<String>,
    /// The index in [`Saved::windows`] of its window.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub window: usize,
}

/// A workspace's split tree, mirroring [`LayoutSpec`] with each tab's details in place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SavedLayout {
    /// A pane and its tabs.
    Pane {
        /// The index of the pane's active tab.
        active: usize,
        /// The pane's tabs, in strip order.
        tabs: Vec<SavedTab>,
    },
    /// Two parts side by side (`row`) or stacked (`column`).
    Split {
        /// Which way the split runs.
        axis: SavedAxis,
        /// The first part's share of the area.
        ratio: f32,
        /// The left or top part.
        first: Box<SavedLayout>,
        /// The right or bottom part.
        second: Box<SavedLayout>,
    },
}

/// [`Axis`] as saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedAxis {
    /// Side by side.
    Row,
    /// One above the other.
    Column,
}

/// One tab: where its terminal ran and what ran in it, or a browser tab's address, plus its
/// look. The default is a plain terminal tab.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    /// The terminal's working directory (from OSC 7, else the agent's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The Claude Code session to resume in this tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<String>,
    /// Set for SSH tabs: the `[[ssh]]` profile to connect to again, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<String>,
    /// Set for browser tabs: the page to reopen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The user's name for the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Whether its notifications are muted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// Whether it's pinned.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// Its icon's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A preset color's name or `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The file in [`scrollback_dir`] holding the terminal's text from before the restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollback: Option<String>,
}

/// Where terminals' text is kept across a restart (`[session] keep_scrollback`): a
/// `scrollback` folder in the config directory, one file per tab, named in its [`SavedTab`].
pub fn scrollback_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("scrollback")
}

/// Where the session file lives: `session.json` in the config directory.
pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join(FILE_NAME)
}

/// The model's windows and layouts plus each tab's directory and agent session. Windows have
/// no position or size here; the app adds those. `restored` holds what was
/// restored for tabs whose terminal hasn't started yet (hidden ones start just after the first shown):
/// they keep that directory and agent session, which the model doesn't have for them, so a tab
/// left unopened survives any number of restarts. `ssh` names the profile each SSH terminal runs.
pub fn capture(
    model: &Model,
    restored: &BTreeMap<TabId, SavedTab>,
    ssh: &BTreeMap<SessionId, String>,
    scrollback: &BTreeMap<TabId, String>,
) -> Saved {
    // `tabs` lists every tab in the order the specs' panes list them, so each pane takes its
    // share off the front.
    let (spec, tabs) = model.spec();
    let mut tabs = tabs.into_iter();
    // Groups are per window; saved, they're one list that workspaces index into.
    let groups: Vec<&macaroni_core::Group> = model
        .windows()
        .into_iter()
        .filter_map(|w| model.window(w))
        .flat_map(|w| w.groups.iter())
        .collect();
    let workspaces = spec
        .workspaces
        .into_iter()
        .zip(model.all_workspaces())
        .map(|(spec, ws)| SavedWorkspace {
            name: spec.name,
            focused: spec.focused,
            layout: layout_from_spec(&spec.layout, model, restored, ssh, scrollback, &mut tabs),
            muted: ws.muted,
            pinned: ws.pinned,
            icon: ws.icon.clone(),
            color: ws.color.clone(),
            cwd: ws.cwd.clone(),
            env: ws.env.clone(),
            startup: ws.startup.clone(),
            group: ws
                .group
                .and_then(|g| groups.iter().position(|group| group.id == g)),
            window: spec.window,
        })
        .collect();
    Saved {
        version: VERSION,
        active: spec.windows.get(spec.focused).copied().unwrap_or(0),
        workspaces,
        windows: spec
            .windows
            .iter()
            .map(|&active| SavedWindow {
                active,
                ..SavedWindow::default()
            })
            .collect(),
        focused_window: spec.focused,
        groups: groups
            .iter()
            .map(|g| SavedGroup {
                name: g.name.clone(),
                collapsed: g.collapsed,
                color: g.color.clone(),
                icon: g.icon.clone(),
            })
            .collect(),
        sidebar: None,
    }
}

/// What's saved for one tab: its name and look, and where its terminal ran and what ran in
/// it (or a browser tab's address). Also what a closed tab is reopened from.
pub fn saved_tab(
    model: &Model,
    restored: &BTreeMap<TabId, SavedTab>,
    ssh: &BTreeMap<SessionId, String>,
    id: TabId,
) -> SavedTab {
    let tab = model.tab(id);
    let common = SavedTab {
        name: tab.and_then(|t| t.name.clone()),
        muted: tab.is_some_and(|t| t.muted),
        pinned: tab.is_some_and(|t| t.pinned),
        icon: tab.and_then(|t| t.icon.clone()),
        color: tab.and_then(|t| t.color.clone()),
        ..SavedTab::default()
    };
    if let Some(tab) = tab
        && let TabKind::Browser(_) = tab.kind
    {
        return SavedTab {
            url: Some(tab.url.clone().unwrap_or_default()),
            ..common
        };
    }
    if let Some(pending) = restored.get(&id) {
        return SavedTab {
            cwd: pending.cwd.clone(),
            agent_session: pending.agent_session.clone(),
            ssh: pending.ssh.clone(),
            ..common
        };
    }
    // An agent's own directory wins over the shell's: `claude --resume` only finds a session
    // from the directory it ran in, and the shell reports one only at a prompt, so
    // `cd x && claude` leaves the old one.
    let agent = tab.and_then(|t| t.agent.as_ref());
    SavedTab {
        cwd: agent
            .and_then(|a| a.cwd.clone())
            .or_else(|| tab.and_then(|t| t.cwd.clone())),
        agent_session: agent.and_then(|a| a.session_id.clone()),
        ssh: match tab.map(|t| &t.kind) {
            Some(TabKind::Terminal(session)) => ssh.get(session).cloned(),
            _ => None,
        },
        ..common
    }
}

/// One workspace's [`LayoutSpec`] as a [`SavedLayout`], taking each pane's tabs in order from
/// `tabs` and filling in their details from the model.
fn layout_from_spec(
    spec: &LayoutSpec,
    model: &Model,
    restored: &BTreeMap<TabId, SavedTab>,
    ssh: &BTreeMap<SessionId, String>,
    scrollback: &BTreeMap<TabId, String>,
    tabs: &mut impl Iterator<Item = TabId>,
) -> SavedLayout {
    match spec {
        LayoutSpec::Pane {
            tabs: specs,
            active,
        } => SavedLayout::Pane {
            active: *active,
            tabs: tabs
                .take(specs.len())
                .map(|id| SavedTab {
                    scrollback: scrollback
                        .get(&id)
                        .cloned()
                        .or_else(|| restored.get(&id).and_then(|r| r.scrollback.clone())),
                    ..saved_tab(model, restored, ssh, id)
                })
                .collect(),
        },
        LayoutSpec::Split {
            axis,
            ratio,
            first,
            second,
        } => SavedLayout::Split {
            axis: match axis {
                Axis::Row => SavedAxis::Row,
                Axis::Column => SavedAxis::Column,
            },
            ratio: *ratio,
            first: Box::new(layout_from_spec(
                first, model, restored, ssh, scrollback, tabs,
            )),
            second: Box::new(layout_from_spec(
                second, model, restored, ssh, scrollback, tabs,
            )),
        },
    }
}

/// What [`Model::restore`] needs, plus the per-tab details in the same order as its tabs. A
/// session from before windows is one window.
pub fn into_spec(saved: Saved) -> (Spec, Vec<SavedTab>) {
    let mut tabs = Vec::new();
    let windows = if saved.windows.is_empty() {
        vec![saved.active]
    } else {
        saved.windows.iter().map(|w| w.active).collect()
    };
    let workspaces = saved
        .workspaces
        .into_iter()
        .map(|ws| WorkspaceSpec {
            name: ws.name,
            focused: ws.focused,
            layout: spec_from_layout(ws.layout, &mut tabs),
            window: ws.window,
        })
        .collect();
    let spec = Spec {
        workspaces,
        windows,
        focused: saved.focused_window,
    };
    (spec, tabs)
}

/// A [`SavedLayout`] as a [`LayoutSpec`], pushing each tab's details onto `out` in the order
/// [`Model::restore`] will create the tabs.
fn spec_from_layout(layout: SavedLayout, out: &mut Vec<SavedTab>) -> LayoutSpec {
    match layout {
        SavedLayout::Pane { active, tabs } => {
            // Model::restore makes at least one tab per pane; a blank entry keeps `out` aligned
            // with the tabs it returns.
            if tabs.is_empty() {
                out.push(SavedTab::default());
            }
            let specs = tabs
                .iter()
                .map(|t| match &t.url {
                    Some(url) => TabSpec::Browser(url.clone()),
                    None => TabSpec::Terminal,
                })
                .collect();
            out.extend(tabs);
            LayoutSpec::Pane {
                tabs: specs,
                active,
            }
        }
        SavedLayout::Split {
            axis,
            ratio,
            first,
            second,
        } => LayoutSpec::Split {
            axis: match axis {
                SavedAxis::Row => Axis::Row,
                SavedAxis::Column => Axis::Column,
            },
            ratio,
            first: Box::new(spec_from_layout(*first, out)),
            second: Box::new(spec_from_layout(*second, out)),
        },
    }
}

/// Reads the session file. `None` if it's missing, unreadable, another version, or has no
/// workspaces.
pub fn load(path: &Path) -> Option<Saved> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Saved>(&text)
        .ok()
        .filter(|s| s.version == VERSION && !s.workspaces.is_empty())
}

/// Writes the session file, creating the directory if needed. Writes to a temporary file first,
/// so a crash mid-write can't leave a truncated session.
pub fn save(path: &Path, saved: &Saved) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(saved).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Deletes the session file, so the next launch starts fresh. A missing file is fine.
pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Whether `id` is safe to type into a shell as `claude --resume <id>`: 1 to 128 ASCII
/// letters, digits, `-` or `_`. Claude Code session ids are UUIDs; anything else (which could
/// carry shell syntax from a tampered file) isn't typed.
pub fn is_safe_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use macaroni_core::{AgentState, Command, Direction};

    use super::*;

    #[test]
    fn capture_and_restore_keep_layout_dirs_and_agent_sessions() {
        let (mut model, _) = Model::new();
        let first = model.focused_tab().id;
        model.apply(Command::SetAgent {
            tab: first,
            state: AgentState::Working,
            detail: None,
            session_id: Some("1f2e-33".into()),
            cwd: Some("C:/repo".into()),
        });
        model.apply(Command::SplitPane(Direction::Down));
        model.apply(Command::NewTab);
        model.apply(Command::OpenBrowser {
            url: "https://example.com/".into(),
            split: None,
        });
        model.apply(Command::RenameTab(first, Some("agent".into())));
        let ws = model.active_workspace().id;
        model.apply(Command::SetWorkspaceIcon(ws, Some("rocket".into())));
        model.apply(Command::NewGroup(ws));
        model.apply(Command::SetTabMuted(first, true));
        model.apply(Command::SetTabPinned(first, true));

        let saved = capture(&model, &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new());
        let json = serde_json::to_string(&saved).unwrap();
        let reloaded: Saved = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded, saved);
        assert_eq!(reloaded.workspaces[0].icon.as_deref(), Some("rocket"));
        assert_eq!(reloaded.workspaces[0].group, Some(0));
        assert_eq!(reloaded.groups[0].name, "Group 1");

        let (spec, tabs) = into_spec(reloaded);
        assert_eq!(tabs.len(), 4);
        assert_eq!(tabs[3].url.as_deref(), Some("https://example.com/"));
        assert_eq!(tabs[0].agent_session.as_deref(), Some("1f2e-33"));
        assert_eq!(tabs[0].cwd.as_deref(), Some("C:/repo"));
        assert_eq!(tabs[0].name.as_deref(), Some("agent"));
        assert!(tabs[0].muted && tabs[0].pinned);
        assert_eq!(tabs[1], SavedTab::default());

        let (restored, _, new_tabs) = Model::restore(spec);
        assert_eq!(new_tabs.len(), tabs.len());
        let browser = restored.tab(new_tabs[3]).unwrap();
        assert_eq!(browser.url.as_deref(), Some("https://example.com/"));
        assert_eq!(
            capture(
                &restored,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &BTreeMap::new()
            )
            .workspaces
            .len(),
            1
        );
    }

    #[test]
    fn unstarted_tabs_keep_what_was_restored_and_agents_keep_their_dir() {
        let (mut model, _) = Model::new();
        let first = model.focused_tab().id;
        model.apply(Command::NewTab);
        let second = model.focused_tab().id;
        // The shell last reported its old directory; the agent runs somewhere else.
        let TabKind::Terminal(session) = model.tab(first).unwrap().kind else {
            panic!("a new tab is a terminal");
        };
        model.apply(Command::SetTabCwd(session, Some("C:/Users/me".into())));
        model.apply(Command::SetAgent {
            tab: first,
            state: AgentState::Idle,
            detail: None,
            session_id: Some("1f2e-33".into()),
            cwd: Some("C:/repo".into()),
        });
        // The second tab was restored but never shown, so its terminal hasn't started.
        let restored = BTreeMap::from([(
            second,
            SavedTab {
                cwd: Some("C:/other".into()),
                agent_session: Some("4a5b-66".into()),
                ..SavedTab::default()
            },
        )]);

        let (_, tabs) = into_spec(capture(
            &model,
            &restored,
            &BTreeMap::new(),
            &BTreeMap::new(),
        ));
        assert_eq!(tabs[0].cwd.as_deref(), Some("C:/repo"));
        assert_eq!(tabs[0].agent_session.as_deref(), Some("1f2e-33"));
        assert_eq!(tabs[1].cwd.as_deref(), Some("C:/other"));
        assert_eq!(tabs[1].agent_session.as_deref(), Some("4a5b-66"));
        assert_eq!(tabs[0].ssh, None);
    }

    #[test]
    fn ssh_tabs_save_their_profile() {
        let (mut model, _) = Model::new();
        let first = model.focused_tab().id;
        model.apply(Command::NewTab);
        let second = model.focused_tab().id;
        let TabKind::Terminal(session) = model.tab(first).unwrap().kind else {
            panic!("a new tab is a terminal");
        };
        let ssh = BTreeMap::from([(session, "Pi".to_owned())]);
        let restored = BTreeMap::from([(
            second,
            SavedTab {
                ssh: Some("Work".into()),
                ..SavedTab::default()
            },
        )]);

        let (_, tabs) = into_spec(capture(&model, &restored, &ssh, &BTreeMap::new()));
        assert_eq!(tabs[0].ssh.as_deref(), Some("Pi"));
        assert_eq!(tabs[1].ssh.as_deref(), Some("Work"));
    }

    #[test]
    fn session_ids_are_checked_before_use() {
        assert!(is_safe_session_id("0f8a2c1e-9b7d-4c3e-8a1f-2b3c4d5e6f70"));
        assert!(!is_safe_session_id("x; rm -rf ~"));
        assert!(!is_safe_session_id("$(evil)"));
        assert!(!is_safe_session_id(""));
    }

    #[test]
    fn unknown_versions_and_empty_sessions_are_ignored() {
        let dir =
            std::env::temp_dir().join(format!("macaroni-session-test-{}", std::process::id()));
        let file = path(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, r#"{"version":99,"active":0,"workspaces":[]}"#).unwrap();
        assert_eq!(load(&file), None);
        let (model, _) = Model::new();
        save(
            &file,
            &capture(&model, &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new()),
        )
        .unwrap();
        assert!(load(&file).is_some());
        forget(&file);
        assert_eq!(load(&file), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
