//! Running panel plugins ([`macaroni_config::Plugin`]) and holding the rows they print.
//!
//! Every [`TICK`] while a shown panel has a plugin on, [`App::plugin_tick`] runs each of its
//! plugins whose interval has passed, or whose input changed (the agent moved on, the folder
//! changed), unless a run is still going. A run is a child process off the UI thread: the
//! panel's [`input`] as JSON on stdin, `MACARONI_TAB_ID`, `MACARONI_SOCKET` and
//! `MACARONI_PLUGIN` in its environment, its folder as the working directory. Each line it
//! prints within [`TIMEOUT`] is a row ([`parse_row`] reads its ANSI colors), up to [`MAX_ROWS`];
//! a failure shows as one row saying why. Hidden panels keep their last rows and catch up when
//! shown.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use iced::Task;
use macaroni_config::{Plugin, Run};
use macaroni_core::{Tab, TabId, TabKind};
use macaroni_term::Rgb;
use serde_json::{Value, json};

use crate::App;
use crate::message::Message;

/// How often shown panels' plugins are checked.
pub const TICK: Duration = Duration::from_secs(1);

/// How long a run may take before it's stopped.
const TIMEOUT: Duration = Duration::from_secs(10);

/// The most rows one plugin adds to a panel.
pub const MAX_ROWS: usize = 4;

/// The most characters of a row that are kept.
const MAX_ROW_CHARS: usize = 400;

/// The most output read from a run, so a runaway one can't fill memory.
const MAX_OUTPUT: u64 = 64 * 1024;

/// One plugin's state in one panel.
#[derive(Default)]
pub struct Rows {
    /// What it showed last.
    pub rows: Vec<Row>,
    /// Whether a run is going.
    running: bool,
    /// When the last run started.
    started: Option<Instant>,
    /// The input the last run got.
    input: String,
}

/// Every panel's plugin rows, by tab and plugin id.
#[derive(Default)]
pub struct PluginRows(HashMap<(TabId, String), Rows>);

impl PluginRows {
    /// What `plugin` shows in `tab`, if it has run there.
    pub fn get(&self, tab: TabId, plugin: &str) -> Option<&Rows> {
        self.0.get(&(tab, plugin.to_owned()))
    }

    /// Makes every plugin run again on the next tick (the config changed: a plugin's command or
    /// settings may have).
    pub fn stale(&mut self) {
        for rows in self.0.values_mut() {
            rows.input.clear();
        }
    }
}

/// A row of text in runs of one style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row(pub Vec<Span>);

/// A run of text in one style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    /// The text.
    pub text: String,
    /// Its color; `None` is the rows' default.
    pub color: Option<SpanColor>,
    /// Bold.
    pub bold: bool,
    /// Dimmed.
    pub dim: bool,
    /// Italic.
    pub italic: bool,
    /// Underlined.
    pub underline: bool,
}

/// A span's color, as the program gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanColor {
    /// One of the 256 palette colors (0 to 15 are the theme's ANSI colors).
    Indexed(u8),
    /// A direct color.
    Rgb(Rgb),
}

/// Applies one SGR sequence's parameters to `style`.
fn sgr(style: &mut Span, params: &str) {
    let mut codes = params
        .split([';', ':'])
        .map(|p| p.parse::<u16>().unwrap_or(0))
        .peekable();
    if codes.peek().is_none() {
        *style = Span::default();
    }
    while let Some(code) = codes.next() {
        match code {
            0 => *style = Span::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            22 => (style.bold, style.dim) = (false, false),
            23 => style.italic = false,
            24 => style.underline = false,
            30..=37 => style.color = Some(SpanColor::Indexed((code - 30) as u8)),
            90..=97 => style.color = Some(SpanColor::Indexed((code - 90 + 8) as u8)),
            39 => style.color = None,
            38 => match codes.next() {
                Some(5) => {
                    style.color = codes.next().map(|i| SpanColor::Indexed(i.min(255) as u8));
                }
                Some(2) => {
                    let mut c = || codes.next().unwrap_or(0).min(255) as u8;
                    style.color = Some(SpanColor::Rgb(Rgb::new(c(), c(), c())));
                }
                _ => {}
            },
            // Backgrounds aren't drawn; their extended forms are skipped whole.
            48 => match codes.next() {
                Some(5) => {
                    codes.next();
                }
                Some(2) => {
                    codes.nth(2);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// One line of a plugin's output as styled spans: SGR sequences (`ESC [ ... m`) set the style,
/// other escape sequences (OSCs whole) and control characters are dropped, and the text is cut at
/// [`MAX_ROW_CHARS`].
pub fn parse_row(line: &str) -> Row {
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Span::default();
    let mut text = String::new();
    let mut kept = 0;
    let mut chars = line.chars().peekable();
    let flush = |spans: &mut Vec<Span>, text: &mut String, style: &Span| {
        if !text.is_empty() {
            spans.push(Span {
                text: std::mem::take(text),
                ..style.clone()
            });
        }
    };
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {}
                // An OSC (a title, a link) runs to BEL or ESC \; none of it is text.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                    continue;
                }
                _ => continue,
            }
            let mut params = String::new();
            let mut last = None;
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    last = Some(c);
                    break;
                }
                params.push(c);
            }
            if last == Some('m') {
                flush(&mut spans, &mut text, &style);
                sgr(&mut style, &params);
            }
        } else if c == '\t' {
            text.push_str("    ");
        } else if !c.is_control() && kept < MAX_ROW_CHARS {
            text.push(c);
            kept += 1;
        }
    }
    flush(&mut spans, &mut text, &style);
    Row(spans)
}

