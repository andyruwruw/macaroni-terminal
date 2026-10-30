//! Claude Code integration: `hook` and `statusline` are what Claude Code runs; `setup` wires
//! them into Claude Code's settings.
//!
//! [`hook_calls`] maps each hook event to [`Call::Agent`] (and for `Notification` and `Stop`,
//! [`Call::Notify`]) about the tab the hook runs in; [`summary`] builds the status line text sent
//! as [`Call::AgentSummary`]. [`configure`] is the pure settings merge that [`setup`] shows,
//! confirms and writes. Entries are recognized as ours by [`MARKER`] in their command, which is
//! what makes setup idempotent and `--remove` exact.

use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};

use macaroni_ipc::{AgentState, Call, TAB_ENV};
use serde_json::{Map, Value, json};

use crate::client::Client;

/// Hook events macaroni listens to, and whether they need a tool matcher.
const HOOK_EVENTS: [(&str, bool); 6] = [
    ("SessionStart", false),
    ("UserPromptSubmit", false),
    ("PreToolUse", true),
    ("Notification", false),
    ("Stop", false),
    ("SessionEnd", false),
];

/// Marks entries in Claude Code's settings as ours, so setup can update or remove them.
const MARKER: &str = "macaroni-cli";

/// The JSON object Claude Code passes on stdin; `None` if stdin isn't readable or isn't JSON.
/// Reads at most 4 MiB so a runaway input can't stall the hook.
fn stdin_json() -> Option<Value> {
    let mut input = String::new();
    std::io::stdin()
        .take(4 * 1024 * 1024)
        .read_to_string(&mut input)
        .ok()?;
    serde_json::from_str(&input).ok()
}

/// The tab this runs in, from `MACARONI_TAB_ID`; `None` outside macaroni. Unlike the CLI's
/// `--tab` default, there's no fallback to the focused tab: a hook must only ever report on its
/// own tab.
fn own_tab() -> Option<u64> {
    std::env::var(TAB_ENV).ok()?.parse().ok()
}

/// `value[key]` as an owned string; `None` if it's missing or not a string.
fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

/// The first line of `text`, trimmed, cut to `max` characters with an ellipsis if longer.
/// `max` must be at least 1.
fn shorten(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        line.to_owned()
    } else {
        let mut short: String = line.chars().take(max - 1).collect();
        short.push('…');
        short
    }
}

/// The last component of a path with either separator, since tool inputs may use both.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A short description of what a tool call is doing, for the tab header and sidebar.
pub fn describe_tool(name: &str, input: &Value) -> String {
    let field = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("");
    match name {
        "Bash" | "PowerShell" => format!("Running {}", shorten(field("command"), 48)),
        "Read" => format!("Reading {}", file_name(field("file_path"))),
        "Edit" | "MultiEdit" | "Write" => format!("Editing {}", file_name(field("file_path"))),
        "NotebookEdit" => format!("Editing {}", file_name(field("notebook_path"))),
        "Grep" => format!("Searching for {}", shorten(field("pattern"), 40)),
        "Glob" => format!("Finding {}", shorten(field("pattern"), 40)),
        "WebFetch" => {
            let url = field("url");
            let host = url
                .split("//")
                .nth(1)
                .unwrap_or(url)
                .split('/')
                .next()
                .unwrap_or(url);
            format!("Fetching {host}")
        }
        "WebSearch" => "Searching the web".to_owned(),
        "Task" | "Agent" => "Running a subagent".to_owned(),
        other => other.rsplit("__").next().unwrap_or(other).to_owned(),
    }
}

