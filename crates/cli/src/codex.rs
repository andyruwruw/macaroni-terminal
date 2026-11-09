//! Codex integration: `codex-notify` is what Codex runs after each turn; `setup` wires it into
//! Codex's `config.toml` as its `notify` program.
//!
//! Codex has no hooks like Claude Code's, only `notify`: a program it runs with a JSON argument
//! when a turn completes (`"type": "agent-turn-complete"`, with `thread-id`, `cwd` and
//! `last-assistant-message`). [`notify_calls`] turns that into [`Call::Agent`] (idle, with the
//! session id and the transcript Codex writes to, found by [`transcript`]) and [`Call::Notify`].
//! A user's own `notify` program is kept: setup puts it after `--` in ours, and
//! `codex-notify` runs it with the same argument.

use std::path::{Path, PathBuf};

use macaroni_ipc::{AgentKind, AgentState, Call, TAB_ENV};
use serde_json::Value;
use toml_edit::{Array, DocumentMut, Item, value};

use crate::client::Client;

/// Marks a `notify` command as ours: its program's path mentions this.
const MARKER: &str = "macaroni-cli";

/// The subcommand Codex runs.
const SUBCOMMAND: &str = "codex-notify";

/// Codex's folder: `CODEX_HOME`, else `~/.codex`.
fn codex_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".codex"))
}

/// The folders in `dir`, newest name first (Codex names them by year, month and day).
fn newest_first(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs.reverse();
    dirs
}

/// The day folders under `sessions` (`YYYY/MM/DD`), newest first.
fn days(sessions: &Path) -> impl Iterator<Item = PathBuf> {
    newest_first(sessions)
        .into_iter()
        .flat_map(|year| newest_first(&year))
        .flat_map(|month| newest_first(&month))
}

/// The `cwd` in a rollout file's first line (its `session_meta`), if it has one.
fn rollout_cwd(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(file).read_line(&mut line).ok()?;
    let meta: Value = serde_json::from_str(&line).ok()?;
    meta.pointer("/payload/cwd")?.as_str().map(str::to_owned)
}

/// The session id in a rollout file's name: `rollout-<time>-<id>.jsonl`, the id a UUID.
fn rollout_id(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let id = stem.get(stem.len().checked_sub(36)?..)?;
    (id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).then(|| id.to_owned())
}

/// The rollout file (`sessions/YYYY/MM/DD/rollout-*.jsonl`) of session `id`, or, without an id
/// (older Codex), the most recently written one in the last few days that ran in `cwd`: the
/// turn that just ended wrote to it. Returns the file and its session id.
pub fn transcript(
    sessions: &Path,
    id: Option<&str>,
    cwd: Option<&str>,
) -> Option<(PathBuf, String)> {
    let rollouts = |day: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(day)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
            })
            .collect()
    };
    if let Some(id) = id {
        // A resumed session keeps writing to the file it started in, which can be days old.
        let suffix = format!("{id}.jsonl");
        return days(sessions).find_map(|day| {
            rollouts(&day)
                .into_iter()
                .find(|p| p.to_string_lossy().ends_with(&suffix))
                .map(|p| (p, id.to_owned()))
        });
    }
    let mut recent: Vec<_> = days(sessions).take(3).flat_map(|d| rollouts(&d)).collect();
    recent.sort_by_key(|p| std::cmp::Reverse(p.metadata().and_then(|m| m.modified()).ok()));
    recent
        .into_iter()
        .find(|p| cwd.is_none_or(|cwd| rollout_cwd(p).as_deref() == Some(cwd)))
        .and_then(|p| Some((p.clone(), rollout_id(&p)?)))
}

/// The macaroni calls for one Codex `notify` payload: the agent, idle, with its session and
/// transcript, then a notification with the start of its last message. Only
/// `agent-turn-complete` is reported.
pub fn notify_calls(payload: &Value, tab: u64, sessions: Option<&Path>) -> Vec<Call> {
    if payload.get("type").and_then(Value::as_str) != Some("agent-turn-complete") {
        return Vec::new();
    }
    let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
    let cwd = text("cwd");
    let found = sessions.and_then(|s| transcript(s, text("thread-id").as_deref(), cwd.as_deref()));
    let (transcript, session_id) = match found {
        Some((path, id)) => (Some(path.to_string_lossy().into_owned()), Some(id)),
        None => (None, text("thread-id")),
    };
    let message = text("last-assistant-message")
        .map(|m| crate::claude::shorten(&m, 120))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "Finished responding".to_owned());
    vec![
        Call::Agent {
            tab: Some(tab),
            kind: AgentKind::Codex,
            state: AgentState::Idle,
            detail: Some("Done".to_owned()),
            session_id,
            cwd,
            transcript_path: transcript,
        },
        Call::Notify {
            tab: Some(tab),
            title: Some("Codex".to_owned()),
            body: message,
        },
    ]
}

