//! Reading coding agents' setup from disk, for the agent setup tab (`view/agent_tab.rs`):
//! Claude Code's and Codex's instructions, settings, skills, subagents, commands, hooks and MCP
//! servers, the user's own (`~/.claude`, `~/.claude.json`, `~/.codex`) and a project's (its
//! `CLAUDE.md`, `AGENTS.md`, `.claude`, `.mcp.json` and `.codex`).
//!
//! [`scan`] is blocking file system work: run it off the UI thread. It never writes; a file
//! that can't be read or parsed is left out (or, for settings, listed without its hooks and
//! servers). What it finds is a flat list of [`Item`]s, each with the file to open and the text
//! to show.

use std::path::{Path, PathBuf};

/// The most of a file's text shown.
const MAX_BODY: usize = 64 * 1024;

/// How deep command folders are searched.
const MAX_COMMAND_DEPTH: usize = 3;

/// Which agent an item belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    /// Claude Code.
    Claude,
    /// OpenAI's Codex CLI.
    Codex,
}

impl Tool {
    /// Its name as shown.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Claude => "Claude Code",
            Tool::Codex => "Codex",
        }
    }
}

/// Whose an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The user's own, in the home folder: applies everywhere.
    Personal,
    /// The project's, in its folder: applies there, and is usually shared through the
    /// repository.
    Project,
}

impl Scope {
    /// Its name as shown.
    pub fn name(self) -> &'static str {
        match self {
            Scope::Personal => "Personal",
            Scope::Project => "Project",
        }
    }
}

/// What kind of setup an item is, in the order the tab lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    /// Instructions the agent always reads: `CLAUDE.md`, `AGENTS.md`.
    Instructions,
    /// Settings files.
    Settings,
    /// Skills: folders with a `SKILL.md`.
    Skills,
    /// Claude Code's subagents.
    Subagents,
    /// Slash commands (Codex's custom prompts).
    Commands,
    /// Hooks from Claude Code's settings.
    Hooks,
    /// MCP servers.
    Mcp,
}

impl Section {
    /// Every section, in order.
    pub const ALL: [Section; 7] = [
        Section::Instructions,
        Section::Settings,
        Section::Skills,
        Section::Subagents,
        Section::Commands,
        Section::Hooks,
        Section::Mcp,
    ];

    /// Its title.
    pub fn title(self) -> &'static str {
        match self {
            Section::Instructions => "Instructions",
            Section::Settings => "Settings",
            Section::Skills => "Skills",
            Section::Subagents => "Subagents",
            Section::Commands => "Commands",
            Section::Hooks => "Hooks",
            Section::Mcp => "MCP servers",
        }
    }
}

/// One thing in an agent's setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// What kind it is.
    pub section: Section,
    /// Whose agent.
    pub tool: Tool,
    /// The user's or the project's.
    pub scope: Scope,
    /// Its name: a file's, a skill's, `/command`, a hook's event and matcher, a server's.
    pub name: String,
    /// One line about it: a description, a hook's commands, a server's command or address.
    pub detail: String,
    /// The file it's in, opened to edit it.
    pub path: PathBuf,
    /// Whether that file exists; only instructions are listed without one, to create.
    pub exists: bool,
    /// The text shown: the file's (at most [`MAX_BODY`]), or a hook's or server's entry.
    pub body: String,
}

impl Item {
    /// What tells it apart from the others, to keep it selected across reloads.
    pub fn key(&self) -> String {
        format!("{:?}|{}|{}", self.section, self.path.display(), self.name)
    }
}

/// One of the agents' folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Whose agent.
    pub tool: Tool,
    /// The user's or the project's.
    pub scope: Scope,
    /// Where it is.
    pub path: PathBuf,
}

/// Everything found, for one tab.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Setup {
    /// The project's folder (the repository's top, or the folder itself), unless there's none
    /// or it's the home folder.
    pub project: Option<PathBuf>,
    /// The agents' folders that exist, personal ones first.
    pub folders: Vec<Folder>,
    /// Every item, in [`Section`] order, personal before project within each.
    pub items: Vec<Item>,
}

