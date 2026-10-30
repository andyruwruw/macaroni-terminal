//! `macaroni-cli`: control a running macaroni from scripts, agent hooks or another terminal.
//!
//! Inside a macaroni terminal, `MACARONI_SOCKET` and `MACARONI_TAB_ID` are set, so commands find
//! the app and act on their own tab by default (`macaroni-cli notify "tests passed"`).
//!
//! Each subcommand ([`Cmd`]) maps to one control-API [`Call`] ([`call`]), sent over a
//! [`Client`] connection; the result is printed as a short summary or, with `--json`, raw.
//! `setup`, `hook` and `statusline` are the Claude Code integration in [`claude`] and are handled
//! before connecting. Errors print to stderr with exit code 1.

mod claude;
mod client;

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use client::Client;
use macaroni_ipc::{BrowseAction, Call, Direction, Page, Screen, TAB_ENV, Tree};

// Doc comments on clap types, fields and variants are also the `--help` text.

/// The command line: one subcommand plus the global `--json` flag.
#[derive(Parser)]
#[command(name = "macaroni-cli", version, about = "Control a running macaroni")]
struct Cli {
    /// What to do.
    #[command(subcommand)]
    command: Cmd,
    /// Print the raw JSON result instead of a readable summary.
    #[arg(long, global = true)]
    json: bool,
}

/// A side of a pane, as typed on the command line; mapped to [`Direction`].
#[derive(Clone, Copy, ValueEnum)]
enum Side {
    /// To the right.
    Right,
    /// Below.
    Down,
    /// To the left.
    Left,
    /// Above.
    Up,
}