/// A tab kind's name in plugin input and in a plugin's `kinds`.
pub fn kind_name(kind: &TabKind) -> &'static str {
    match kind {
        TabKind::Terminal(_) => "terminal",
        TabKind::Browser(_) => "browser",
        TabKind::Git => "git",
        TabKind::Tmux => "tmux",
        TabKind::AgentSetup => "agent_setup",
        TabKind::File => "file",
    }
}

/// Whether `plugin` is on in `tab`: the tab's own choice, else whether it's on by default and
/// applies to the tab.
pub fn is_on(tab: &Tab, plugin: &Plugin) -> bool {
    tab.plugins.get(&plugin.id).copied().unwrap_or_else(|| {
        plugin.on
            && plugin.applies(
                kind_name(&tab.kind),
                tab.agent.as_ref().map(|a| a.kind.name()),
            )
    })
}

/// A plugin's settings as JSON.
fn settings(plugin: &Plugin) -> Value {
    serde_json::to_value(&plugin.settings).unwrap_or(Value::Null)
}

/// What a plugin is told about `tab`: the plugin, the tab, its agent, and (as Claude Code's
/// status line input has them) `session_id`, `transcript_path` and `cwd` at the top level.
pub fn input(tab: &Tab, workspace: Option<&str>, plugin: &Plugin) -> Value {
    let agent = tab.agent.as_ref();
    let cwd = agent
        .and_then(|a| a.cwd.clone())
        .or_else(|| tab.cwd.clone());
    json!({
        "macaroni": 1,
        "plugin": {
            "id": plugin.id,
            "settings": settings(plugin),
        },
        "tab": {
            "id": tab.id.0,
            "kind": kind_name(&tab.kind),
            "title": tab.label(),
            "cwd": tab.cwd,
            "url": tab.url,
            "file": tab.file,
            "workspace": workspace,
        },
        "agent": agent.map(|a| json!({
            "kind": a.kind.name(),
            "state": match a.state {
                macaroni_core::AgentState::Working => "working",
                macaroni_core::AgentState::NeedsInput => "needs_input",
                macaroni_core::AgentState::Idle => "idle",
            },
            "detail": a.detail,
            "summary": a.summary,
            "session_id": a.session_id,
            "transcript_path": a.transcript,
            "cwd": a.cwd,
        })),
        "session_id": agent.and_then(|a| a.session_id.clone()),
        "transcript_path": agent.and_then(|a| a.transcript.clone()),
        "cwd": cwd,
    })
}

/// `macaroni-cli` next to this executable.
fn cli() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let cli = exe.with_file_name(format!("macaroni-cli{}", std::env::consts::EXE_SUFFIX));
    cli.is_file().then_some(cli)
}

/// The process for `plugin`, in its folder, without a console window.
fn command(plugin: &Plugin) -> Result<Command, String> {
    let mut command = match &plugin.run {
        Run::Shell(line) => {
            let mut command;
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command = Command::new("cmd");
                // As typed: `cmd` has its own quoting, which Rust's escaping would break.
                command.arg("/C").raw_arg(line);
            }
            #[cfg(not(windows))]
            {
                command = Command::new("sh");
                command.arg("-c").arg(line);
            }
            command
        }
        Run::Program(words) => {
            let (program, args) = words.split_first().ok_or("its command is empty")?;
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        Run::Cli(args) => {
            let mut command =
                Command::new(cli().ok_or("macaroni-cli wasn't found next to macaroni")?);
            command.args(args);
            command
        }
    };
    if let Some(folder) = &plugin.folder {
        command.current_dir(folder);
    }
    crate::vscode::hide_window(&mut command);
    Ok(command)
}

