# Changelog

All notable changes to macaroni are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0-alpha.4] - 2026-11-09

Git, file, agent setup and command history tabs, tmux control mode, panel plugins, Codex support, screen readers and keyboard navigation, accessibility settings, macaroni's own SSH client, and sounds you can try before picking.

### Added

- A crash leaves `crash.log` in macaroni's local data folder: what failed and where, and the last few hundred things the app did before it, without anything typed, pasted or shown. It holds the most recent crash.
- 25 more sounds for agents asking for attention and the bell (Settings > Notifications > Sound): success, notification and error sounds, recorded, before the tones.
- The control API's `list` (and `macaroni-cli list --json`) shows each workspace's sidebar group: its id, name and whether it's folded.
- Sounds can be tried before picking one: Settings > Notifications opens a sound menu grouped into success, notification, error and tones, where each sound's play button plays it and clicking its name picks it. A play button beside the setting replays the one picked.
- Click in the command you're typing at a shell's prompt to move the cursor there (bash, zsh and PowerShell started by macaroni).
- Agent setup: Markdown files are rendered, as in a file tab, with a Source button to see the raw text. The raw text (every file) is highlighted and can be selected and copied.
- Git tab: drag the edge of the file list to resize it, and wrap a diff's long lines with the button above it. The file list has the sidebar's background.
- Git tabs (Ctrl+Shift+H, Cmd+Shift+H, or Git in the new-tab menu): a repository's changes and history in the manner of GitHub Desktop. A top bar shows the repository and branch, with menus to switch either or create a branch, and a button that publishes, pulls, pushes or fetches. Changes lists every changed file with a checkbox for the next commit, the file's diff, and a summary and description to commit with; a file's changes can be discarded. Right-click the tab, or anywhere in it, to point it at another folder. Its button in the tab strip is grayed out outside a repository. History lists the branch's commits, marking those not pushed yet, with each commit's files and diffs. The tab follows changes made elsewhere on its own.
- Screen reader support (Narrator, NVDA and other UI Automation readers on Windows, VoiceOver on macOS): the sidebar's workspaces and groups, each pane's tabs and buttons, menus, the settings dialog, the command palette and each terminal's visible text with its cursor. New output in the focused terminal is read out as it arrives (`[accessibility] announce_output`).
- Keyboard navigation: F6 (Shift+F6 back) moves the keyboard from the terminal to its pane's tab strip, then the sidebar, with a focus ring; arrows move it, Enter or Space picks, F2 renames, Delete closes a tab, and Shift+F10 or the menu key opens the item's menu, whose items the arrows move through too. The settings dialog takes arrows, Space and Enter, and a Git tab arrows, Space, Delete and Ctrl+Enter.
- Settings > Accessibility (`[accessibility]`): an interface size for the whole window (`ui_scale`), high contrast (a stronger interface and the new `macaroni-high-contrast` terminal theme), a minimum contrast for terminal text, reduced motion, and a thicker cursor. High contrast and reduced motion follow the system's settings unless set.
- File tabs (File in the new-tab menu, the `open_file` action, `macaroni-cli open <path>` or the `open_file` method): one file in a tab. Text and code in the terminal font, highlighted by language, selectable and copyable; Markdown rendered with its tables, code blocks and the images beside it (Source shows the text), its links opening pages or other files; pictures and SVGs fitted to the tab; videos and sounds with a player, and animated GIFs (Windows and macOS). The tab reads the file again when it changes, keeps its place, and has buttons to open the file in an editor or show it in its folder. Its name opens a list of the other files in the folder, and a bar along the bottom (or Left and Right on a picture) steps to the previous or next one. The button at the left of its bar shows a folder tree of the file's repository (or folder) beside it, opened down to the file and following new files as they're made; click a file to show it, or right-click it for the places it can open.
- File viewers (Settings > Files, Windows and macOS): optional downloads that let file tabs draw Mermaid diagrams, TeX math (KaTeX), Graphviz graphs, PlantUML and D2, in Markdown code blocks and in `.mmd`, `.dot`, `.puml` and `.d2` files (Drawing / Source switches). Each is a pinned npm package checked against its SHA-256. Rendered Markdown now shows in a web page, with GitHub's extras (task lists, footnotes, alerts) and its HTML, which can't run scripts.
- Agent setup tabs (Agent setup in the new-tab menu, or the `agent_setup` action): Claude Code's and Codex's setup, your own (`~/.claude`, `~/.claude.json`, `~/.codex`) and the project's, listed together: instructions (`CLAUDE.md`, `AGENTS.md`, created with one click when missing), settings, skills, subagents, slash commands and Codex prompts, hooks and MCP servers, each with its text beside the list and buttons to open it or show it in its folder. Each `.claude` and `.codex` folder opens as a workspace, in a terminal, in VS Code or in the file manager. The tab follows changes made elsewhere.
- Command history (Ctrl+Shift+K, Cmd+Shift+K, or Command History on a terminal tab's menu): every workspace remembers the commands run in each of its panes and lists them, its most run first and then the pane's own, newest first. Type to filter, Enter to type one at the prompt, Ctrl+Enter (Cmd+Enter) to run it, Shift+Delete to forget it. Kept across restarts. Recorded in bash, zsh and PowerShell started by macaroni, which now also mark where the command line starts (OSC 133 `B`).
- Panel plugins: small programs that add rows to the top or bottom of panels. Put one in the `plugins` folder next to `config.toml` (a `plugin.toml` naming its command, where its rows go, how often it runs, and which panels it's for, such as Codex or Claude Code ones). macaroni runs it with the panel's details as JSON (its tab, folder, and agent with session id and transcript) and shows each line it prints, colors and all. Turn plugins on or off for a panel from its tab's menu (Plugins), or everywhere with `[plugins.<name>]`, which also passes settings to it.
- A built-in "Prompt cache" plugin shows under Claude Code and Codex panels whether the agent's prompt cache is still warm and for how long (5 minutes unless `[plugins.cache] minutes` says otherwise), or how long it's been cold.
- Codex: `macaroni-cli setup codex` (or Settings > Agents) makes Codex report to macaroni after each turn, so its tab shows it, finished turns notify (with the attention sound), plugins see its session and transcript, and a restart resumes it with `codex resume`. A `notify` program you already had keeps running.
- tmux control mode: run `tmux -CC` in a terminal (local or SSH), or give an SSH profile `tmux = "main"`, and the tmux session's windows open as tabs beside it, each drawing its window's panes where tmux puts them as real macaroni terminals (scrollback, selection, find, copy). Clicking a pane selects it, splitting or opening a tab there does it in tmux, closing the tab closes the window, and Esc in the original terminal detaches, leaving the session running.

### Changed

- The solid look's New workspace button has no outline (`add_workspace_border` brings one back).
- A tab asking for attention while you're not looking at it (an agent finishing or waiting for you) plays a soft chime instead of the system's notification sound. Settings > Notifications > Sound (`[notifications] sound`) picks another of macaroni's own sounds (Marimba, Bell, Rise, Blip, Bubble, Hum), the system's sound, a file, or none, and plays each as you pick it. The bell can use the same sounds.
- SSH profiles connect with macaroni's own SSH client instead of running `ssh`. It reads `~/.ssh/config` (HostName, User, Port, IdentityFile, ProxyJump and Include), signs in with the SSH agent (OpenSSH's or Pageant on Windows), your keys, keyboard-interactive or a password, and checks and records host keys in `~/.ssh/known_hosts`. Passwords, passphrases and new host keys are asked in the tab. Tabs to the same host share one connection, so only the first signs in. A profile's `args` is replaced by `proxy_jump`.

### Fixed

- Building from source no longer fills the disk: debug builds keep less debug info, and `cargo xtask dev` and `./install` clear their build folder once it's past 40 GB of old builds.
- Building from source no longer needs CMake or Ninja installed: `./install` and `cargo xtask` use Visual Studio's copies, or download official, checksum-checked portable ones into `target/tools`.
- Windows: dragging a window by its title bar no longer leaves it stuck to the cursor (snapping to the top half on the next click) or refusing to be dragged again after a quick click, and with several windows open each one is rescaled correctly when dragged to a monitor with a different scale.
- A tab's menu no longer runs past its edge with a long workspace folder: the folder shows on its own line, cut from the start.
- Split panes can be resized in the solid look again: the divider between panes had no width to grab.
- Thinner scrollbars in the Git, agent setup and file tabs, the file tree and the agent dashboard, beside lists instead of over their right edge.
- Agent setup: sections are easier to tell apart (headed in capitals under a line, with a count), each item's Personal or Project is a colored badge clear of the scrollbar, and rows are vertically centered.
- Git tab: the top bar's buttons are centered, a changed file's row and letter are vertically centered, long commit messages are cut off at the edge of the list instead of running past it, and the commit summary and description stand out from the list behind them.
- Terminals are ready sooner after a restart with many tabs: the window no longer freezes while shells start, the tabs you can see start first, and hidden tabs start a few at a time instead of all at once. Keys typed before a terminal has started are sent once it has.
- VS Code tabs no longer pile up old copies of VS Code: it downloads one per VS Code release (about 600 MB) and kept five, past 3 GB; now only the newest is kept.
- VS Code tabs can save settings, themes and extensions: the server's data folder no longer clashes with the file that keeps its port.

## [0.1.0-alpha.3] - 2026-11-02

Multiple windows, agent tools (a dashboard, an inbox, answering permission prompts), prompt marks, triggers, inline images, scrollback that survives a restart, find and zoom in browser pages, and the solid look by default.

### Added

- Multiple windows, each with its own sidebar of workspaces and groups. Ctrl+Alt+Shift+N (Cmd+Shift+N on macOS, `new_window`) opens one; `close_window` closes the focused one.
- Move workspaces and tabs between windows: a workspace's Move menu has To New Window and To Window N (`move_to_new_window` for the active one), and a tab's Move menu has New window and the other windows' workspaces. Running terminals and browser pages move with them.
- Drag a tab or workspace onto another window to move it there, or out onto the desktop to open a new window with it.
- Every window comes back after a restart, where it was and at its size. Closing one of several windows closes what's in it.
- `macaroni-cli new-window` opens a window.
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
- Ctrl+click (Cmd+click on macOS) a file named in a terminal (`src/main.rs:12:5`, `C:\work\x.py:3`, `main.cpp(40,7)`, `README.md`) to open it at that line in a file tab beside the terminal. Settings > Files (`[files]`) can open files in a VS Code tab or an app you pick (VS Code or the text editor by default) instead. Ctrl+right-click (Cmd+right-click) a file or link for a menu of places to open it, show it in its folder or copy it. Relative paths are found from the tab's folder.
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

- `macaroni-cli list` (and the control API's `list`) shows every window's workspaces, grouped under `window N`; each workspace reports its window and whether that window is focused.
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