/// The project folder for a tab whose folder is `dir`: the repository holding it, else `dir`;
/// `None` for no folder or the home folder (whose setup is the personal one).
pub fn project_of(dir: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let dir = dir?;
    let top = crate::git::top(dir).unwrap_or_else(|| dir.to_path_buf());
    (Some(top.as_path()) != home).then_some(top)
}

/// Reads the setup in `home` (the user's) and `project`. Blocking.
pub fn scan(home: Option<&Path>, project: Option<&Path>) -> Setup {
    let mut setup = Setup {
        project: project.map(Path::to_path_buf),
        ..Setup::default()
    };
    let mut roots: Vec<(Scope, &Path)> = Vec::new();
    roots.extend(home.map(|h| (Scope::Personal, h)));
    roots.extend(project.map(|p| (Scope::Project, p)));
    let codex_used = home.is_some_and(|h| h.join(".codex").is_dir());
    for &(scope, root) in &roots {
        for (tool, name) in [(Tool::Claude, ".claude"), (Tool::Codex, ".codex")] {
            let path = root.join(name);
            if path.is_dir() {
                setup.folders.push(Folder { tool, scope, path });
            }
        }
    }
    let items = &mut setup.items;
    for &(scope, root) in &roots {
        let claude = root.join(".claude");
        let codex = root.join(".codex");
        // Instructions: the main file is listed even when missing, to create.
        match scope {
            Scope::Personal => {
                instructions(items, Tool::Claude, scope, claude.join("CLAUDE.md"), true);
                instructions(
                    items,
                    Tool::Codex,
                    scope,
                    codex.join("AGENTS.md"),
                    codex_used,
                );
            }
            Scope::Project => {
                instructions(items, Tool::Claude, scope, root.join("CLAUDE.md"), true);
                instructions(items, Tool::Claude, scope, claude.join("CLAUDE.md"), false);
                instructions(
                    items,
                    Tool::Claude,
                    scope,
                    root.join("CLAUDE.local.md"),
                    false,
                );
                instructions(
                    items,
                    Tool::Codex,
                    scope,
                    root.join("AGENTS.md"),
                    codex_used,
                );
            }
        }
        let mut settings = vec![claude.join("settings.json")];
        if scope == Scope::Project {
            settings.push(claude.join("settings.local.json"));
        }
        for path in settings {
            if let Some(text) = read(&path) {
                hooks(items, scope, &path, &text);
                items.push(file_item(
                    Section::Settings,
                    Tool::Claude,
                    scope,
                    &path,
                    text,
                ));
            }
        }
        let codex_config = codex.join("config.toml");
        if let Some(text) = read(&codex_config) {
            codex_servers(items, scope, &codex_config, &text);
            items.push(file_item(
                Section::Settings,
                Tool::Codex,
                scope,
                &codex_config,
                text,
            ));
        }
        skills(items, Tool::Claude, scope, &claude.join("skills"));
        skills(items, Tool::Codex, scope, &codex.join("skills"));
        markdown_dir(
            items,
            Section::Subagents,
            Tool::Claude,
            scope,
            &claude.join("agents"),
            "",
        );
        markdown_dir(
            items,
            Section::Commands,
            Tool::Claude,
            scope,
            &claude.join("commands"),
            "/",
        );
        markdown_dir(
            items,
            Section::Commands,
            Tool::Codex,
            scope,
            &codex.join("prompts"),
            "/prompts:",
        );
        if scope == Scope::Project {
            let mcp = root.join(".mcp.json");
            if let Some(text) = read(&mcp) {
                json_servers(items, Tool::Claude, scope, &mcp, &text, None);
            }
        }
    }
    // Claude Code keeps user servers, and each project's private ones, in `~/.claude.json`.
    if let Some(home) = home {
        let path = home.join(".claude.json");
        if let Some(text) = read(&path) {
            json_servers(items, Tool::Claude, Scope::Personal, &path, &text, None);
            if let Some(project) = project {
                json_servers(
                    items,
                    Tool::Claude,
                    Scope::Project,
                    &path,
                    &text,
                    Some(project),
                );
            }
        }
    }
    items.sort_by_key(|i| (i.section, i.scope == Scope::Project));
    setup
}