/// The macaroni calls for one Claude Code hook event.
pub fn hook_calls(input: &Value, tab: u64) -> Vec<Call> {
    let session_id = string(input, "session_id");
    let cwd = string(input, "cwd");
    let agent = |state, detail: Option<String>| Call::Agent {
        tab: Some(tab),
        state,
        detail,
        session_id: session_id.clone(),
        cwd: cwd.clone(),
    };
    let notify = |body: String| Call::Notify {
        tab: Some(tab),
        title: Some("Claude Code".to_owned()),
        body,
    };
    match input.get("hook_event_name").and_then(Value::as_str) {
        Some("SessionStart") => vec![agent(AgentState::Idle, None)],
        Some("UserPromptSubmit") => vec![agent(AgentState::Working, None)],
        Some("PreToolUse") => {
            let name = input
                .get("tool_name")
                .and_then(Value::as_str)
                .unwrap_or("tool");
            let detail = describe_tool(name, input.get("tool_input").unwrap_or(&Value::Null));
            vec![agent(AgentState::Working, Some(detail))]
        }
        Some("Notification") => {
            let message =
                string(input, "message").unwrap_or_else(|| "Claude Code needs you".to_owned());
            vec![
                agent(AgentState::NeedsInput, Some(message.clone())),
                notify(message),
            ]
        }
        Some("Stop") => vec![
            agent(AgentState::Idle, Some("Done".to_owned())),
            notify("Finished responding".to_owned()),
        ],
        Some("SessionEnd") => vec![agent(AgentState::Ended, None)],
        _ => Vec::new(),
    }
}

/// `macaroni-cli hook`: run by Claude Code for each hook event. Never fails and never prints:
/// a hook must not block Claude Code or add to its context, and outside macaroni it does nothing.
pub fn hook() {
    let (Some(input), Some(tab)) = (stdin_json(), own_tab()) else {
        return;
    };
    let calls = hook_calls(&input, tab);
    if calls.is_empty() {
        return;
    }
    if let Ok(mut client) = Client::connect() {
        for call in calls {
            let _ = client.call(call);
        }
    }
}

/// The status line text: model, context use and cost, from Claude Code's status line input.
pub fn summary(input: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(model) = input.pointer("/model/display_name").and_then(Value::as_str) {
        parts.push(model.to_owned());
    }
    if let Some(used) = input
        .pointer("/context_window/used_percentage")
        .and_then(Value::as_f64)
    {
        parts.push(format!("{used:.0}% context"));
    }
    if let Some(cost) = input
        .pointer("/cost/total_cost_usd")
        .and_then(Value::as_f64)
    {
        parts.push(format!("${cost:.2}"));
    }
    parts.join(" · ")
}

/// `macaroni-cli statusline`: prints a status line for Claude Code and forwards it to macaroni.
pub fn statusline() {
    let input = stdin_json().unwrap_or(Value::Null);
    let text = summary(&input);
    println!("{text}");
    if let Some(tab) = own_tab()
        && let Ok(mut client) = Client::connect()
    {
        let _ = client.call(Call::AgentSummary {
            tab: Some(tab),
            summary: Some(text),
        });
    }
}

/// `~/.claude/settings.json`, from `HOME` (or `USERPROFILE` on Windows).
fn settings_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .ok_or("can't find your home folder (HOME / USERPROFILE)")?;
    Ok(PathBuf::from(home).join(".claude").join("settings.json"))
}

/// Whether a hook entry or status line was written by setup: its `command` mentions
/// [`MARKER`].
fn ours(entry: &Value) -> bool {
    entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| c.contains(MARKER))
}

/// Removes our hooks from Claude Code settings, dropping groups and events left empty.
fn remove_hooks(settings: &mut Map<String, Value>) {
    let Some(Value::Object(hooks)) = settings.get_mut("hooks") else {
        return;
    };
    for groups in hooks.values_mut() {
        if let Value::Array(groups) = groups {
            for group in groups.iter_mut() {
                if let Some(Value::Array(entries)) = group.get_mut("hooks") {
                    entries.retain(|e| !ours(e));
                }
            }
            groups.retain(|g| {
                g.get("hooks")
                    .and_then(Value::as_array)
                    .is_none_or(|e| !e.is_empty())
            });
        }
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|g| !g.is_empty()));
    if hooks.is_empty() {
        settings.remove("hooks");
    }
}

