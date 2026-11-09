# tmux control mode

## Mental model

`tmux -CC` turns a terminal (the gateway: a local shell, or SSH) into a pipe for tmux's control protocol. macaroni then shows the tmux session's windows as tabs beside the gateway, each drawing its window's panes where tmux puts them, and every pane is a real macaroni terminal (scrollback, selection, find, copy).

```
gateway Session (any source) ─ pty-reader ─► tmux::Gate ─┬─► emulator (only what's outside control mode)
                                                         └─► TmuxClient::line ─► %output ─► pane Session (Source::Tmux) reader
                                                                               └► TmuxEvent on the gateway ─► App::tmux_event
pane Session writer ─► send-keys -H ─► gateway input ─► tmux
App: TabKind::Tmux tab per window (App.tmux[gateway].windows), pane sessions in App.running under ids from PANE_IDS
```

- **Detection** (`term/src/tmux.rs`, `Gate`): every session but a tmux pane's runs its output through a `Gate`, which looks for `ESC P 1000 p` (held across reads if cut off), then hands lines to the client until `%exit` and its `ESC \`.
- **Replies are matched by order.** Every command line produces one `%begin` ... `%end`/`%error` block; `Inner::pending` holds what each is for (`Windows`, `Capture(pane)`, `Ignore`). Blocks not from this client (flag 1 unset; on tmux before 3.2, only the first block) are skipped.
- **A pane starts from a capture.** New panes (seen in a layout) get `capture-pane -p -e -J -S -2000 ; display-message cursor`, one reply. `%output` before the capture is dropped (the capture has it), after it is kept, then forwarded once the session opens (`open_pane` drains the backlog).
- **tmux owns the layout.** `Layout` is the parsed layout string in cells; the tab body (`view/tmux_tab.rs`) pins each pane's view at `cell × cell size` and draws a line in each one-cell border. The body reports its bounds (the generic `BrowserHost`), which become `refresh-client -C cols,rows` (one size for the client, so all its windows). A pane view's own resize only changes its grid (`PaneControl` does nothing).
- **The app** (`update/tmux.rs`): `Tmux` per gateway (client, windows by number with tab, layout and active pane, pane sessions). `Windows` syncs tabs (`Command::OpenTmux` after the session's last tab, `SetTmuxTitle`, closes gone ones) and on attach selects tmux's current window. `Layout`, `ActivePane`, `Renamed`, `WindowClosed` update it; `Detached` (or the gateway exiting) tears it down.
- **Going back to tmux**: clicking a pane `select-pane`; `split_*` in a tmux tab `split-window`; `new_tab` there `new-window` (`App::tmux_command`, before `apply`); closing the tab `kill-window` (`App::tmux_prune`, after commands that close tabs); Esc in the gateway `detach-client` (`tmux_gateway_input`; other typing there is dropped, since tmux would read it as commands).
- **Not saved.** `TabKind::Tmux` tabs are left out of `Model::spec` and can't be reopened; attaching again brings the windows back. An SSH profile's `tmux = "name"` runs `tmux -CC new-session -A -s name` instead of the shell (`SshTarget::command`, an `exec` request), so reconnecting reattaches.

## Files

- `crates/term/src/tmux.rs`: `Gate`, `TmuxClient` (commands, replies, notifications, pane routing), `TmuxPane` (the source), `Layout`, `TmuxEvent`, tests driving a whole session through a gate.
- `crates/term/src/session.rs`: the gate in `pty-reader`, `Session::tmux`, `SessionEvent::Tmux`.
- `crates/app/src/update/tmux.rs`: app state and handlers. `crates/app/src/view/tmux_tab.rs`: the body.
- `crates/core`: `TabKind::Tmux`, `Command::OpenTmux`, `Command::SetTmuxTitle`.

## Traps

- **Pane session ids aren't the model's.** They start at `PANE_IDS` (2^62). `session_event` ignores everything but `Wakeup` for them (no tab to title or notify; their end is tmux's to report), and `view_event` turns `Clicked` into `select-pane`.
- **Never type into a gateway in control mode**: anything but Esc is dropped.
- **`;` joins commands into one reply** (the capture and the cursor). Commands are sent as typed at tmux's prompt, so quoting follows tmux's rules.
- **Send keys as hex** (`send-keys -H`, tmux 3.0+), at most 256 bytes a command; literal mode would need quoting.
- **Testing**: `cargo test -p macaroni-term tmux`. A live check needs tmux 3.x on a host (or WSL/macOS): run `tmux -CC` in a terminal, or give an SSH profile `tmux = "main"`.
