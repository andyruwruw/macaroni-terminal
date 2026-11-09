//! macaroni's built-in plugins, run by the app like any other plugin: the panel's JSON on
//! stdin, one row per line printed (ANSI colors allowed), nothing printed for no row.
//!
//! `cache` shows whether a coding agent's prompt cache is likely still warm: providers keep a
//! conversation's cached prefix for a few minutes after its last request, and the transcript is
//! written on every request and reply, so the time since the transcript last changed says how
//! long the cache has been idle.

use std::time::{Duration, SystemTime};

use serde_json::Value;

/// Minutes a prompt cache lasts unless the plugin's settings say otherwise: Anthropic's default
/// cache lifetime, and the low end of OpenAI's.
const DEFAULT_MINUTES: f64 = 5.0;

/// `macaroni-cli plugin cache`: prints the cache row for the panel on stdin, or nothing when it
/// has no agent transcript.
pub fn cache() {
    let input = crate::claude::stdin_json().unwrap_or(Value::Null);
    if let Some(row) = cache_row(&input, SystemTime::now()) {
        println!("{row}");
    }
}

/// How long the cache lasts for this panel's agent, from the plugin's settings:
/// `<agent>_minutes` (as `codex_minutes`), else `minutes`, else [`DEFAULT_MINUTES`].
fn lifetime(input: &Value) -> Duration {
    let settings = input.pointer("/plugin/settings");
    let kind = input
        .pointer("/agent/kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    let minutes = settings
        .and_then(|s| {
            s.get(format!("{kind}_minutes"))
                .or_else(|| s.get("minutes"))
        })
        .and_then(Value::as_f64)
        .filter(|m| m.is_finite() && *m > 0.0)
        .unwrap_or(DEFAULT_MINUTES);
    Duration::from_secs_f64(minutes * 60.0)
}

/// `d` as `4m` from a minute up, else `45s`.
fn short(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// The row for `input` at `now`: warm (green) with the time left while the agent works or
/// its transcript changed within the cache's lifetime, yellow in its last minute, cold (dim)
/// after. `None` without a readable transcript.
pub fn cache_row(input: &Value, now: SystemTime) -> Option<String> {
    let path = input
        .pointer("/agent/transcript_path")
        .or_else(|| input.get("transcript_path"))?
        .as_str()?;
    let changed = std::fs::metadata(path).ok()?.modified().ok()?;
    let idle = now.duration_since(changed).unwrap_or_default();
    let working = input.pointer("/agent/state").and_then(Value::as_str) == Some("working");
    let lifetime = lifetime(input);
    Some(if working {
        "\x1b[32m\u{25cf}\x1b[0m cache warm \x1b[2m\u{b7} working\x1b[0m".to_owned()
    } else if idle < lifetime {
        let left = lifetime - idle;
        let color = if left < Duration::from_secs(60) {
            33
        } else {
            32
        };
        format!(
            "\x1b[{color}m\u{25cf}\x1b[0m cache warm \x1b[2m\u{b7} {} left\x1b[0m",
            short(left)
        )
    } else {
        format!(
            "\x1b[2m\u{25cb} cache cold \u{b7} idle {}\x1b[0m",
            short(idle)
        )
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_cache_is_warm_then_cold() {
        let file =
            std::env::temp_dir().join(format!("macaroni-cache-{}.jsonl", std::process::id()));
        std::fs::write(&file, "{}").unwrap();
        let changed = std::fs::metadata(&file).unwrap().modified().unwrap();
        let input = |state: &str, settings: Value| {
            json!({"plugin": {"settings": settings},
                   "agent": {"kind": "codex", "state": state,
                             "transcript_path": file.to_string_lossy()}})
        };
        let at = |secs| changed + Duration::from_secs(secs);
        let row = |state, settings, secs| cache_row(&input(state, settings), at(secs)).unwrap();

        assert!(row("idle", json!({}), 60).contains("cache warm \x1b[2m\u{b7} 4m left"));
        assert!(row("idle", json!({}), 250).starts_with("\x1b[33m"));
        assert!(row("idle", json!({}), 720).contains("cache cold \u{b7} idle 12m"));
        assert!(row("working", json!({}), 720).contains("working"));
        // Settings: the agent's own lifetime wins over the general one.
        let settings = json!({"minutes": 60, "codex_minutes": 10});
        assert!(row("idle", settings, 720).contains("cache cold"));
        assert!(row("idle", json!({"minutes": 60}), 720).contains("48m left"));

        assert_eq!(cache_row(&json!({"agent": {"kind": "codex"}}), at(0)), None);
        let _ = std::fs::remove_file(&file);
    }
}
