//! Panel plugins: programs that add rows to the top or bottom of panels (tabs) they apply to.
//!
//! A plugin is a folder in `<config dir>/plugins` holding a `plugin.toml` ([`Manifest`]) and
//! whatever its command runs. The app runs the command for each panel the plugin is on in, with
//! the panel's details as JSON on stdin, and shows each line it prints as a row. macaroni's own
//! plugins ([`built_in`]) work the same way, run through `macaroni-cli plugin`.
//!
//! `config.toml`'s `[plugins.<id>]` tables override a plugin's `on`, `agents`, `kinds`,
//! `position` and `interval`; any other key there goes to the plugin as one of its settings.
//! [`load`] reads the folder and applies them. A broken `plugin.toml` doesn't stop the app: the
//! plugin is left out and the problem reported.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where a plugin's rows go in a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    /// Above the panel's content.
    Top,
    /// Below it.
    #[default]
    Bottom,
}

/// What a plugin runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// A command line for the system shell (`cmd /C` on Windows, `sh -c` elsewhere), run in the
    /// plugin's folder.
    Shell(String),
    /// A program and its arguments, run directly in the plugin's folder.
    Program(Vec<String>),
    /// `macaroni-cli` with these arguments (a built-in plugin).
    Cli(Vec<String>),
}

/// A plugin, with `config.toml`'s overrides applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    /// Its folder's name (a built-in's fixed name): how `config.toml` and the session refer to
    /// it.
    pub id: String,
    /// Shown in menus.
    pub name: String,
    /// One line on what it shows.
    pub description: String,
    /// What it runs.
    pub run: Run,
    /// Its folder, where it runs; `None` for a built-in.
    pub folder: Option<PathBuf>,
    /// Where its rows go.
    pub position: Position,
    /// Seconds between runs while a panel it's on is in view.
    pub interval: u32,
    /// Agents a panel must be running for the plugin to apply (`"claude"`, `"codex"`, or
    /// `"any"` for any agent); empty, it applies with or without one.
    pub agents: Vec<String>,
    /// Kinds of panel it applies to (`"terminal"`, `"browser"`, `"git"`, `"file"`, `"tmux"`,
    /// `"agent_setup"`, or `"any"`).
    pub kinds: Vec<String>,
    /// Whether it's on in the panels it applies to, unless a panel says otherwise.
    pub on: bool,
    /// Its settings: the manifest's `[settings]` with `config.toml`'s keys over them, passed to
    /// it in its input.
    pub settings: toml::Table,
}

impl Plugin {
    /// Whether it applies to a panel of `kind` running `agent` (if any).
    pub fn applies(&self, kind: &str, agent: Option<&str>) -> bool {
        let any = |list: &[String], name: &str| list.iter().any(|k| k == name || k == "any");
        any(&self.kinds, kind)
            && (self.agents.is_empty() || agent.is_some_and(|a| any(&self.agents, a)))
    }
}

/// The fewest seconds between runs.
pub const MIN_INTERVAL: u32 = 1;

/// The most seconds between runs: a day.
pub const MAX_INTERVAL: u32 = 86_400;

/// A plugin folder's `plugin.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    /// Shown in menus; defaults to the folder's name.
    #[serde(default)]
    name: Option<String>,
    /// One line on what it shows.
    #[serde(default)]
    description: String,
    /// What to run: a command line for the shell, or a program and its arguments.
    command: Command,
    /// Where its rows go.
    #[serde(default)]
    position: Position,
    /// Seconds between runs.
    #[serde(default = "default_interval")]
    interval: u32,
    /// Agents it needs.
    #[serde(default)]
    agents: Vec<String>,
    /// Kinds of panel.
    #[serde(default = "default_kinds")]
    kinds: Vec<String>,
    /// On by default.
    #[serde(default = "default_on")]
    on: bool,
    /// Its settings.
    #[serde(default)]
    settings: toml::Table,
}

/// A manifest's `command`: a string for the shell, or a list run directly.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Command {
    /// `command = "python rows.py"`.
    Shell(String),
    /// `command = ["python", "rows.py"]`.
    Program(Vec<String>),
}

