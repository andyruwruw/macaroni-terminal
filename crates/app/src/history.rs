//! The commands run in each workspace, remembered with the pane each ran in, for the command
//! history picker (`command_history`, a mode of the palette in `view/palette.rs`).
//!
//! A command is recorded when the shell marks its output starting (OSC 133 `C`, read off the
//! grid by `Emulator::typed_command`), so only shells with macaroni's integration report them.
//! Each workspace keeps its own list; running a command again in the same pane counts it once
//! more instead of adding a line. [`History::listed`] merges a workspace's panes into one list:
//! the most run commands first, then the pane's own, then the rest of the workspace's, newest
//! first. The lists are saved with the session (`SavedWorkspace::commands`), each entry's pane
//! by its place in the layout.

use std::cmp::Reverse;
use std::collections::HashMap;

use macaroni_core::{PaneId, WorkspaceId};

use crate::session::SavedCommand;

/// The most commands kept per workspace; past it, the one run longest ago goes.
const MAX_KEPT: usize = 300;

/// How many of the most run commands lead the list.
const COMMON: usize = 5;

/// How many runs make a command one of the most run.
const COMMON_RUNS: u32 = 3;

/// A command as recorded in one pane.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ran {
    /// The command line, as shown after the prompt.
    command: String,
    /// Where it ran; `None` for a pane from before a restart that's no longer in the layout.
    pane: Option<PaneId>,
    /// How many times it ran there.
    runs: u32,
    /// When it last ran, in seconds since the Unix epoch.
    last: u64,
}

/// Every workspace's commands.
#[derive(Debug, Default)]
pub(crate) struct History {
    /// Each workspace's commands, in the order they were first run.
    workspaces: HashMap<WorkspaceId, Vec<Ran>>,
}

/// One line of the picker: a command and what's known about it across the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listed {
    /// The command line.
    pub(crate) command: String,
    /// One of the workspace's most run commands, listed first.
    pub(crate) common: bool,
    /// It ran in the pane the picker is for.
    pub(crate) here: bool,
    /// How many times it ran in the workspace.
    pub(crate) runs: u32,
    /// When it last ran, in seconds since the Unix epoch.
    pub(crate) last: u64,
}

impl History {
    /// Records `command` running in `pane` of `ws` at `now` (seconds since the Unix epoch).
    pub(crate) fn record(&mut self, ws: WorkspaceId, pane: PaneId, command: String, now: u64) {
        let list = self.workspaces.entry(ws).or_default();
        if let Some(ran) = list
            .iter_mut()
            .find(|r| r.pane == Some(pane) && r.command == command)
        {
            ran.runs += 1;
            ran.last = now;
            return;
        }
        list.push(Ran {
            command,
            pane: Some(pane),
            runs: 1,
            last: now,
        });
        if list.len() > MAX_KEPT
            && let Some(oldest) = (0..list.len()).min_by_key(|&i| list[i].last)
        {
            list.remove(oldest);
        }
    }

    /// Forgets `command` in every pane of `ws`.
    pub(crate) fn forget(&mut self, ws: WorkspaceId, command: &str) {
        if let Some(list) = self.workspaces.get_mut(&ws) {
            list.retain(|r| r.command != command);
        }
    }

    /// The commands of `ws` containing every word of `query` (ignoring case), for the picker
    /// on `pane`: up to [`COMMON`] commands run at least [`COMMON_RUNS`] times, most run first,
    /// then the ones run in `pane`, then the rest, each newest first.
    pub(crate) fn listed(&self, ws: WorkspaceId, pane: PaneId, query: &str) -> Vec<Listed> {
        let mut merged: Vec<Listed> = Vec::new();
        for ran in self
            .workspaces
            .get(&ws)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let here = ran.pane == Some(pane);
            match merged.iter_mut().find(|l| l.command == ran.command) {
                Some(listed) => {
                    listed.runs += ran.runs;
                    listed.last = listed.last.max(ran.last);
                    listed.here |= here;
                }
                None => merged.push(Listed {
                    command: ran.command.clone(),
                    common: false,
                    here,
                    runs: ran.runs,
                    last: ran.last,
                }),
            }
        }
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        merged.retain(|l| {
            let command = l.command.to_lowercase();
            words.iter().all(|w| command.contains(w.as_str()))
        });
        let mut most: Vec<usize> = (0..merged.len())
            .filter(|&i| merged[i].runs >= COMMON_RUNS)
            .collect();
        most.sort_by_key(|&i| (Reverse(merged[i].runs), Reverse(merged[i].last)));
        for &i in most.iter().take(COMMON) {
            merged[i].common = true;
        }
        merged.sort_by_key(|l| {
            (
                !l.common,
                Reverse(if l.common { l.runs } else { 0 }),
                !l.here,
                Reverse(l.last),
            )
        });
        merged
    }

    /// What's saved for `ws`, whose layout's panes are `leaves` (in leaf order).
    pub(crate) fn saved(&self, ws: WorkspaceId, leaves: &[PaneId]) -> Vec<SavedCommand> {
        self.workspaces
            .get(&ws)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|ran| SavedCommand {
                command: ran.command.clone(),
                pane: ran.pane.and_then(|p| leaves.iter().position(|l| *l == p)),
                runs: ran.runs,
                last: ran.last,
            })
            .collect()
    }

    /// Gives restored workspace `ws`, whose layout's panes are `leaves`, its saved commands.
    pub(crate) fn restore(&mut self, ws: WorkspaceId, leaves: &[PaneId], saved: &[SavedCommand]) {
        if saved.is_empty() {
            return;
        }
        let list = saved
            .iter()
            .map(|s| Ran {
                command: s.command.clone(),
                pane: s.pane.and_then(|i| leaves.get(i).copied()),
                runs: s.runs.max(1),
                last: s.last,
            })
            .collect();
        self.workspaces.insert(ws, list);
    }
}

