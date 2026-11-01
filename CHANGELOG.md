# Changelog

All notable changes to macaroni are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Buttons at the right of each tab strip open a terminal, a browser tab or VS Code. Turn them off in Settings > Tabs.
- VS Code tabs (Windows): an opt-in download in Settings > Integrations > VS Code runs VS Code for the web in a browser tab, opened at the pane's folder from the + menu or the tab strip. It's served only to this machine, with a private token, and restored tabs reconnect after a restart.

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