/// A file's whole text, if it can be read. Settings are parsed from all of it; what's shown
/// is cut by [`shown`].
fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// `text` as it's shown: at most [`MAX_BODY`] of it.
fn shown(mut text: String) -> String {
    if text.len() > MAX_BODY {
        let mut end = MAX_BODY;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n\n[\u{2026} the rest isn't shown]");
    }
    text
}

/// An item for a whole file.
fn file_item(section: Section, tool: Tool, scope: Scope, path: &Path, text: String) -> Item {
    Item {
        section,
        tool,
        scope,
        name: file_name(path),
        detail: short(&path.to_string_lossy()),
        path: path.to_path_buf(),
        exists: true,
        body: shown(text),
    }
}

/// A path's last part.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Adds an instructions file: when it exists, or (`always`) to create.
fn instructions(items: &mut Vec<Item>, tool: Tool, scope: Scope, path: PathBuf, always: bool) {
    match read(&path) {
        Some(text) => {
            let mut item = file_item(Section::Instructions, tool, scope, &path, text);
            item.detail = summary(&item.body).unwrap_or(item.detail);
            items.push(item);
        }
        None if always => items.push(Item {
            section: Section::Instructions,
            tool,
            scope,
            name: file_name(&path),
            detail: "Not created yet".to_owned(),
            path,
            exists: false,
            body: String::new(),
        }),
        None => {}
    }
}

/// Adds each skill in `dir`: a folder with a `SKILL.md`, named by its front matter.
fn skills(items: &mut Vec<Item>, tool: Tool, scope: Scope, dir: &Path) {
    for folder in sorted_entries(dir) {
        let path = folder.join("SKILL.md");
        let Some(text) = read(&path) else {
            continue;
        };
        let front = front_matter(&text);
        items.push(Item {
            section: Section::Skills,
            tool,
            scope,
            name: front
                .get("name")
                .cloned()
                .unwrap_or_else(|| file_name(&folder)),
            detail: front.get("description").cloned().unwrap_or_default(),
            path,
            exists: true,
            body: shown(text),
        });
    }
}

/// Adds each Markdown file in `dir` (and, for commands, its folders), named `prefix` and its
/// file name without `.md`; the description is the front matter's, else the first line.
fn markdown_dir(
    items: &mut Vec<Item>,
    section: Section,
    tool: Tool,
    scope: Scope,
    dir: &Path,
    prefix: &str,
) {
    let mut files = Vec::new();
    let depth = if section == Section::Commands {
        MAX_COMMAND_DEPTH
    } else {
        1
    };
    markdown_files(dir, depth, &mut files);
    for path in files {
        let Some(text) = read(&path) else {
            continue;
        };
        let front = front_matter(&text);
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = match section {
            Section::Subagents => front.get("name").cloned().unwrap_or(stem),
            _ => format!("{prefix}{stem}"),
        };
        let mut detail = front
            .get("description")
            .cloned()
            .or_else(|| summary(&text))
            .unwrap_or_default();
        // A command in a subfolder is shown with its folder, as Claude Code lists it.
        if let Some(folder) = path
            .parent()
            .filter(|p| *p != dir)
            .and_then(|p| p.strip_prefix(dir).ok())
        {
            detail = format!("({}) {detail}", folder.to_string_lossy().replace('\\', "/"));
        }
        items.push(Item {
            section,
            tool,
            scope,
            name,
            detail,
            path,
            exists: true,
            body: shown(text),
        });
    }
}

/// The `.md` files in `dir` and, `depth` levels down, its folders, sorted.
fn markdown_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let entries = sorted_entries(dir);
    for path in &entries {
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            && path.is_file()
        {
            out.push(path.clone());
        }
    }
    for path in entries.iter().filter(|p| p.is_dir()) {
        markdown_files(path, depth - 1, out);
    }
}