/// The interval when a manifest has none: ten seconds.
fn default_interval() -> u32 {
    10
}

/// The kinds when a manifest has none: terminals.
fn default_kinds() -> Vec<String> {
    vec!["terminal".to_owned()]
}

/// Plugins are on unless they say otherwise.
fn default_on() -> bool {
    true
}

/// macaroni's own plugins.
pub fn built_in() -> Vec<Plugin> {
    vec![Plugin {
        id: "cache".to_owned(),
        name: "Prompt cache".to_owned(),
        description: "Whether a coding agent's prompt cache is still warm, and for how long."
            .to_owned(),
        run: Run::Cli(vec!["plugin".to_owned(), "cache".to_owned()]),
        folder: None,
        position: Position::Bottom,
        interval: 5,
        agents: vec!["claude".to_owned(), "codex".to_owned()],
        kinds: default_kinds(),
        on: true,
        settings: toml::Table::new(),
    }]
}

/// Whether `id` can name a plugin: letters, digits, `-` and `_`, as a TOML bare key and a
/// session key need.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The plugin in `folder`, from its `plugin.toml`; `None` for a folder without one.
fn read(folder: &Path, id: &str) -> Option<Result<Plugin, String>> {
    let path = folder.join("plugin.toml");
    let text = std::fs::read_to_string(&path).ok()?;
    let manifest: Manifest = match toml::from_str(&text) {
        Ok(manifest) => manifest,
        Err(e) => return Some(Err(format!("{}: {e}", path.display()))),
    };
    let run = match manifest.command {
        Command::Shell(line) if !line.trim().is_empty() => Run::Shell(line),
        Command::Program(words) if words.first().is_some_and(|p| !p.trim().is_empty()) => {
            Run::Program(words)
        }
        _ => return Some(Err(format!("{}: command is empty", path.display()))),
    };
    Some(Ok(Plugin {
        id: id.to_owned(),
        name: manifest.name.unwrap_or_else(|| id.to_owned()),
        description: manifest.description,
        run,
        folder: Some(folder.to_owned()),
        position: manifest.position,
        interval: manifest.interval,
        agents: manifest.agents,
        kinds: manifest.kinds,
        on: manifest.on,
        settings: manifest.settings,
    }))
}

