# Claude Code integration

## Mental model

Claude Code runs as an ordinary program in a terminal. Its hooks call `macaroni-cli hook`, which turns each event into control-API calls about its own tab (`MACARONI_TAB_ID`). The app stores the result as `Tab.agent` and saves the agent's session id so a restart can resume it.

```
hook event JSON ──► macaroni-cli hook ──► agent / notify calls ──► Tab.agent
status line JSON ─► macaroni-cli statusline ──► agent_summary ──► tab strip
quit ──► session.json (layout, cwd, agent session) ──► next launch: claude --resume <id>
```

## Key files

- `crates/cli/src/claude.rs`: `setup`/`configure` (settings merge), `hook_calls` (event to calls), `describe_tool`, `summary`/`statusline`.
- `crates/app/src/ipc.rs`: `App::agent`, `App::notify`.
- `crates/core`: `Tab.agent: Option<Agent>`, `SetAgent` (keeps `session_id`/`cwd` when given `None`), `SetAgentSummary`, `ClearAgent`.
- `crates/app/src/session.rs` (`capture`, `load`, `save`, `is_safe_session_id`), `restore.rs` (`startup`), `update/terminals.rs` (types the resume command).

## Setup (`macaroni-cli setup claude`)

Merges into `~/.claude/settings.json` (or `--settings <path>`) a hook running `"<abs path to macaroni-cli>" hook` per handled event, plus `statusLine` unless the user has their own. Invariants: keeps the user's hooks and key order, idempotent, backs up first, `--remove` deletes only entries that `ours` recognizes (`macaroni-cli` in the command). The path is absolute, so moving the binary needs a re-run. The Agents settings page runs `setup claude --yes [--remove]`.

## Hook mapping (`hook_calls`)

- `SessionStart`: idle, records `session_id` and `cwd`.
- `UserPromptSubmit`, `PreToolUse`: working; `PreToolUse` detail from `describe_tool`.
- `Notification`: needs input, then `notify`.
- `Stop`: idle "Done", then `notify`.
- `SessionEnd`: clears the agent.

To handle a new event, add it to the event list at the top of `claude.rs` and a branch in `hook_calls`.

Trap: `macaroni-cli hook` must never fail or print (some hooks' stdout goes into Claude's context) and is a no-op without `MACARONI_TAB_ID` or a reachable app, so the hooks are safe to leave installed.

## Notification gating

`App::notify` reads the agent state the hook set just before `notify`: idle means finished (`[agents] notify_finished`), needs input means `notify_input`. Keep the hook's order: state first, then `notify`. Everything after goes through the attention path in [`notifications.md`](notifications.md).

## Session restore

- `session.json` sits next to `config.toml`, built from `Model::spec()` and rebuilt with `Model::restore()` (new tabs in saved order). Per tab: `cwd` (OSC 7, else the agent's cwd) and agent session.
- Saved at most once per `SESSION_SAVE_INTERVAL` after changes, and on window close (the app intercepts close requests to save first). Closing the last workspace forgets the session. `macaroni <program>` neither restores nor saves.
- With `[session] resume_agents`, a restored agent tab gets `claude --resume <id>` typed into its shell. Trap: the id is typed into a shell, so it must pass `is_safe_session_id` first.

## Input

- Shift+Enter sends `ESC CR` (`crates/term/src/keys.rs`), which Claude Code reads as a newline.

## Testing

Unit tests live in `claude.rs`. End to end: pipe hook JSON into `MACARONI_TAB_ID=<tab> macaroni-cli hook` against an app on a scratch `MACARONI_SOCKET`, and run `setup` with `--settings` on a scratch file.