/// The entries of `dir`, sorted by name; none if it can't be read.
fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|read| read.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    entries
}

/// Adds each hook in a Claude Code settings file: one item per event and matcher.
fn hooks(items: &mut Vec<Item>, scope: Scope, path: &Path, text: &str) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let Some(events) = json.get("hooks").and_then(|h| h.as_object()) else {
        return;
    };
    for (event, groups) in events {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group
                .get("matcher")
                .and_then(|m| m.as_str())
                .filter(|m| !m.is_empty());
            let commands: Vec<&str> = group
                .get("hooks")
                .and_then(|h| h.as_array())
                .into_iter()
                .flatten()
                .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
                .collect();
            items.push(Item {
                section: Section::Hooks,
                tool: Tool::Claude,
                scope,
                name: match matcher {
                    Some(m) => format!("{event} \u{b7} {m}"),
                    None => event.clone(),
                },
                detail: commands.join("; "),
                path: path.to_path_buf(),
                exists: true,
                body: serde_json::to_string_pretty(group).unwrap_or_default(),
            });
        }
    }
}

/// Adds the MCP servers in a JSON file's `mcpServers`: at the top, or with `project`, in
/// `~/.claude.json`'s entry for that project.
fn json_servers(
    items: &mut Vec<Item>,
    tool: Tool,
    scope: Scope,
    path: &Path,
    text: &str,
    project: Option<&Path>,
) {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let holder = match project {
        None => Some(&json),
        Some(project) => json
            .get("projects")
            .and_then(|p| p.as_object())
            .and_then(|projects| {
                let wanted = same_path_key(project);
                projects
                    .iter()
                    .find(|(key, _)| same_path_key(Path::new(key)) == wanted)
                    .map(|(_, entry)| entry)
            }),
    };
    let Some(servers) = holder
        .and_then(|h| h.get("mcpServers"))
        .and_then(|s| s.as_object())
    else {
        return;
    };
    for (name, server) in servers {
        let detail = match server.get("url").and_then(|u| u.as_str()) {
            Some(url) => url.to_owned(),
            None => command_line(
                server.get("command").and_then(|c| c.as_str()),
                server
                    .get("args")
                    .and_then(|a| a.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.as_str()),
            ),
        };
        items.push(Item {
            section: Section::Mcp,
            tool,
            scope,
            name: name.clone(),
            detail,
            path: path.to_path_buf(),
            exists: true,
            body: serde_json::to_string_pretty(server).unwrap_or_default(),
        });
    }
}

/// Adds the MCP servers in a Codex `config.toml` (`[mcp_servers.<name>]`).
fn codex_servers(items: &mut Vec<Item>, scope: Scope, path: &Path, text: &str) {
    let Ok(config) = text.parse::<toml::Table>() else {
        return;
    };
    let Some(servers) = config.get("mcp_servers").and_then(|s| s.as_table()) else {
        return;
    };
    for (name, server) in servers {
        let detail = match server.get("url").and_then(|u| u.as_str()) {
            Some(url) => url.to_owned(),
            None => command_line(
                server.get("command").and_then(|c| c.as_str()),
                server
                    .get("args")
                    .and_then(|a| a.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|a| a.as_str()),
            ),
        };
        let mut entry = toml::Table::new();
        entry.insert(name.clone(), server.clone());
        let mut outer = toml::Table::new();
        outer.insert("mcp_servers".to_owned(), toml::Value::Table(entry));
        items.push(Item {
            section: Section::Mcp,
            tool: Tool::Codex,
            scope,
            name: name.clone(),
            detail,
            path: path.to_path_buf(),
            exists: true,
            body: toml::to_string(&outer).unwrap_or_default(),
        });
    }
}