/// Claude Code's settings with macaroni's hooks and status line added (or removed).
/// Existing user hooks and an existing, different status line are left alone.
pub fn configure(
    mut settings: Map<String, Value>,
    exe: &str,
    remove: bool,
) -> (Map<String, Value>, Vec<String>) {
    let mut notes = Vec::new();
    remove_hooks(&mut settings);
    let statusline_is_ours = settings.get("statusLine").is_some_and(ours);
    if remove {
        if statusline_is_ours {
            settings.remove("statusLine");
        }
        return (settings, notes);
    }

    let command = |sub: &str| format!("\"{exe}\" {sub}");
    let hooks = settings
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(hooks) = hooks {
        for (event, needs_matcher) in HOOK_EVENTS {
            let mut group = Map::new();
            if needs_matcher {
                group.insert("matcher".into(), json!("*"));
            }
            group.insert(
                "hooks".into(),
                json!([{ "type": "command", "command": command("hook"), "timeout": 5 }]),
            );
            let groups = hooks.entry(event).or_insert_with(|| json!([]));
            if let Value::Array(groups) = groups {
                groups.push(Value::Object(group));
            }
        }
    }

    match settings.get("statusLine") {
        None => {
            settings.insert(
                "statusLine".into(),
                json!({ "type": "command", "command": command("statusline") }),
            );
        }
        Some(_) if statusline_is_ours => {
            settings.insert(
                "statusLine".into(),
                json!({ "type": "command", "command": command("statusline") }),
            );
        }
        Some(_) => notes.push(
            "You already have a status line, so it was left alone; macaroni's pane headers \
             won't show the model and context until it runs `macaroni-cli statusline`."
                .into(),
        ),
    }
    (settings, notes)
}

/// This executable's absolute path, with forward slashes: Windows accepts them, and they need
/// no escaping in the JSON or in the shell Claude Code runs hooks through (Git Bash on Windows).
fn exe_path() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(exe.to_string_lossy().replace('\\', "/"))
}

/// Asks a yes/no question on stdout and reads the answer from stdin; anything but `y`/`yes`
/// (including a closed stdin) is no.
fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).is_ok()
        && matches!(answer.trim(), "y" | "Y" | "yes")
}

