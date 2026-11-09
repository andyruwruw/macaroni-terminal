# Panel plugins

## Mental model

A plugin is a program that adds rows to the top or bottom of the panels (tabs) it's on in. macaroni runs it every so often with the panel's details as JSON on stdin; each line it prints is a row, ANSI colors and all. Nothing else: plugins can't draw anything but text rows, and they see only what the input gives them (plus whatever they read themselves, like an agent's transcript).

```
config dir/plugins/<id>/plugin.toml ─┐
built_in() ("cache") ────────────────┼─► plugins::load + [plugins.<id>] ─► Config.plugins
                                     │
PluginTick (1 s, only while a shown panel has one on)
  └─► for each shown tab × plugin on there: due (interval) or input changed, and not running?
        └─► child process off the UI thread: stdin = input JSON ─► stdout lines ─► PluginRan
              └─► PluginRows[(tab, id)].rows ─► drawn above/below the tab's body
```

## Key files

- `crates/config/src/plugins.rs`: `Plugin`, `Run` (`Shell`, `Program`, `Cli`), `Position`, the `plugin.toml` `Manifest`, `built_in()`, `load` (folders in name order after the built-ins, `[plugins.<id>]` applied, problems collected), `Plugin::applies`.
- `crates/app/src/plugins.rs`: `is_on` (the tab's choice, else `on && applies`), `input` (the JSON), `command`/`run` (spawn, stdin, timeout, rows), `parse_row` (SGR to `Span`s), `App::plugin_tick` / `plugin_ran` / `tab_plugins` / `plugins_shown`, `PluginRows`.
- `crates/app/src/view/panes.rs`: `with_plugin_rows`, `plugin_row` (terminal font and palette).
- `crates/app/src/view/menus.rs`: `tab_plugin_entries` (the tab menu's Plugins page, `MenuPage::Plugins`), sending `Message::TabPlugin`.
- `crates/core`: `Tab.plugins` (the tab's on/off choices by id), `Command::SetTabPlugin`; saved as `SavedTab.plugins` and put back in `restore.rs`.
- `crates/cli/src/plugins.rs`: the built-in plugins' code, run as `macaroni-cli plugin <name>`.

## The contract

- Input (stdin, one JSON object): `macaroni` (1), `plugin` (`id`, `settings`), `tab` (`id`, `kind`, `title`, `cwd`, `url`, `file`, `workspace`), `agent` (`kind`, `state`, `detail`, `summary`, `session_id`, `transcript_path`, `cwd`, or `null`), and Claude Code status-line style `session_id`, `transcript_path`, `cwd` at the top level. Agent fields come from the hooks (see [`claude-code.md`](claude-code.md)).
- Environment: `MACARONI_TAB_ID`, `MACARONI_SOCKET` (so a plugin can call `macaroni-cli`), `MACARONI_PLUGIN`. Working directory: the plugin's folder.
- Output: non-blank lines, at most `MAX_ROWS` (4), each cut to `MAX_ROW_CHARS`. SGR colors, bold, dim, italic and underline show; backgrounds, other escapes and OSCs are dropped. Printing nothing means no rows. A non-zero exit, a failed start or a run longer than `TIMEOUT` (10 s, then killed) shows one red row: the name and stderr's first line (or the exit code).
- `plugin.toml`: `command` (a string for `cmd /C` or `sh -c`, or a list run directly), `name`, `description`, `position` (`top`/`bottom`), `interval` (seconds, clamped to 1..86400), `agents` (`claude`, `codex`, `any`; empty means no agent needed), `kinds` (tab kinds or `any`; default `terminal`), `on`, `[settings]`.
- `config.toml` `[plugins.<id>]`: `on`, `agents`, `kinds`, `position`, `interval` override; any other key merges into `settings`. Problems (a manifest that doesn't parse, a folder name that isn't `[A-Za-z0-9_-]`, a built-in's name, an unknown id, a wrong type) leave that part out and show in the config banner (`Config.plugin_problems`), never failing the config.

## Invariants

- Runs happen only for panels on screen (`shown_tabs`); hidden ones keep their last rows and catch up when shown, since their last start is past the interval. No subscription runs while no shown panel has a plugin on.
- One run per (tab, plugin) at a time. A run starts when the interval has passed since the last start, or at once when its input changed (an agent changed state or session), so rows follow the agent within a tick.
- A config reload marks every run stale (`PluginRows::stale`), so changed commands and settings show on the next tick. Closed tabs' rows are dropped on the tick.
- Rows sit inside the tab's card around the body, so a terminal gives up grid rows to them (its `Resize` comes from its layout bounds).

## The built-in cache plugin

`cache` runs `macaroni-cli plugin cache`: for a Claude Code or Codex panel with a transcript, how long since the transcript last changed against the cache lifetime (`minutes`, or `claude_minutes`/`codex_minutes`, default 5): warm (green, then yellow in the last minute) with the time left, "working" while the agent works, then cold (dim) with the idle time. Interval 5 s. Codex panels only have a transcript once `macaroni-cli setup codex` is done and a turn has finished.

## Adding a built-in plugin

Add its code to `crates/cli/src/plugins.rs` and a `BuiltIn` variant in `crates/cli/src/main.rs`, then a `Plugin` with `Run::Cli(["plugin", "<name>"])` in `built_in()`. Its id can't be used by a folder.

## Traps

- `Run::Shell` on Windows passes the line to `cmd /C` with `raw_arg`: Rust's argument escaping would break `cmd`'s own quoting.
- `CREATE_NO_WINDOW` (`vscode::hide_window`) on every run, or a console flashes up each interval.
- Stdin is written from its own thread and both pipes are drained on threads, so a plugin that prints before reading can't deadlock the run.
- Testing in Git Bash: MSYS rewrites `/C`-style arguments (`MSYS_NO_PATHCONV=1` stops it).

## Testing

Unit tests: `crates/config/src/plugins.rs` (loading, overrides), `crates/app/src/plugins.rs` (rows, `is_on` and input, real runs through the shell), `crates/cli/src/plugins.rs` (the cache row). By hand, `echo '<input json>' | macaroni-cli plugin cache` prints a row; a folder plugin can be tried the same way from its folder.
