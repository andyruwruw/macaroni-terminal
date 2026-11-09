# Command history

## Mental model

Every command the user runs is remembered per workspace, tagged with the pane it ran in, and listed by a mode of the command palette (`command_history`, Ctrl+Shift+K / Cmd+Shift+K, or Command History… on a terminal tab's menu). It exists because a tab's title shows the running command only while it runs.

```
shell integration: A prompt B  typed command  C output D
reader thread: feed up to C ─► Emulator::typed_command ─► SessionEvent::CommandStarted(String)
App::session_event ─► record_command ─► History::record(ws, pane, command, now)
palette Mode::History { ws, pane } ─► History::listed ─► history_items ─► Enter: type_command
save_session ─► History::saved ─► SavedWorkspace::commands ─► App::new ─► History::restore
```

## Capturing the command (`term` crate)

- The command is read off the grid, not from typed keys: editing, completion and history recall all end up on screen. `Emulator::typed_command` (`emulator.rs`) scans up from the cursor (at most `MAX_COMMAND_ROWS`) for the last cell carrying the prompt's mark link (`macaroni-mark:prompt`, closed by OSC 133 `B`), and takes the text from the next cell to the cell before the cursor, rows joined with newlines, trimmed. A row with an output mark in the way means this command's prompt wasn't marked: `None`.
- It has to be read the moment `C` is fed, before output moves the cursor. The session reader (`session.rs`) records each `Mark::Output` offset in the chunk and feeds the emulator in parts cut there (each part with the inserts at or before its cut), calling `typed_command` at each cut. Events are sent after the emulator lock is released.
- It needs `B`: without it the prompt link runs on through the typed text. bash gets it appended to `PS1` by `PROMPT_COMMAND` (unless already there), zsh by the `__macaroni_command_mark` precmd hook (which moves itself to the end of `precmd_functions` so prompt themes that rebuild `PS1` run first), PowerShell's `prompt.ps1` prints it after the prompt. See `terminal-and-pty.md`, Prompt marks.

## The store (`crates/app/src/history.rs`)

- `History.workspaces: HashMap<WorkspaceId, Vec<Ran>>`; a `Ran` is a command, its pane (`None` once a restored pane is gone), its runs and last run (Unix seconds). The same command in the same pane counts again instead of adding a line. Past `MAX_KEPT` the least recently run goes.
- `listed(ws, pane, query)` merges a command's panes into one `Listed`, keeps those containing every word of the query (ignoring case), marks up to `COMMON` commands with at least `COMMON_RUNS` runs as common, and sorts: common by runs, then the pane's own, then the rest, newest first.
- `forget` drops a command from every pane of the workspace (Shift+Delete in the picker).
- Saved as `SavedWorkspace::commands` (`session.rs`, `SavedCommand`), each pane as its index in the layout's leaf order: `save_session` fills it, `App::new` gives it back with `History::restore` after `restore::startup`, relying on `Model::restore` keeping the order of workspaces and panes. Recording sets `session_dirty`.

## The picker (`view/palette.rs`)

- `PaletteMessage::OpenHistory(Option<PaneId>)` (`None` is the focused pane) opens `Mode::History { ws, pane }`; `history_items` turns `listed` into items (star icon for common ones, detail "This pane · 4 runs · 2h ago").
- Enter types the pick at the pane's prompt (`type_command`: selects the pane's tab, `App::paste`, so it's bracketed when the program asked); with Ctrl or Cmd held it also sends `\r`. `on_submit` can't see modifiers, so `palette_keys` forwards `ModifiersChanged` as `PaletteMessage::Modifiers` into `Palette.modifiers`, and Shift+Delete as `PaletteMessage::Forget`.

## Traps

- Only shells with the integration record anything: bash and zsh started by macaroni (a `.bashrc` that sets its own `PROMPT_COMMAND` replaces ours), plain PowerShell, and fish only if it sends OSC 133 with `B` itself. cmd and remote shells over SSH don't.
- zsh's right prompt (`RPROMPT`) sits after `B` on the command's row and would be read as part of the command unless `TRANSIENT_RPROMPT` clears it first.
- Commands are saved in `session.json` in plain text, as a shell's own history file is.
- Tests: `the_typed_command_is_read_from_after_the_prompt` (emulator), `a_marked_command_is_reported_when_it_starts` (a program printing the marks) and `shells_report_typed_commands` (Git Bash or bash, and PowerShell on Windows, typed into for real) in `term`; `history.rs`'s own tests for ordering, filtering, limits and saving.