/// `macaroni-cli codex-notify [-- <program>...] <json>`: run by Codex after each turn. Reports
/// to macaroni when run in one of its terminals, then runs the user's own `notify` program (the
/// words after `--`) with the same JSON. Never fails and never prints, like Claude Code's hook.
pub fn notify(args: &[String]) {
    let Some((json, then)) = args.split_last() else {
        return;
    };
    if let (Ok(payload), Some(tab)) = (
        serde_json::from_str::<Value>(json),
        std::env::var(TAB_ENV).ok().and_then(|t| t.parse().ok()),
    ) {
        let sessions = codex_home().map(|h| h.join("sessions"));
        let calls = notify_calls(&payload, tab, sessions.as_deref());
        if !calls.is_empty()
            && let Ok(mut client) = Client::connect()
        {
            for call in calls {
                let _ = client.call(call);
            }
        }
    }
    let then = then.strip_prefix(&["--".to_owned()][..]).unwrap_or(then);
    if let Some((program, rest)) = then.split_first() {
        let _ = std::process::Command::new(program)
            .args(rest)
            .arg(json)
            .status();
    }
}

/// Whether a `notify` array is ours: its program mentions [`MARKER`].
fn ours(notify: &Array) -> bool {
    notify
        .get(0)
        .and_then(|v| v.as_str())
        .is_some_and(|p| p.contains(MARKER))
}

/// The user's own program kept in our `notify` (the words after `--`), if any.
fn kept(notify: &Array) -> Vec<String> {
    let words: Vec<String> = notify
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    match words.iter().position(|w| w == "--") {
        Some(at) => words[at + 1..].to_vec(),
        None => Vec::new(),
    }
}

/// Codex's `config.toml` text with macaroni's `notify` added (or removed with `remove`). A
/// `notify` of the user's own is kept and run after ours; removing puts it back. Fails on text
/// that isn't TOML or a `notify` that isn't a list of strings.
pub fn configure(text: &str, exe: &str, remove: bool) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("isn't valid TOML ({e})"))?;
    let current = match doc.get("notify") {
        None => None,
        Some(item) => Some(
            item.as_array()
                .filter(|a| a.iter().all(|v| v.is_str()))
                .ok_or("its notify isn't a list of strings")?
                .clone(),
        ),
    };
    let theirs = match &current {
        Some(notify) if ours(notify) => kept(notify),
        Some(notify) => notify
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        None => Vec::new(),
    };
    if remove {
        if current.as_ref().is_some_and(ours) {
            if theirs.is_empty() {
                doc.remove("notify");
            } else {
                doc["notify"] = value(Array::from_iter(theirs));
            }
        }
    } else {
        let mut notify = Array::from_iter([exe.to_owned(), SUBCOMMAND.to_owned()]);
        if !theirs.is_empty() {
            notify.push("--");
            notify.extend(theirs);
        }
        doc["notify"] = Item::Value(notify.into());
    }
    Ok(doc.to_string())
}