/// Now, in seconds since the Unix epoch.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How long before `now` `then` was, briefly: "just now", "5m ago", "3h ago", "2d ago".
pub(crate) fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86_400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS: WorkspaceId = WorkspaceId(1);
    const A: PaneId = PaneId(2);
    const B: PaneId = PaneId(3);

    fn commands(listed: &[Listed]) -> Vec<&str> {
        listed.iter().map(|l| l.command.as_str()).collect()
    }

    #[test]
    fn running_again_counts_instead_of_adding() {
        let mut history = History::default();
        history.record(WS, A, "ls".into(), 1);
        history.record(WS, A, "ls".into(), 2);
        history.record(WS, B, "ls".into(), 3);
        let listed = history.listed(WS, A, "");
        assert_eq!(commands(&listed), ["ls"]);
        assert_eq!((listed[0].runs, listed[0].last), (3, 3));
        assert_eq!(history.workspaces[&WS].len(), 2, "one per pane");
    }

    #[test]
    fn most_run_then_this_pane_then_the_rest() {
        let mut history = History::default();
        for t in 0..3 {
            history.record(WS, B, "cargo test".into(), t);
        }
        history.record(WS, B, "git status".into(), 10);
        history.record(WS, A, "npm run dev".into(), 5);
        history.record(WS, A, "cd web".into(), 4);
        let listed = history.listed(WS, A, "");
        assert_eq!(
            commands(&listed),
            ["cargo test", "npm run dev", "cd web", "git status"]
        );
        assert!(listed[0].common && !listed[0].here);
        assert!(listed[1].here);
    }

    #[test]
    fn every_word_must_match() {
        let mut history = History::default();
        history.record(WS, A, "cargo test -p term".into(), 1);
        history.record(WS, A, "cargo build".into(), 2);
        assert_eq!(
            commands(&history.listed(WS, A, "TEST cargo")),
            ["cargo test -p term"]
        );
        assert!(history.listed(WS, A, "make").is_empty());
    }

    #[test]
    fn forgetting_takes_it_from_every_pane() {
        let mut history = History::default();
        history.record(WS, A, "rm -rf build".into(), 1);
        history.record(WS, B, "rm -rf build".into(), 2);
        history.forget(WS, "rm -rf build");
        assert!(history.listed(WS, A, "").is_empty());
    }

    #[test]
    fn the_oldest_goes_past_the_limit() {
        let mut history = History::default();
        for i in 0..=MAX_KEPT as u64 {
            history.record(WS, A, format!("echo {i}"), i);
        }
        let listed = history.listed(WS, A, "");
        assert_eq!(listed.len(), MAX_KEPT);
        assert!(!listed.iter().any(|l| l.command == "echo 0"));
    }

    #[test]
    fn saved_panes_go_by_their_place_in_the_layout() {
        let mut history = History::default();
        history.record(WS, B, "make".into(), 7);
        let saved = history.saved(WS, &[A, B]);
        assert_eq!(saved[0].pane, Some(1));
        let mut restored = History::default();
        let (c, d) = (PaneId(8), PaneId(9));
        restored.restore(WorkspaceId(4), &[c, d], &saved);
        let listed = restored.listed(WorkspaceId(4), d, "");
        assert!(listed[0].here);
        assert_eq!(listed[0].last, 7);
    }

    #[test]
    fn ago_is_brief() {
        assert_eq!(ago(100, 90), "just now");
        assert_eq!(ago(1000, 400), "10m ago");
        assert_eq!(ago(8000, 0), "2h ago");
        assert_eq!(ago(200_000, 0), "2d ago");
    }
}