/// Runs `command` with `input` on stdin and returns its rows, or why it failed. Blocks for up to
/// [`TIMEOUT`], so it runs off the UI thread.
fn run(mut command: Command, input: String) -> Result<Vec<Row>, String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("didn't start: {e}"))?;
    // Written from a thread of its own: a plugin that prints before reading its input could
    // otherwise fill one pipe while this waits on the other.
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(pipe) = pipe {
                let _ = pipe.take(MAX_OUTPUT).read_to_end(&mut out);
            }
            String::from_utf8_lossy(&out).into_owned()
        })
    };
    let stdout = read(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = read(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("took longer than {}s", TIMEOUT.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    if !status.success() {
        let stderr = stderr.join().unwrap_or_default();
        let why = stderr.lines().map(str::trim).find(|l| !l.is_empty());
        return Err(match (why, status.code()) {
            (Some(why), _) => why.to_owned(),
            (None, Some(code)) => format!("exited with {code}"),
            (None, None) => "was stopped".to_owned(),
        });
    }
    Ok(stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(MAX_ROWS)
        .map(parse_row)
        .collect())
}

impl App {
    /// Whether any panel on screen has a plugin on, which is when [`TICK`]s are needed.
    pub(crate) fn plugins_shown(&self) -> bool {
        self.shown_tabs().into_iter().any(|tab| {
            self.model
                .tab(tab)
                .is_some_and(|t| self.config.plugins.iter().any(|p| is_on(t, p)))
        })
    }

    /// The active tab of every pane on screen, in every window.
    fn shown_tabs(&self) -> Vec<TabId> {
        self.model
            .all_workspaces()
            .iter()
            .flat_map(|ws| ws.shown_panes(macaroni_core::Rect::UNIT))
            .filter_map(|(pane, _)| self.model.pane(pane).map(|p| p.active))
            .filter(|tab| self.model.tab_visible(*tab))
            .collect()
    }

    /// The plugins on in `tab`, in config order, with what they show there.
    pub(crate) fn tab_plugins(&self, tab: &Tab) -> Vec<(&Plugin, Option<&Rows>)> {
        self.config
            .plugins
            .iter()
            .filter(|p| is_on(tab, p))
            .map(|p| (p, self.plugin_rows.get(tab.id, &p.id)))
            .collect()
    }

    /// Starts each due run in the panels on screen, and forgets closed tabs' rows.
    pub(crate) fn plugin_tick(&mut self) -> Task<Message> {
        let endpoint = self.ipc.endpoint.clone();
        self.plugin_rows
            .0
            .retain(|(tab, _), _| self.model.tab(*tab).is_some());
        let mut tasks = Vec::new();
        for id in self.shown_tabs() {
            let Some(tab) = self.model.tab(id) else {
                continue;
            };
            let workspace = self
                .workspace_of_tab(id)
                .and_then(|w| self.model.workspace(w))
                .map(|w| w.name.clone());
            for plugin in self.config.plugins.iter().filter(|p| is_on(tab, p)) {
                let input = input(tab, workspace.as_deref(), plugin).to_string();
                let state = self
                    .plugin_rows
                    .0
                    .entry((id, plugin.id.clone()))
                    .or_default();
                let due = state
                    .started
                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(u64::from(plugin.interval)));
                if state.running || (!due && state.input == input) {
                    continue;
                }
                state.running = true;
                state.started = Some(Instant::now());
                state.input.clone_from(&input);
                let plugin_id = plugin.id.clone();
                let command = command(plugin).map(|mut c| {
                    c.env(macaroni_ipc::TAB_ENV, id.0.to_string())
                        .env("MACARONI_PLUGIN", &plugin.id);
                    if let Some(endpoint) = &endpoint {
                        c.env(macaroni_ipc::SOCKET_ENV, endpoint);
                    }
                    c
                });
                tasks.push(Task::perform(
                    crate::vscode::off_thread(move || command.and_then(|c| run(c, input))),
                    move |result| Message::PluginRan(id, plugin_id.clone(), result),
                ));
            }
        }
        Task::batch(tasks)
    }

    /// A run finished: its rows replace the old ones, or one row says why it failed.
    pub(crate) fn plugin_ran(
        &mut self,
        tab: TabId,
        plugin: String,
        result: Result<Vec<Row>, String>,
    ) -> Task<Message> {
        let name = self
            .config
            .plugins
            .iter()
            .find(|p| p.id == plugin)
            .map_or_else(|| plugin.clone(), |p| p.name.clone());
        if let Some(state) = self.plugin_rows.0.get_mut(&(tab, plugin)) {
            state.running = false;
            state.rows = result.unwrap_or_else(|why| {
                vec![Row(vec![Span {
                    text: format!("{name}: {why}"),
                    color: Some(SpanColor::Indexed(1)),
                    ..Span::default()
                }])]
            });
        }
        Task::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text and color of each span.
    fn runs(row: &Row) -> Vec<(&str, Option<SpanColor>)> {
        row.0.iter().map(|s| (s.text.as_str(), s.color)).collect()
    }

    #[test]
    fn rows_keep_ansi_colors_and_drop_other_escapes() {
        let row = parse_row("\x1b[32m\u{25cf}\x1b[0m cache warm \x1b[2m\u{b7} 4m left\x1b[0m");
        assert_eq!(
            runs(&row),
            [
                ("\u{25cf}", Some(SpanColor::Indexed(2))),
                (" cache warm ", None),
                ("\u{b7} 4m left", None),
            ]
        );
        assert!(row.0[2].dim && !row.0[1].dim);

        let row = parse_row(
            "\x1b[1;38;5;208mhot\x1b[22;38;2;1;2;3m rgb\x1b[39;48;5;1m plain\x1b]0;title\x07!",
        );
        assert_eq!(
            runs(&row),
            [
                ("hot", Some(SpanColor::Indexed(208))),
                (" rgb", Some(SpanColor::Rgb(Rgb::new(1, 2, 3)))),
                (" plain!", None),
            ]
        );
        assert!(row.0[0].bold && !row.0[1].bold);
        assert_eq!(
            runs(&parse_row("\x1b[95mx\x1b[mY")),
            [("x", Some(SpanColor::Indexed(13))), ("Y", None)]
        );
        assert_eq!(parse_row(&"x".repeat(1000)).0[0].text.len(), MAX_ROW_CHARS);
    }

    #[test]
    fn plugins_apply_by_agent_unless_the_tab_says_otherwise() {
        let (mut model, _) = macaroni_core::Model::new();
        let id = model.focused_tab().id;
        let cache = &macaroni_config::Config::default().plugins[0];
        assert!(!is_on(model.tab(id).unwrap(), cache), "no agent, no cache");
        model.apply(macaroni_core::Command::SetAgent {
            tab: id,
            kind: macaroni_core::AgentKind::Codex,
            state: macaroni_core::AgentState::Idle,
            detail: None,
            session_id: Some("s".into()),
            cwd: Some("C:/repo".into()),
            transcript: Some("C:/t.jsonl".into()),
        });
        let tab = model.tab(id).unwrap();
        assert!(is_on(tab, cache));
        let input = input(tab, Some("Main"), cache);
        assert_eq!(input["agent"]["kind"], "codex");
        assert_eq!(input["transcript_path"], "C:/t.jsonl");
        assert_eq!(input["cwd"], "C:/repo");
        assert_eq!(input["tab"]["workspace"], "Main");
        model.apply(macaroni_core::Command::SetTabPlugin(
            id,
            "cache".into(),
            Some(false),
        ));
        assert!(!is_on(model.tab(id).unwrap(), cache));
    }

    /// A plugin running `line` in the shell.
    fn shell(line: &str) -> Command {
        command(&Plugin {
            run: Run::Shell(line.to_owned()),
            ..macaroni_config::Config::default().plugins[0].clone()
        })
        .unwrap()
    }

    #[cfg(not(windows))]
    #[test]
    fn runs_print_rows_or_say_why_they_failed() {
        let rows = run(shell("printf 'one\\n\\ntwo\\n'"), "{}".into()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].0[0].text, "two");
        assert_eq!(
            run(shell("echo nope >&2; exit 3"), String::new()),
            Err("nope".into())
        );
        assert_eq!(
            run(shell("exit 4"), String::new()),
            Err("exited with 4".into())
        );
        // Reads its input.
        let rows = run(shell("grep macaroni"), "{\"macaroni\": 1}\n".into()).unwrap();
        assert_eq!(rows[0].0[0].text, "{\"macaroni\": 1}");
    }

    #[cfg(windows)]
    #[test]
    fn runs_print_rows_or_say_why_they_failed() {
        let rows = run(shell("echo one& echo.& echo two"), "{}".into()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].0[0].text, "two");
        assert_eq!(
            run(shell("echo nope 1>&2& exit 3"), String::new()),
            Err("nope".into())
        );
        assert_eq!(
            run(shell("exit 4"), String::new()),
            Err("exited with 4".into())
        );
        // Reads its input.
        let rows = run(shell("findstr macaroni"), "{\"macaroni\": 1}\n".into()).unwrap();
        assert_eq!(rows[0].0[0].text, "{\"macaroni\": 1}");
    }
}