/// Applies `config.toml`'s `[plugins.<id>]` table to `plugin`.
fn apply(plugin: &mut Plugin, table: &toml::Table) -> Result<(), String> {
    let id = &plugin.id;
    for (key, value) in table {
        let wrong = |what: &str| format!("plugins.{id}.{key} must be {what}");
        let strings = |value: &toml::Value| -> Result<Vec<String>, String> {
            value
                .as_array()
                .and_then(|a| a.iter().map(|v| v.as_str().map(str::to_owned)).collect())
                .ok_or_else(|| wrong("a list of strings"))
        };
        match key.as_str() {
            "on" => plugin.on = value.as_bool().ok_or_else(|| wrong("true or false"))?,
            "agents" => plugin.agents = strings(value)?,
            "kinds" => plugin.kinds = strings(value)?,
            "position" => {
                plugin.position = match value.as_str() {
                    Some("top") => Position::Top,
                    Some("bottom") => Position::Bottom,
                    _ => return Err(wrong("\"top\" or \"bottom\"")),
                }
            }
            "interval" => {
                plugin.interval = value
                    .as_integer()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| wrong("a whole number of seconds"))?
            }
            _ => {
                plugin.settings.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(())
}

/// The plugins: the built-in ones, then each folder in `<dir>/plugins` with a `plugin.toml`, in
/// name order, with `overrides` (`config.toml`'s `[plugins]`) applied. Also returns the
/// problems found: a folder whose manifest doesn't parse is left out, and so is an override that
/// doesn't, or that names no plugin.
pub fn load(dir: &Path, overrides: &BTreeMap<String, toml::Table>) -> (Vec<Plugin>, Vec<String>) {
    let mut plugins = built_in();
    let mut problems = Vec::new();
    let mut folders: Vec<PathBuf> = std::fs::read_dir(dir.join("plugins"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    folders.sort();
    for folder in folders {
        let Some(id) = folder
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
        else {
            continue;
        };
        match read(&folder, &id) {
            None => {}
            Some(Err(problem)) => problems.push(problem),
            Some(Ok(_)) if !valid_id(&id) => problems.push(format!(
                "{}: a plugin's folder name may only have letters, digits, - and _",
                folder.display()
            )),
            Some(Ok(_)) if plugins.iter().any(|p| p.id == id) => problems.push(format!(
                "{}: {id} is the name of a built-in plugin",
                folder.display()
            )),
            Some(Ok(plugin)) => plugins.push(plugin),
        }
    }
    for (id, table) in overrides {
        match plugins.iter_mut().find(|p| &p.id == id) {
            Some(plugin) => {
                if let Err(problem) = apply(plugin, table) {
                    problems.push(problem);
                }
            }
            None => problems.push(format!("plugins.{id}: there's no plugin called {id}")),
        }
    }
    for plugin in &mut plugins {
        plugin.interval = plugin.interval.clamp(MIN_INTERVAL, MAX_INTERVAL);
    }
    (plugins, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config folder for one test, emptied first.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("macaroni-plugins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        dir
    }

    /// Writes `<dir>/plugins/<id>/plugin.toml`.
    fn manifest(dir: &Path, id: &str, text: &str) {
        let folder = dir.join("plugins").join(id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("plugin.toml"), text).unwrap();
    }

    #[test]
    fn folders_become_plugins_after_the_built_in_ones() {
        let dir = scratch("load");
        manifest(
            &dir,
            "codex-cost",
            "name = \"Cost\"\ncommand = [\"python\", \"cost.py\"]\nposition = \"top\"\n\
             agents = [\"codex\"]\n[settings]\ncurrency = \"usd\"\n",
        );
        manifest(&dir, "clock", "command = \"date\"\ninterval = 0\n");
        manifest(&dir, "broken", "command = 3\n");
        manifest(&dir, "cache", "command = \"x\"\n");
        std::fs::create_dir_all(dir.join("plugins").join("notes")).unwrap();

        let (plugins, problems) = load(&dir, &BTreeMap::new());
        let ids: Vec<_> = plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["cache", "clock", "codex-cost"]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("broken")));
        assert!(problems.iter().any(|p| p.contains("built-in")));

        let clock = &plugins[1];
        assert_eq!(clock.run, Run::Shell("date".into()));
        assert_eq!(
            (clock.interval, clock.position),
            (MIN_INTERVAL, Position::Bottom)
        );
        assert!(clock.applies("terminal", None) && clock.applies("terminal", Some("claude")));
        assert!(!clock.applies("browser", None));

        let cost = &plugins[2];
        assert_eq!(cost.name, "Cost");
        assert_eq!(
            cost.run,
            Run::Program(vec!["python".into(), "cost.py".into()])
        );
        assert_eq!(
            cost.folder.as_deref(),
            Some(&*dir.join("plugins").join("codex-cost"))
        );
        assert!(cost.applies("terminal", Some("codex")));
        assert!(!cost.applies("terminal", Some("claude")) && !cost.applies("terminal", None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_overrides_and_settings() {
        let dir = scratch("overrides");
        let overrides: BTreeMap<String, toml::Table> = toml::from_str(
            "[cache]\nagents = [\"codex\"]\nminutes = 60\non = false\n\
             [nope]\non = true\n",
        )
        .unwrap();
        let (plugins, problems) = load(&dir, &overrides);
        let cache = &plugins[0];
        assert!(!cache.on);
        assert_eq!(cache.agents, ["codex"]);
        assert_eq!(
            cache.settings.get("minutes"),
            Some(&toml::Value::Integer(60))
        );
        assert_eq!(problems, ["plugins.nope: there's no plugin called nope"]);

        let bad: BTreeMap<String, toml::Table> =
            toml::from_str("[cache]\nposition = \"left\"\n").unwrap();
        let (plugins, problems) = load(&dir, &bad);
        assert_eq!(plugins[0].position, Position::Bottom);
        assert_eq!(problems.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