/// A command and its arguments as one line.
fn command_line<'a>(command: Option<&'a str>, args: impl Iterator<Item = &'a str>) -> String {
    command
        .into_iter()
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A path as `~/.claude.json` keys its projects: forward slashes, no trailing one, and on
/// Windows any case.
fn same_path_key(path: &Path) -> String {
    let key = path
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

/// The `key: value` lines of a Markdown file's front matter (between `---` lines at its
/// start), quotes taken off.
fn front_matter(text: &str) -> std::collections::HashMap<String, String> {
    let mut found = std::collections::HashMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return found;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some((key, value)) = line.split_once(':')
            && !key.starts_with(char::is_whitespace)
        {
            let value = value.trim().trim_matches(['"', '\'']).trim();
            if !value.is_empty() {
                found.insert(key.trim().to_owned(), value.to_owned());
            }
        }
    }
    found
}

/// The first line of a Markdown file's text after any front matter that isn't blank, a
/// heading marker or a rule, shortened.
fn summary(text: &str) -> Option<String> {
    let mut lines = text.lines().peekable();
    if lines.peek().map(|l| l.trim()) == Some("---") {
        lines.next();
        for line in lines.by_ref() {
            if line.trim() == "---" {
                break;
            }
        }
    }
    lines
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty() && !l.chars().all(|c| c == '-' || c == '='))
        .map(short)
}

