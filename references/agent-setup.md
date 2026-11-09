# Agent setup tab

## Mental model

A tab that shows coding agents' setup, Claude Code's and Codex's, the user's own and the tab's project's, in one list beside the selected item's text. It also gets the user into the agents' folders quickly: each `.claude` / `.codex` folder has buttons to open it as a workspace, in a terminal, in VS Code, or in the file manager. It reads; editing happens in the user's editor (Open, Create).

```
model:  TabKind::AgentSetup, project folder in Tab.cwd   (Command::OpenAgentSetup)
app:    App.agent_tabs: TabId -> AgentTab               (view/agent_tab.rs: state, AgentMessage, update, view)
scan:   agent_setup.rs: scan(home, project) -> Setup    (blocking; run via vscode::off_thread)
```

- **The model holds only the kind and the project folder** (none when blank). `OpenAgentSetup` emits nothing, `TabSpec::AgentSetup(folder)` saves it, `SavedTab.agent_setup` in `session.json`.
- **The project** is `agent_setup::project_of(pane_dir)`: the repository holding the pane's folder, else the folder itself; the home folder is no project (its setup is the personal one).
- **Reading**: opening the tab reads it; `Message::AgentTick` (every `AGENT_TICK`, only while one is on screen, `shown_agent_tabs`) reads each again, so edits made elsewhere show. One read at a time per tab (`AgentTab.loading`). The selection is kept across reads by `Item::key`, else the first existing item is selected.

## What's read (`agent_setup.rs`)

| Section | Personal | Project |
|---|---|---|
| Instructions | `~/.claude/CLAUDE.md` (listed even when missing), `~/.codex/AGENTS.md` (when `~/.codex` exists) | `CLAUDE.md` (listed even when missing), `.claude/CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md` (when Codex is in use) |
| Settings | `~/.claude/settings.json`, `~/.codex/config.toml` | `.claude/settings.json`, `.claude/settings.local.json`, `.codex/config.toml` |
| Skills | `~/.claude/skills/*/SKILL.md`, `~/.codex/skills/*/SKILL.md` | the same under the project |
| Subagents | `~/.claude/agents/*.md` | `.claude/agents/*.md` |
| Commands | `~/.claude/commands/**/*.md` (3 levels), `~/.codex/prompts/*.md` as `/prompts:<name>` | `.claude/commands/**/*.md` |
| Hooks | `hooks` in Claude settings: one item per event and matcher | the same |
| MCP servers | `~/.claude.json` `mcpServers`, `~/.codex/config.toml` `[mcp_servers.*]` | `.mcp.json`, `~/.claude.json` `projects[<path>].mcpServers` (keys use forward slashes; matched ignoring case on Windows), `.codex/config.toml` |

- Names and descriptions come from Markdown front matter (`name:`, `description:`), else the file name and the first line of text. A command in a subfolder shows the folder in its detail, as Claude Code lists it.
- Files are parsed whole and shown cut to `MAX_BODY` (`shown`): `~/.claude.json` is easily past it.
- Nothing is written by the scan. Create (`AgentMessage::Create`) makes an empty instructions file (with its folders, never over an existing one) and opens it.

## The list

- Sections are headed in capitals (`header`), each after a separator, with the count as a pill. Each item's Personal or Project is a badge in a palette color (blue, green). Rows are fixed-height buttons, so their content fills the height to be centered (see the button trap in [`app-layout.md`](app-layout.md)).

## The detail

- **Text** is a read-only `text_editor` (`AgentTab.text`: the item's key, a hash of its text, the `Content`), highlighted by extension (`file_view::syntax`), so it can be selected and copied (Ctrl+C in the editor; the `copy` action falls back to `App::agent_selection`). `agent_sync` (after each read, a selection or the Source button) makes a new `Content` only when the item or its text changed, so the tick doesn't drop a selection. Edits are dropped (`Action::is_edit`).
- **Markdown is rendered** in a preview page, as in a file tab (see [`file-tab.md`](file-tab.md)): `agent_sync` renders `preview::html::markdown` off the UI thread (`AgentMessage::Rendered`, keyed by `PageKey`: item, text hash, dark), and `agent_publish` publishes it to a page whose `Panel.file` is the agent tab (an id from `file_tab::next_page`). `agent_page_shown` shows it only while it holds the selected item's current text; until then, and with Source pressed (`AgentTab.source`), the editor shows. `App::tab_page_shown` (file or agent page) is what `place_browsers` checks, `App::page_link` routes the page's links (`AgentMessage::Link` → `open_page_link`, relative to the item's file), and `orphaned_pages` keeps a page while its agent tab exists. Where the preview server can't start, `page_error` says why and Markdown shows as its source.

## Actions

- Open uses `open_text_file` (VS Code's `code -g` when installed, else the plain text editor), never the file's default handler. Show in folder (`reveal`) selects the file in Explorer / Finder.
- A folder's Workspace button makes a new workspace named after it with it as the workspace folder (set in the same update, so its first terminal starts there). Terminal opens a new tab in the agent setup tab's pane, started in the folder through `App.restored` (a terminal starts where a restored tab was). VS Code uses `open_vscode` and only shows when `code` was found (looked up with each read, off the UI thread).
- Keys (`App::agent_key`, from `unhandled_key`): Up/Down move the selection, Enter opens or creates, F5 reads again.
- Entry points: "Agent setup" in the new-tab menu, the `agent_setup` action (no default shortcut; it's in the command palette).

## Traps

- **`serde_json`'s `preserve_order` is on in the workspace build** (the CLI asks for it) and off when the app is built alone, so map order (hooks, servers) differs between `cargo test -p macaroni` and `cargo test --workspace`. Tests must not depend on it.
- Files the agents themselves keep in `~/.claude` (projects, sessions, history) are never walked: they're large and private.
- Testing: `cargo test -p macaroni agent_setup` builds a fake home and project in the temp folder (named with the process id, deleted when the test passes).
