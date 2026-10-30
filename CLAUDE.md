# macaroni

A native, GPU-rendered terminal for Windows and macOS in the spirit of cmux: a vertical workspace sidebar, split panes with optional tabs, and attention notifications for coding agents. Rust + iced, with `alacritty_terminal` for emulation and `portable-pty` (bundled ConPTY on Windows) for shells. See `README.md` to build and run.

Docs describe the code as it is now: no plans, roadmaps or "not yet" notes in committed files or comments. A local, gitignored `TODO.md` may hold unfinished work; don't assume it's present in a fresh clone.

## Documentation standard

This repo overrides the global "no comments" default. Every module has a `//!` doc and every item (private ones too) a `///` doc; comments inside functions explain the why (invariants, ordering, platform traps, non-obvious math), never what a line obviously does. Lints enforce the item docs. See `CONTRIBUTING.md` for the full standard, and record user-visible changes in `CHANGELOG.md`.

## References: the project handoff knowledge base

`references/` holds short per-domain docs for agents working on the code: the mental model, the invariants, the traps, the files that matter, and how to extend each area. They're not user docs; keep them concise. Read the relevant file on demand before working in that area.

| File | What's inside |
|------|---------------|
| [`references/architecture.md`](references/architecture.md) | Workspace layout, the crates and their one-way dependency rules, key files, lints and doc requirements, CI, the Windows CEF/Ninja and `cargo xtask dev` / `bundle` traps, and the build/check commands. |
| [`references/model-and-commands.md`](references/model-and-commands.md) | The pure `core` crate: Workspace > SplitNode > Pane > Tab, `Command` -> `Model::apply` -> `Effect`, invariants, close/focus/move rules, adding a command, effect or tab kind, and the randomized test. |
| [`references/app-layout.md`](references/app-layout.md) | The app crate's UI: how `App::view` is layered, the update/apply loop, pane layout math and shared borders, `Chrome` and glass, sidebar and tab strip, focus, renaming, dragging, menus, the settings dialog (adding settings, colors, numbers), icons, shortcuts, iced traps, and driving the UI from a script. |
| [`references/config.md`](references/config.md) | The `config` crate: location (`MACARONI_CONFIG_DIR`), the template and its sync tests, parsing and checks, solid vs glass defaults, color resolution and `border` followers, live reload, errors vs warnings, the keymap, and how to add a setting, style number, color, theme or action. |
| [`references/claude-code.md`](references/claude-code.md) | Claude Code integration: `macaroni-cli setup claude` and its merge rules, hook events to agent state, notification gating, the status line, restore with `claude --resume`, Shift+Enter, and testing. |
| [`references/control-api.md`](references/control-api.md) | The socket API and `macaroni-cli`: endpoint and owner-only access, `MACARONI_SOCKET` / `MACARONI_TAB_ID`, the JSON-lines protocol (`Call` / `Event`), dispatch and task rules, adding a method, and end-to-end testing. |
| [`references/notifications.md`](references/notifications.md) | Agent attention: the OSC 9/777/99 sniffer, `Notice` on tabs, the `App::attention` path (muting, visibility, OS notifications), where it shows, what clears it, platform traps, and testing. |
| [`references/terminal-and-pty.md`](references/terminal-and-pty.md) | The `term` crate: `Session` threads and merged wakeups, lock and reply-ordering rules, the `Emulator` boundary over `alacritty_terminal`, key/paste/mouse encoding, shells and env, the bundled ConPTY and its traps, tests and benchmarks. |
| [`references/browser.md`](references/browser.md) | Browser tabs: CEF as a native child window over a placeholder, lazy start and placement, keyboard and focus routing, links and popups, building CEF (CMake/Ninja, `CEF_PATH`), and the traps (GL backend, embedded manifest, one profile per process, shutdown order, overlap). |
| [`references/rendering-and-text.md`](references/rendering-and-text.md) | The `TerminalView` widget: event routing, `Metrics`, row runs and the row cache, box drawing, draw order, mouse reporting, IME, bundled fonts, and the iced text traps. |

### Keeping references current (required)

- When you change a domain, update its reference file in the same change.
- When you introduce a new domain, add `references/<domain>.md` and a row to the table above. Write a reference when its code lands, not before.
- Every `references/*.md` must have a row, and every row must point to a real file.
- Write for a competent agent with zero prior context: lead with the mental model and invariants, name the key files, call out the traps. The code is the source of truth; the reference is the map.