/// `macaroni-cli setup claude`: merges macaroni's hooks and status line into Claude Code's
/// settings at `path` (default `~/.claude/settings.json`), or removes them with `remove`.
///
/// Prints the plan and asks first unless `yes`. Before writing, copies the old file to
/// `<file>.macaroni-backup`. Does nothing if the file already matches, and refuses to touch a
/// file that isn't valid JSON. Writes pretty-printed JSON; `serde_json`'s `preserve_order`
/// feature keeps the user's key order.
pub fn setup(remove: bool, yes: bool, path: Option<PathBuf>) -> Result<(), String> {
    let path = match path {
        Some(path) => path,
        None => settings_path()?,
    };
    let current: Map<String, Value> = match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} isn't valid JSON ({e}); fix it first, nothing was changed",
                path.display()
            )
        })?,
        _ => Map::new(),
    };
    let exe = exe_path()?;
    let (updated, notes) = configure(current.clone(), &exe, remove);
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

    println!("Claude Code settings: {}", path.display());
    if remove {
        println!("  remove macaroni's hooks and status line");
    } else {
        let events: Vec<_> = HOOK_EVENTS.iter().map(|(e, _)| *e).collect();
        println!("  hooks ({}): \"{exe}\" hook", events.join(", "));
        if updated.get("statusLine").is_some_and(ours) {
            println!("  status line: \"{exe}\" statusline");
        }
    }
    for note in &notes {
        println!("  note: {note}");
    }
    if !yes && !confirm("Write these changes?") {
        println!("Nothing changed.");
        return Ok(());
    }

    if path.exists() {
        let backup = backup_path(&path);
        std::fs::copy(&path, &backup)
            .map_err(|e| format!("backing up to {}: {e}", backup.display()))?;
        println!("Backed up the previous settings to {}", backup.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut text =
        serde_json::to_string_pretty(&Value::Object(updated)).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
    println!(
        "Done. {}",
        if remove {
            "Claude Code no longer reports to macaroni."
        } else {
            "Start a new Claude Code session in a macaroni terminal to see it in the sidebar."
        }
    );
    Ok(())
}

/// `<path>.macaroni-backup`, next to the settings file.
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".macaroni-backup");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = "C:/tools/macaroni-cli.exe";

    #[test]
    fn tool_descriptions() {
        assert_eq!(
            describe_tool("Bash", &json!({"command": "cargo test --workspace\nmore"})),
            "Running cargo test --workspace"
        );
        assert_eq!(
            describe_tool("Edit", &json!({"file_path": "C:\\repo\\src\\main.rs"})),
            "Editing main.rs"
        );
        assert_eq!(
            describe_tool("WebFetch", &json!({"url": "https://docs.rs/iced/latest"})),
            "Fetching docs.rs"
        );
        assert_eq!(
            describe_tool("mcp__github__create_issue", &json!({})),
            "create_issue"
        );
    }

    #[test]
    fn hook_events_map_to_agent_state_and_notifications() {
        let calls = hook_calls(
            &json!({"hook_event_name": "Notification", "session_id": "s1", "cwd": "/w",
                    "message": "Claude needs your permission to use Bash"}),
            7,
        );
        assert_eq!(calls.len(), 2);
        assert!(matches!(
            &calls[0],
            Call::Agent { tab: Some(7), state: AgentState::NeedsInput, session_id: Some(s), .. } if s == "s1"
        ));
        assert!(matches!(&calls[1], Call::Notify { body, .. } if body.contains("permission")));

        let calls = hook_calls(
            &json!({"hook_event_name": "PreToolUse", "tool_name": "Bash",
                    "tool_input": {"command": "npm test"}}),
            7,
        );
        assert!(
            matches!(&calls[0], Call::Agent { detail: Some(d), .. } if d == "Running npm test")
        );
        assert!(hook_calls(&json!({"hook_event_name": "PostToolUse"}), 7).is_empty());
        assert!(matches!(
            hook_calls(&json!({"hook_event_name": "SessionEnd"}), 7)[0],
            Call::Agent {
                state: AgentState::Ended,
                ..
            }
        ));
    }

    #[test]
    fn status_line_summary() {
        let input = json!({"model": {"display_name": "Opus"},
                           "context_window": {"used_percentage": 41.6},
                           "cost": {"total_cost_usd": 0.4213}});
        assert_eq!(summary(&input), "Opus · 42% context · $0.42");
        assert_eq!(summary(&json!({})), "");
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn setup_adds_hooks_and_keeps_the_users_own() {
        let mine = obj(json!({
            "model": "opus",
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}
        }));
        let (updated, notes) = configure(mine.clone(), EXE, false);
        assert!(notes.is_empty());
        assert_eq!(updated["model"], "opus");
        let stop = updated["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2, "the user's Stop hook stays");
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(updated["hooks"]["PreToolUse"][0]["matcher"], "*");
        assert_eq!(
            updated["statusLine"]["command"],
            "\"C:/tools/macaroni-cli.exe\" statusline"
        );

        let (again, _) = configure(updated.clone(), EXE, false);
        assert_eq!(again, updated, "running setup twice changes nothing");

        let (removed, _) = configure(updated, EXE, true);
        assert_eq!(removed, mine, "remove restores the original");
    }

    #[test]
    fn an_existing_status_line_is_left_alone() {
        let mine = obj(json!({"statusLine": {"type": "command", "command": "my-line"}}));
        let (updated, notes) = configure(mine, EXE, false);
        assert_eq!(updated["statusLine"]["command"], "my-line");
        assert_eq!(notes.len(), 1);
    }
}
