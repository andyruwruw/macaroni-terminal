# Coding agent integration (Claude Code and Codex)

## Mental model

Claude Code runs as an ordinary program in a terminal. Its hooks call `macaroni-cli hook`, which turns each event into control-API calls about its own tab (`MACARONI_TAB_ID`). The app stores the result as `Tab.agent` (with its `AgentKind`, session id, cwd and transcript) and saves the agent's session id and kind so a restart can resume it. Codex reports the same way through its `notify` program, but only when a turn finishes (see Codex below). Plugins read the agent's details, transcript included (see [`plugins.md`](plugins.md)).

```
hook event JSON ──► macaroni-cli hook ──► agent / notify calls ──► Tab.agent
status line JSON ─► macaroni-cli statusline ──► agent_summary ──► tab strip
quit ──► session.json (layout, cwd, agent session) ──► next launch: claude --resume <id>
```

## Key files

- `crates/cli/src/claude.rs`: `setup`/`configure` (settings merge), `hook_calls` (event to calls), `describe_tool`, `summary`/`statusline`.
- `crates/app/src/ipc.rs`: `App::agent`, `App::notify`.
- `crates/cli/src/codex.rs`: `configure`/`setup` (Codex's `config.toml`), `notify_calls`, `transcript`, `notify`.
- `crates/core`: `Tab.agent: Option<Agent>` (`kind: AgentKind`), `SetAgent` (keeps `session_id`/`cwd`/`transcript` when given `None`; a report from another kind of agent replaces the old one whole), `SetAgentSummary`, `ClearAgent`.
- `crates/app/src/session.rs` (`capture`, `load`, `save`, `is_safe_session_id`), `restore.rs` (`startup`), `update/terminals.rs` (types the resume command).

## Setup (`macaroni-cli setup claude`)

Merges into `~/.claude/settings.json` (or `--settings <path>`) a hook running `"<abs path to macaroni-cli>" hook` per handled event, plus `statusLine` unless the user has their own. Invariants: keeps the user's hooks and key order, idempotent, backs up first, `--remove` deletes only entries that `ours` recognizes (`macaroni-cli` in the command). The path is absolute, so moving the binary needs a re-run. The Agents settings page runs `setup claude --yes [--remove]`.

## Hook mapping (`hook_calls`)

- `SessionStart`: idle, records `session_id` and `cwd`. Every event passes `session_id` (and `transcript_path`) on, but only once its `transcript_path` exists: Claude saves a session only after its first message, and `--resume` can't find an unsaved one, so the tab keeps its last resumable id.
- `UserPromptSubmit`, `PreToolUse`: working; `PreToolUse` detail from `describe_tool`.
- `Notification`: needs input, then `notify`.
- `Stop`: idle "Done", then `notify`.
- `SessionEnd`: clears the agent.

To handle a new event, add it to the event list at the top of `claude.rs` and a branch in `hook_calls`.

Trap: `macaroni-cli hook` must never fail or print (some hooks' stdout goes into Claude's context) and is a no-op without `MACARONI_TAB_ID` or a reachable app, so the hooks are safe to leave installed.

## Codex

- `macaroni-cli setup codex` sets top-level `notify = ["<abs cli>", "codex-notify"]` in `$CODEX_HOME/config.toml` (else `~/.codex/config.toml`) with `toml_edit`, keeping the rest of the file as written. A `notify` of the user's own is kept after `--` in ours and run by `codex-notify` with the same argument; `--remove` puts it back. Ours is recognized by `macaroni-cli` in the program. The Agents settings page runs `setup codex --yes [--remove]`.
- Codex runs `notify` with the JSON as the last argument after each turn: `agent-turn-complete` with `thread-id`, `cwd` and `last-assistant-message`. `notify_calls` reports the agent (kind `codex`, idle, "Done", session id and transcript) and then `notify` with the message's first line, so finished-turn notifications and sounds work as for Claude Code. There's no working or needs-input state.
- The transcript is Codex's rollout file, `sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, found by id across every day folder (a resumed session keeps writing its old file). Without a `thread-id` (older Codex), the newest rollout of the last three days whose first line's `cwd` matches.
- Codex never says it exited: the app clears a Codex agent when the shell reports its command finished (OSC 133 `D`, `SessionEvent::CommandFinished`), so this needs macaroni's shell integration.
- Trap: like `hook`, `codex-notify` must never fail or print, and is a no-op without `MACARONI_TAB_ID`, so it's safe outside macaroni.

## Where agents show

The sidebar row and tab (`agent_line`), and the agent dashboard (`agent_dashboard`; see [`app-layout.md`](app-layout.md)), which also shows `Agent.summary` from `macaroni-cli statusline` and how long the agent has been in its state (`App.agent_since`, kept by `ipc.rs` as hooks arrive). To try it without Claude, pipe a hook's JSON into `macaroni-cli hook` with `MACARONI_TAB_ID` set, from bash: PowerShell 5's pipe into a native program doesn't deliver it intact.

## Notification gating

`App::notify` reads the agent state the hook set just before `notify`: idle means finished (`[agents] notify_finished`), needs input means `notify_input`. Keep the hook's order: state first, then `notify`. Everything after goes through the attention path in [`notifications.md`](notifications.md).

## Session restore

- `session.json` sits next to `config.toml`, built from `Model::spec()` and rebuilt with `Model::restore()` (new tabs in saved order, windows with their positions; see [`app-layout.md`](app-layout.md)). Per tab: `cwd` (the agent's cwd while one runs, since `claude --resume` only finds a session from its own directory; else OSC 7) and agent session, plus `ssh` (the profile name) for SSH tabs, which reconnect on restore while the profile still exists. Hidden tabs' terminals start at launch, a few at a time once the shown ones are up, at the shown one's size (`App::start_hidden`), so every restored agent resumes without its tab being opened. Trap: restored details wait in `App.restored` until a tab's terminal starts, so `capture` takes them from there for unstarted tabs; reading only the model would drop them on the next save.
- Saved at most once per `SESSION_SAVE_INTERVAL` after changes (moving or resizing a window counts), and when the last window closes (the app intercepts close requests to save first). Closing one of several windows closes what's in it, so it isn't restored. After that close save, `App::exit` turns saving off: agents killed by the close can still report (`SessionEnd` clears the agent) while browsers finish closing. Closing the last workspace forgets the session. `macaroni <program>` neither restores nor saves.
- With `[session] resume_agents`, a restored agent tab gets `claude --resume <id>` or `codex resume <id>` typed into its shell, by `SavedTab.agent_kind` (left out for Claude Code, so older files read as Claude's). Trap: the id is typed into a shell, so it must pass `is_safe_session_id` first.

## Input

- Shift+Enter sends `ESC CR` (`crates/term/src/keys.rs`), which Claude Code reads as a newline.

## Testing

Unit tests live in `claude.rs` and `codex.rs`; `setup codex --settings <scratch file>` tries the merge. End to end: pipe hook JSON into `MACARONI_TAB_ID=<tab> macaroni-cli hook` against an app on a scratch `MACARONI_SOCKET`, and run `setup` with `--settings` on a scratch file.