/// `macaroni-cli setup codex`: adds macaroni's `notify` to Codex's `config.toml` at `path`
/// (default `$CODEX_HOME/config.toml` or `~/.codex/config.toml`), or removes it with `remove`.
/// Prints the plan and asks first unless `yes`, and backs the old file up as Claude Code's
/// setup does.
pub fn setup(remove: bool, yes: bool, path: Option<PathBuf>) -> Result<(), String> {
    let path = match path {
        Some(path) => path,
        None => codex_home()
            .ok_or("can't find your home folder (HOME / USERPROFILE)")?
            .join("config.toml"),
    };
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let exe = crate::claude::exe_path()?;
    let updated = configure(&current, &exe, remove)
        .map_err(|e| format!("{} {e}; fix it first, nothing was changed", path.display()))?;
    if updated == current {
        println!(
            "{}: already {}.",
            path.display(),
            if remove {
                "free of macaroni"
            } else {
                "set up for macaroni"
            }
        );
        return Ok(());
    }
    println!("Codex config: {}", path.display());
    if remove {
        println!("  remove macaroni's notify program");
    } else {
        println!("  notify: \"{exe}\" {SUBCOMMAND}");
        let doc: Option<DocumentMut> = current.parse().ok();
        if let Some(theirs) = doc
            .as_ref()
            .and_then(|d| d.get("notify")?.as_array().cloned())
            .filter(|n| !ours(n))
        {
            println!(
                "  note: your notify program still runs, after macaroni's: {}",
                theirs.to_string().trim()
            );
        }
    }
    if !yes && !crate::claude::confirm("Write these changes?") {
        println!("Nothing changed.");
        return Ok(());
    }
    if path.exists() {
        let backup = crate::claude::backup_path(&path);
        std::fs::copy(&path, &backup)
            .map_err(|e| format!("backing up to {}: {e}", backup.display()))?;
        println!("Backed up the previous config to {}", backup.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, updated).map_err(|e| format!("writing {}: {e}", path.display()))?;
    println!(
        "Done. {}",
        if remove {
            "Codex no longer reports to macaroni."
        } else {
            "Codex reports to macaroni after each turn in a macaroni terminal."
        }
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A scratch folder for one test, emptied first.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("macaroni-codex-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const ID: &str = "0199a213-81c0-7800-8aa1-bbab2a035a53";

    #[test]
    fn finished_turns_report_the_session_and_its_transcript() {
        let sessions = scratch("turns");
        let day = sessions.join("2026").join("11").join("03");
        std::fs::create_dir_all(&day).unwrap();
        let file = day.join(format!("rollout-2026-11-03T10-00-00-{ID}.jsonl"));
        std::fs::write(
            &file,
            r#"{"type":"session_meta","payload":{"cwd":"C:/repo"}}"#,
        )
        .unwrap();
        // Newer days are searched too; the session's own file is found wherever it is.
        std::fs::create_dir_all(sessions.join("2026").join("11").join("04")).unwrap();

        let payload = json!({"type": "agent-turn-complete", "thread-id": ID, "cwd": "C:/repo",
                             "last-assistant-message": "All tests pass.\nDetails follow."});
        let calls = notify_calls(&payload, 3, Some(&sessions));
        let Call::Agent {
            kind,
            state,
            session_id,
            transcript_path,
            ..
        } = &calls[0]
        else {
            panic!("the agent comes first");
        };
        assert_eq!((*kind, *state), (AgentKind::Codex, AgentState::Idle));
        assert_eq!(session_id.as_deref(), Some(ID));
        assert_eq!(transcript_path.as_deref(), Some(&*file.to_string_lossy()));
        assert!(matches!(&calls[1], Call::Notify { body, .. } if body == "All tests pass."));

        // Without a thread id, the newest rollout from the same folder is the one.
        let payload = json!({"type": "agent-turn-complete", "cwd": "C:/repo"});
        let calls = notify_calls(&payload, 3, Some(&sessions));
        assert!(matches!(&calls[0], Call::Agent { session_id: Some(s), .. } if s == ID));
        let payload = json!({"type": "agent-turn-complete", "cwd": "C:/elsewhere"});
        let calls = notify_calls(&payload, 3, Some(&sessions));
        assert!(matches!(
            &calls[0],
            Call::Agent {
                session_id: None,
                transcript_path: None,
                ..
            }
        ));

        assert!(notify_calls(&json!({"type": "something-else"}), 3, Some(&sessions)).is_empty());
        let _ = std::fs::remove_dir_all(&sessions);
    }

    #[test]
    fn setup_adds_notify_and_keeps_the_users_own() {
        let text = "model = \"gpt-5\"\n\n[mcp_servers.docs]\ncommand = \"docs\"\n";
        let added = configure(text, "C:/bin/macaroni-cli.exe", false).unwrap();
        let doc: DocumentMut = added.parse().unwrap();
        assert_eq!(
            doc["notify"].as_array().unwrap().to_string().trim(),
            r#"["C:/bin/macaroni-cli.exe", "codex-notify"]"#
        );
        // Top level, before the tables, and the rest untouched.
        assert!(added.find("notify").unwrap() < added.find("[mcp_servers").unwrap());
        assert!(added.contains("[mcp_servers.docs]\ncommand = \"docs\""));
        assert_eq!(
            configure(&added, "C:/bin/macaroni-cli.exe", false).unwrap(),
            added
        );
        assert_eq!(
            configure(&added, "C:/bin/macaroni-cli.exe", true).unwrap(),
            text
        );

        let theirs = "notify = [\"say\", \"done\"]\n";
        let added = configure(theirs, "/bin/macaroni-cli", false).unwrap();
        let doc: DocumentMut = added.parse().unwrap();
        let words: Vec<_> = doc["notify"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            words,
            ["/bin/macaroni-cli", "codex-notify", "--", "say", "done"]
        );
        let removed: DocumentMut = configure(&added, "/bin/macaroni-cli", true)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            removed["notify"].as_array().unwrap().to_string().trim(),
            r#"["say", "done"]"#
        );

        assert!(configure("notify = 3\n", "x", false).is_err());
        assert!(configure("not toml [", "x", false).is_err());
    }
}
