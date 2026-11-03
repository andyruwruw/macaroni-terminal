# Changelog

All notable changes to macaroni are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0-alpha.3] - 2026-11-02

Agent tools (a dashboard, an inbox, answering permission prompts), prompt marks, triggers, inline images, scrollback that survives a restart, find and zoom in browser pages, and the solid look by default.

### Added

- Browser tabs on macOS, with Safari's engine.
- Rearrange panes: `swap_left`, `swap_right`, `swap_up` and `swap_down` (Ctrl+Alt+Shift+arrows, Cmd+Option+Shift+arrows on macOS) swap the focused pane with its neighbour, `rotate_panes` moves every pane one place along, and `equalize_panes` gives side-by-side panes equal room.
- `{sync}` in `[workspace_items]` templates shows how far a workspace's branch is ahead of and behind its upstream (`↑2↓1`); Settings > Sidebar > Detail > Git branch includes it.
- Pane badges: `[workspace_items] pane_badge` (Settings > Sidebar) shows a large faded label, like `{name}` or `{branch}`, in the top right of each of a workspace's panes.
- `[font] fallback`: fonts tried, in order, for characters the terminal font lacks.
- Find in a browser tab's page: Ctrl+F (Cmd+F on macOS) opens a Find in page card under its address bar; Enter and the arrows step through the matches.
- Zoom a browser tab's page with Ctrl+= / Ctrl+- / Ctrl+0 (Cmd on macOS), the same shortcuts that zoom a terminal.
- Prompt marks in PowerShell: the pass/fail bars, prompt jumps, copying the last output and long-command notices work there too, and its tabs reopen in their folder.
- Inline images: a program can show a picture in the terminal with iTerm2's image sequence (OSC 1337, as `imgcat` sends), at its own size or the width and height it asks for. It scrolls with the text and goes when the text is cleared.
- Triggers: `[[trigger]]` tables in `config.toml` match a regex against each line a terminal prints and highlight the match on screen, notify, or set the tab's badge (`$1` puts in a group) until you look at it. With shell integration, the prompt and the command you typed don't count.
- Search every terminal's scrollback: Ctrl+Shift+G (Cmd+Option+F on macOS, `search_all`) lists matching lines from every tab, newest first, and `search_workspace` just the active workspace's; pick one to open its tab with the find bar on that line.
- Scrollback across restarts: each terminal's text is saved when macaroni quits and shown again, dimmed, in its restored tab (`[session] keep_scrollback`, on by default). `save_scrollback` (in the command palette) saves a terminal's text to a file.
- Per-workspace environment variables and a startup command (Environment & Startup… on a workspace's menu): its terminals get the variables, and each new shell there runs the command first (not SSH tabs). Both are saved with the session.
- Reopen closed tabs: Ctrl+Shift+R (Cmd+Shift+R, `reopen_tab`) brings back the last 20 tabs you closed, newest first, in their pane: a terminal in its folder, resuming its Claude Code session, a browser tab at its page.
- Jump to a pane by letter: Ctrl+Shift+J (Cmd+Shift+J, `jump_to_pane`) shows a letter over each pane; press one to focus it.
- Answer Claude Code's permission prompts without switching to it: its notification has Allow and Deny buttons (on macOS, an Answer menu), and so does its card in the agent dashboard. They answer only while the prompt is still on screen.
- An agent dashboard: Ctrl+Shift+A (Cmd+Shift+A, `agent_dashboard`) shows a card for every Claude Code session in every workspace, most urgent first, with what it's doing, its status line and how long it's been at it; click one to go to it.
- Ctrl+click (Cmd+click on macOS) a file reference in a terminal (`src/main.rs:12:5`, `C:\work\x.py:3`, `main.cpp(40,7)`) to open it at that line in VS Code, or in the system's plain text editor without it. Relative paths are found from the tab's folder.
- Prompt marks (OSC 133), which bash (Git Bash included) and zsh now send, so they work on macOS too: a bar beside each prompt shows how the command before it ended (red failed, green succeeded); Ctrl+Shift+Up and Down (Cmd+Up and Down on macOS) jump between prompts, Ctrl+Shift+O (Cmd+Shift+O, `copy_output`) copies the last command's output, and a command that ran 30 seconds or more asks for attention when it finishes (`[notifications] long_command`).
- Worktree workspaces: New Worktree… on a workspace's menu (or `new_worktree`) adds a git worktree for a branch beside the repository and opens it as a new workspace named after the branch, optionally starting Claude Code in it.
- An attention inbox: Ctrl+Shift+I (Cmd+Shift+I, `show_waiting`) lists every tab waiting for you, oldest first, with its notice; Ctrl+Shift+U (Cmd+Shift+U, `next_waiting`) jumps to the next one.
- A command palette (Ctrl+Shift+P, Cmd+Shift+P on macOS, `command_palette`): type to find and run any action, or go to a workspace, a tab, an SSH profile or a settings page.
- `{branch}` and `{dirty}` in `[workspace_items]` templates show a workspace's git branch and a `*` for uncommitted changes (Settings > Sidebar > Detail > Git branch).
- Notifications carry macaroni's name and icon on Windows instead of "Windows PowerShell", and clicking one (on Windows and macOS) brings macaroni up on the tab that asked.
- Maximize a pane: Ctrl+Shift+Enter (Cmd+Shift+Enter on macOS, `maximize_pane`) or a tab's right-click menu makes its pane fill the workspace; again, or moving focus to another pane, brings the layout back.
- A default folder (`[terminal] default_dir`, or Settings > Terminal): where terminals start in a workspace that has no folder of its own.
- `[browser] engine` (Settings > Integrations > Browser, Windows): pages use the system's web view by default, Edge's WebView2 on Windows; `"chromium"` keeps embedded Chromium.

### Changed

- A terminal's find bar takes the place of the new tab buttons while it's open, and narrows in a narrow pane instead of running off the edge.

- The solid interface (flat greys) is the default. Set `ui_theme = "glass"`, or pick Glass in Settings > Theme, for the translucent one.
- Browser tabs use the system's web view by default instead of Chromium. A `window.open` popup (a sign-in window, say) opens as a tab with it.
- Chromium no longer ships with macaroni, so the Windows download is about 40 MB instead of 400. Download it in Settings > Integrations > Browser (about 175 MB) to use the Chromium engine.

## [0.1.0-alpha.2] - 2026-10-31

VS Code and SSH tabs, reorganized settings, and more reliable session restore.

### Added

- Buttons at the right of each tab strip open a terminal, a browser tab or VS Code. Turn them off in Settings > Tabs.
- VS Code tabs (Windows): an opt-in download in Settings > Integrations > VS Code runs VS Code for the web in a browser tab, opened at the pane's folder from the + menu or the tab strip. It's served only to this machine, with a private token, and restored tabs reconnect after a restart.
- Downloads for Windows (x64) and macOS (Apple silicon) on GitHub Releases.
- `./install` builds macaroni from source, installs it and starts it, on macOS and Windows; `--uninstall` removes it.
- SSH profiles: `[[ssh]]` entries in config.toml, or Settings > Terminal > SSH profiles. Each opens from the new-tab menu (right-click a pane's +) as a terminal running `ssh` to that host, and reconnects when the session is restored. A connection that fails or drops leaves the tab open with the error; Enter connects again.

### Changed

- After a restart, every tab's terminal starts at launch, so Claude Code resumes in all of them, not only once each tab is opened.
- Hovering a tab shows its icon and full name, and what its agent is doing with its status line. This replaces the agent status at the right of the tab strip (`[agents] tab_status` is gone).
- Settings has an Integrations group: Agents, Browser (moved from General) and VS Code.
- Settings are reorganized. Appearance is split into a page per part of the window (Theme, Window, Sidebar, Tabs, Panes, Menus & Controls, Effects), each with that part's colors beside its sizes and what it shows. The terminal font moved to Terminal, and Sidebar & Tabs folded into the Sidebar and Tabs pages.
- Every color, including the solid interface's own (tabs, workspaces, + buttons, borders), is in the release build's settings, each in one place. Rows the interface in use doesn't draw are hidden.
- Each page has one live preview, pinned above its cards so it stays in view while they scroll.
- Search also matches config.toml key names, and clicking a result's card title opens it on its page.

### Fixed

- Closing a browser tab quit the app.
- After a restart, `claude --resume` could be given a session Claude never saved (one started but given no message), and report "No conversation found". Tabs now keep the last session that can be resumed.
- Agents stopped by quitting could be cleared from the saved session while browser tabs finished closing.
- Restored tabs that weren't opened before the next save lost their directory and Claude Code session, so a later restart reopened them in the home folder.
- A tab running Claude Code is saved with Claude's directory, not the shell's last prompt, so `claude --resume` finds its conversation after `cd dir && claude`.

## [0.1.0-alpha.1] - 2026-10-29

An early alpha: expect rough edges, and settings and the control API may still change.

### Added

- The first version of macaroni: a native, GPU-rendered terminal for Windows and macOS with a workspace sidebar, split panes and tabs, browser tabs (Windows), attention notifications and Claude Code integration, a control API with `macaroni-cli`, session restore, and glass and solid interfaces with live-reloading settings. See the [README](README.md).