/// The subcommands. Every `--tab` defaults to the tab the command runs in (`MACARONI_TAB_ID`),
/// else the app's focused tab.
#[derive(Subcommand)]
enum Cmd {
    /// Show workspaces, panes and tabs.
    List,
    /// Create a workspace (and switch to it).
    NewWorkspace {
        /// Name for the new workspace.
        #[arg(long)]
        name: Option<String>,
    },
    /// Switch to a workspace.
    SelectWorkspace {
        /// The workspace's id (see `list`).
        workspace: u64,
    },
    /// Rename a workspace.
    RenameWorkspace {
        /// The workspace's id (see `list`).
        workspace: u64,
        /// The new name.
        name: String,
    },
    /// Close a workspace (default: the active one).
    CloseWorkspace {
        /// The workspace's id (see `list`).
        workspace: Option<u64>,
    },
    /// Split the pane holding a tab; the new pane runs a new terminal.
    Split {
        /// Which side the new pane goes.
        #[arg(value_enum, default_value = "right")]
        direction: Side,
        /// The tab whose pane to split.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Open a terminal tab next to a tab.
    NewTab {
        /// The tab to open next to.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Close a tab.
    CloseTab {
        /// The tab to close.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Show a tab: switch to its workspace and pane.
    Focus {
        /// The tab's id (see `list`).
        tab: u64,
    },
    /// Type text into a tab's shell.
    Send {
        /// The text, sent as is.
        text: String,
        /// Press Enter after the text.
        #[arg(long)]
        enter: bool,
        /// The terminal tab to type into.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Print the visible text of a tab.
    Read {
        /// The terminal tab to read.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Ask for attention: marks the tab, pane and workspace, and sends an OS notification if
    /// the tab isn't in view.
    Notify {
        /// The message.
        body: String,
        /// The notification's heading.
        #[arg(long)]
        title: Option<String>,
        /// The terminal tab asking for attention.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Clear a tab's attention mark.
    Clear {
        /// The tab to clear.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Stream events (tabs and workspaces opening and closing, titles, notifications) as JSON
    /// lines until interrupted.
    Events,
    /// Drive browser tabs. From a terminal, these act on the browser in that terminal's
    /// workspace (`--tab` picks another tab).
    Browser {
        /// What to do in the browser.
        #[command(subcommand)]
        command: BrowserCmd,
    },
    /// Connect a coding agent to macaroni (adds hooks and a status line to its settings).
    Setup {
        /// The agent to set up.
        #[arg(value_enum)]
        agent: Agent,
        /// Undo the setup.
        #[arg(long)]
        remove: bool,
        /// Don't ask before writing.
        #[arg(long)]
        yes: bool,
        /// Settings file to change (default: ~/.claude/settings.json).
        #[arg(long)]
        settings: Option<PathBuf>,
    },
    /// Run by Claude Code hooks: reads the hook's JSON on stdin and reports the agent's state.
    #[command(hide = true)]
    Hook,
    /// Run by Claude Code as its status line: prints model, context and cost, and shows them in
    /// macaroni's pane header.
    #[command(hide = true)]
    Statusline,
}

/// `browser` subcommands. `--tab` may be a browser tab or a terminal tab (meaning the browser
/// in its workspace).
#[derive(Subcommand)]
enum BrowserCmd {
    /// Open a page (default: the home page). Without --split, it opens in the workspace's
    /// browser pane, or in a new pane to the right.
    Open {
        /// An address or search words.
        url: Option<String>,
        /// Open in a new pane on this side.
        #[arg(long, value_enum)]
        split: Option<Side>,
        /// The tab to open from.
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Load an address (or search for words).
    Navigate {
        /// An address or search words.
        url: String,
        /// The browser (or a terminal in its workspace).
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Go back a page.
    Back {
        /// The browser (or a terminal in its workspace).
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Go forward a page.
    Forward {
        /// The browser (or a terminal in its workspace).
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Reload the page.
    Reload {
        /// The browser (or a terminal in its workspace).
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Open Chromium DevTools for the page.
    Devtools {
        /// The browser (or a terminal in its workspace).
        #[arg(long)]
        tab: Option<u64>,
    },
    /// Print the page's visible text (--json adds its address and title).
    Read {
        /// The browser (or a terminal in its workspace).
        #[arg(long)]
        tab: Option<u64>,
    },
}

/// Coding agents `setup` knows.
#[derive(Clone, Copy, ValueEnum)]
enum Agent {
    /// Claude Code.
    Claude,
}

/// An explicit `--tab` wins; inside a macaroni terminal the default is that terminal's tab.
/// `None` (outside macaroni, or an unparsable `MACARONI_TAB_ID`) lets the app use its focused
/// tab.
fn own_tab(tab: Option<u64>) -> Option<u64> {
    tab.or_else(|| std::env::var(TAB_ENV).ok()?.parse().ok())
}

/// The protocol's name for a [`Side`].
fn direction(side: Side) -> Direction {
    match side {
        Side::Right => Direction::Right,
        Side::Down => Direction::Down,
        Side::Left => Direction::Left,
        Side::Up => Direction::Up,
    }
}

/// The call for a `browser` subcommand.
fn browser_call(cmd: &BrowserCmd) -> Call {
    let browse = |tab: &Option<u64>, action| Call::Browse {
        tab: own_tab(*tab),
        action,
    };
    match cmd {
        BrowserCmd::Open { url, split, tab } => Call::OpenBrowser {
            tab: own_tab(*tab),
            url: url.clone(),
            split: split.map(direction),
        },
        BrowserCmd::Navigate { url, tab } => Call::Navigate {
            tab: own_tab(*tab),
            url: url.clone(),
        },
        BrowserCmd::Back { tab } => browse(tab, BrowseAction::Back),
        BrowserCmd::Forward { tab } => browse(tab, BrowseAction::Forward),
        BrowserCmd::Reload { tab } => browse(tab, BrowseAction::Reload),
        BrowserCmd::Devtools { tab } => browse(tab, BrowseAction::Devtools),
        BrowserCmd::Read { tab } => Call::ReadPage { tab: own_tab(*tab) },
    }
}

/// The control-API call for a subcommand, with `--tab` defaults filled in.
///
/// # Panics
///
/// For `setup`, `hook` and `statusline`, which don't talk to the socket this way; [`run`]
/// handles them before calling this.
fn call(cmd: &Cmd) -> Call {
    match cmd {
        Cmd::List => Call::List,
        Cmd::NewWorkspace { name } => Call::NewWorkspace { name: name.clone() },
        Cmd::SelectWorkspace { workspace } => Call::SelectWorkspace {
            workspace: *workspace,
        },
        Cmd::RenameWorkspace { workspace, name } => Call::RenameWorkspace {
            workspace: *workspace,
            name: name.clone(),
        },
        Cmd::CloseWorkspace { workspace } => Call::CloseWorkspace {
            workspace: *workspace,
        },
        Cmd::Split {
            direction: side,
            tab,
        } => Call::Split {
            tab: own_tab(*tab),
            direction: direction(*side),
        },
        Cmd::NewTab { tab } => Call::NewTab { tab: own_tab(*tab) },
        Cmd::CloseTab { tab } => Call::CloseTab { tab: own_tab(*tab) },
        Cmd::Focus { tab } => Call::Focus { tab: *tab },
        Cmd::Send { text, enter, tab } => Call::SendText {
            tab: own_tab(*tab),
            text: if *enter {
                format!("{text}\r")
            } else {
                text.clone()
            },
        },
        Cmd::Read { tab } => Call::ReadScreen { tab: own_tab(*tab) },
        Cmd::Notify { body, title, tab } => Call::Notify {
            tab: own_tab(*tab),
            title: title.clone(),
            body: body.clone(),
        },
        Cmd::Clear { tab } => Call::ClearAttention { tab: own_tab(*tab) },
        Cmd::Events => Call::Subscribe,
        Cmd::Browser { command } => browser_call(command),
        Cmd::Setup { .. } | Cmd::Hook | Cmd::Statusline => {
            unreachable!("handled before connecting")
        }
    }
}

/// Prints `list`'s result as an indented outline: `*` marks the active workspace, `>` each
/// pane's selected tab, with attention and agent state in brackets.
fn print_tree(tree: &Tree) {
    for ws in &tree.workspaces {
        let marker = if ws.active { "*" } else { " " };
        println!("{marker} workspace {}  {}", ws.id, ws.name);
        for pane in &ws.panes {
            let focus = if pane.focused { " (focused)" } else { "" };
            println!("    pane {}{focus}", pane.id);
            for tab in &pane.tabs {
                let active = if tab.active { ">" } else { " " };
                let title = match (&tab.title, &tab.url) {
                    (Some(title), _) => title.as_str(),
                    (None, Some(url)) => url.as_str(),
                    (None, None) => "Terminal",
                };
                let kind = match tab.kind {
                    macaroni_ipc::TabKind::Terminal => "",
                    macaroni_ipc::TabKind::Browser => "browser  ",
                };
                let attention = tab
                    .attention
                    .as_deref()
                    .map(|a| format!("  [attention: {a}]"))
                    .unwrap_or_default();
                let agent = tab
                    .agent
                    .as_ref()
                    .map(|a| {
                        // The wire name (`needs_input`), which is what scripts would match on.
                        let state = serde_json::to_value(a.state)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_default();
                        match &a.detail {
                            Some(detail) => format!("  [agent: {state}, {detail}]"),
                            None => format!("  [agent: {state}]"),
                        }
                    })
                    .unwrap_or_default();
                println!(
                    "      {active} tab {}  {kind}{title}{attention}{agent}",
                    tab.id
                );
            }
        }
    }
}

/// Runs one command: the Claude Code ones locally, everything else as a call to the app, then
/// prints the result. `events` keeps reading and printing event lines until the app goes away
/// or stdout closes. Errors are messages for stderr.
fn run(cli: &Cli) -> Result<(), String> {
    match &cli.command {
        Cmd::Hook => {
            claude::hook();
            return Ok(());
        }
        Cmd::Statusline => {
            claude::statusline();
            return Ok(());
        }
        Cmd::Setup {
            agent: Agent::Claude,
            remove,
            yes,
            settings,
        } => return claude::setup(*remove, *yes, settings.clone()),
        _ => {}
    }

    let mut client = Client::connect()?;
    let result = client.call(call(&cli.command))?;

    match &cli.command {
        Cmd::Events => {
            let stdout = std::io::stdout();
            while let Some(event) = client.next_event().map_err(|e| e.to_string())? {
                let mut out = stdout.lock();
                // Flush per event so a pipe reader sees each line as it happens; a write error
                // means the reader went away (`| head`), which ends the stream quietly.
                if writeln!(out, "{event}").and_then(|()| out.flush()).is_err() {
                    break;
                }
            }
        }
        _ if cli.json => println!("{result}"),
        Cmd::List => {
            let tree: Tree = serde_json::from_value(result).map_err(|e| e.to_string())?;
            print_tree(&tree);
        }
        Cmd::Read { .. } => {
            let screen: Screen = serde_json::from_value(result).map_err(|e| e.to_string())?;
            println!("{}", screen.text);
        }
        Cmd::Browser {
            command: BrowserCmd::Read { .. },
        } => {
            let page: Page = serde_json::from_value(result).map_err(|e| e.to_string())?;
            println!("{}", page.text);
        }
        // Commands that create or show a tab print its id, so scripts can capture it.
        Cmd::NewWorkspace { .. }
        | Cmd::Split { .. }
        | Cmd::NewTab { .. }
        | Cmd::Focus { .. }
        | Cmd::Browser {
            command: BrowserCmd::Open { .. },
        } => {
            if let Some(tab) = result.get("tab") {
                println!("{tab}");
            }
        }
        _ => {}
    }
    Ok(())
}

/// Parses the command line and runs it; exit code 1 with `macaroni-cli: <error>` on stderr on
/// failure.
fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("macaroni-cli: {err}");
            ExitCode::FAILURE
        }
    }
}