/// `text` cut to about a line.
fn short(text: &str) -> String {
    const MAX: usize = 120;
    match text.char_indices().nth(MAX) {
        Some((at, _)) => format!("{}\u{2026}", &text[..at]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder in the temp directory for one test, emptied; the test deletes it when it
    /// passes.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "macaroni-agent-setup-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes `text` to `root/relative`, making its folders.
    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn names(setup: &Setup, section: Section) -> Vec<(String, Scope, Tool)> {
        setup
            .items
            .iter()
            .filter(|i| i.section == section)
            .map(|i| (i.name.clone(), i.scope, i.tool))
            .collect()
    }

    #[test]
    fn finds_everything_personal_and_project() {
        let home = scratch("home");
        let project = scratch("project");
        write(&home, ".claude/CLAUDE.md", "# Me\n\nAlways use tabs.");
        write(
            &home,
            ".claude/settings.json",
            r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "macaroni-cli notify"}]}],
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "check.sh"}]}]}}"#,
        );
        write(
            &home,
            ".claude/skills/review/SKILL.md",
            "---\nname: code-review\ndescription: \"Reviews a diff\"\n---\nBody",
        );
        write(
            &home,
            ".claude/agents/tester.md",
            "---\nname: tester\ndescription: Runs tests\n---\n",
        );
        write(&home, ".claude/commands/ship.md", "Ship it.\n");
        write(
            &home,
            ".claude/commands/web/build.md",
            "---\ndescription: Build the site\n---\n",
        );
        // Longer than what's shown of a file, as a real one gets: it's still parsed whole.
        write(
            &home,
            ".claude.json",
            &format!(
                r#"{{"history": "{}", "mcpServers": {{"docs": {{"url": "https://example.com/mcp"}}}},
                    "projects": {{"{}": {{"mcpServers": {{"db": {{"command": "pg", "args": ["--ro"]}}}}}}}}}}"#,
                "x".repeat(MAX_BODY),
                project.to_string_lossy().replace('\\', "/")
            ),
        );
        write(
            &home,
            ".codex/config.toml",
            "[mcp_servers.fs]\ncommand = \"fs-mcp\"\nargs = [\"/\"]\n",
        );
        write(&home, ".codex/prompts/fix.md", "Fix the failing test.");
        write(&project, "CLAUDE.md", "Project rules");
        write(
            &project,
            ".mcp.json",
            r#"{"mcpServers": {"web": {"command": "npx", "args": ["web-mcp"]}}}"#,
        );

        let setup = scan(Some(&home), Some(&project));
        assert_eq!(
            setup
                .folders
                .iter()
                .map(|f| (f.tool, f.scope))
                .collect::<Vec<_>>(),
            [
                (Tool::Claude, Scope::Personal),
                (Tool::Codex, Scope::Personal)
            ]
        );
        let instructions = names(&setup, Section::Instructions);
        assert!(instructions.contains(&("CLAUDE.md".into(), Scope::Personal, Tool::Claude)));
        assert!(instructions.contains(&("CLAUDE.md".into(), Scope::Project, Tool::Claude)));
        // Codex is in use, so its instructions are offered, though neither file exists.
        let agents_md: Vec<_> = setup
            .items
            .iter()
            .filter(|i| i.name == "AGENTS.md")
            .map(|i| (i.scope, i.exists))
            .collect();
        assert_eq!(
            agents_md,
            [(Scope::Personal, false), (Scope::Project, false)]
        );
        let mine = setup
            .items
            .iter()
            .find(|i| i.section == Section::Instructions && i.scope == Scope::Personal)
            .unwrap();
        assert_eq!(mine.detail, "Me");

        assert_eq!(
            names(&setup, Section::Skills),
            [("code-review".into(), Scope::Personal, Tool::Claude)]
        );
        assert_eq!(names(&setup, Section::Subagents)[0].0, "tester");
        let commands = names(&setup, Section::Commands);
        assert_eq!(
            commands.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["/ship", "/build", "/prompts:fix"]
        );
        let build = setup.items.iter().find(|i| i.name == "/build").unwrap();
        assert_eq!(build.detail, "(web) Build the site");

        // In the file's order or sorted, depending on whether something in the build turns on
        // serde_json's `preserve_order` (the CLI does).
        let mut hooks: Vec<String> = names(&setup, Section::Hooks)
            .into_iter()
            .map(|h| h.0)
            .collect();
        hooks.sort();
        assert_eq!(hooks, ["PreToolUse \u{b7} Bash", "Stop"]);
        let stop = setup.items.iter().find(|i| i.name == "Stop").unwrap();
        assert_eq!(stop.detail, "macaroni-cli notify");

        let servers: Vec<_> = setup
            .items
            .iter()
            .filter(|i| i.section == Section::Mcp)
            .map(|i| (i.name.as_str(), i.scope, i.detail.as_str()))
            .collect();
        assert!(servers.contains(&("docs", Scope::Personal, "https://example.com/mcp")));
        assert!(servers.contains(&("fs", Scope::Personal, "fs-mcp /")));
        assert!(servers.contains(&("web", Scope::Project, "npx web-mcp")));
        assert!(servers.contains(&("db", Scope::Project, "pg --ro")));
        let fs = setup.items.iter().find(|i| i.name == "fs").unwrap();
        assert!(fs.body.contains("[mcp_servers.fs]"), "{}", fs.body);

        // Sections in order, personal before project within each.
        let order: Vec<_> = setup
            .items
            .iter()
            .map(|i| (i.section, i.scope == Scope::Project))
            .collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted);
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(project);
    }

    #[test]
    fn nothing_set_up_offers_the_main_instructions() {
        let home = scratch("empty-home");
        let setup = scan(Some(&home), None);
        assert!(setup.folders.is_empty());
        assert_eq!(
            names(&setup, Section::Instructions),
            [("CLAUDE.md".into(), Scope::Personal, Tool::Claude)]
        );
        assert!(!setup.items[0].exists);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_home_folder_is_no_project() {
        let home = scratch("home-as-project");
        assert_eq!(project_of(Some(&home), Some(&home)), None);
        let other = scratch("plain-folder");
        assert_eq!(project_of(Some(&other), Some(&home)), Some(other.clone()));
        assert_eq!(project_of(None, Some(&home)), None);
        let _ = std::fs::remove_dir_all(home);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn front_matter_and_summaries() {
        let front = front_matter("---\nname: x\ndescription: 'Does y'\n  nested: no\n---\nbody");
        assert_eq!(front.get("name").map(String::as_str), Some("x"));
        assert_eq!(front.get("description").map(String::as_str), Some("Does y"));
        assert!(!front.contains_key("nested"));
        assert_eq!(
            summary("---\na: b\n---\n\n## Title\nmore").as_deref(),
            Some("Title")
        );
        assert_eq!(summary("\n---\n"), None);
    }
}
