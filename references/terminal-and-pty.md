# Terminal sessions and PTY

## Mental model

`crates/term` (`macaroni-term`) turns "a program in a PTY" into "a grid you can copy and draw". It knows nothing about workspaces, tabs or iced; the app owns sessions and reacts to their events.

```
shell ⇄ PTY (portable-pty; bundled ConPTY on Windows)
  pty-reader: read ─► OscSniffer ─► AlacrittyEmulator::feed (Arc<FairMutex>) ─► write queued PtyWrite replies
              └─► notify(SessionEvent::Wakeup) once per snapshot ─► app calls Session::snapshot()
  pty-waiter: child.wait() ─► notify(SessionEvent::Exited)
  pty-sync:   flushes a synchronized update (mode 2026) past its deadline
```

- `Session::spawn(config, notify)` opens the PTY, spawns the program or default shell, and starts the threads. `notify` runs on those threads; the app forwards into its own channel.
- **Wakeups are merged.** An `AtomicBool` dirty flag allows one pending `Wakeup`; `snapshot()` clears it before copying.
- **The lock is held only to copy.** `snapshot()` copies visible rows, cursor, modes and color overrides into an owned `Snapshot`; rendering is lock-free.
- **PTY replies are written outside the emulator lock**, right after `feed` and before the next read. Writing under the lock would let a program that stops reading block `snapshot` on the UI thread; writing before the next read is what the ConPTY cursor trap needs.
- **Dropping a `Session` kills the child.**

## Files

- `session.rs`: `Session`, `SessionConfig`, the threads.
- `emulator.rs`: the `Emulator` trait and `AlacrittyEmulator`, its only implementation.
- `grid.rs`: our public types (`Cell`, `Color`, `CellFlags`, `Cursor`, `Modes`, `Snapshot`, `GridPoint`, `DefaultColors`). No `alacritty_terminal` type leaves the crate.
- `osc.rs`: `OscSniffer`, turns OSC 9/777/99 into `SessionEvent::Notify` and OSC 7 into `SessionEvent::Cwd`. See [`notifications.md`](notifications.md).
- `keys.rs` (`encode_key`, `encode_paste`), `mouse.rs` (`encode_mouse`), `links.rs` (`url_at`, `is_openable`): pure and unit tested. The app maps iced input to these in `crates/app/src/input.rs`.
- `shell.rs`: default shell, `available()`, `command()`, bash integration.

## Emulator boundary

- `Color` is unresolved (`Foreground`, `Background`, `Indexed`, `Rgb`); the app resolves it against its palette plus `Snapshot::color_overrides`. alacritty's `Dim*` colors map to the base index with `CellFlags::DIM`.
- Color (OSC 4/10/11/12) and size (`CSI 14t`/`18t`) queries need state alacritty's listener can't see, so `AlacrittyEmulator` queues them and answers after parsing (`answer_queries`), using program-set colors, else `DefaultColors` (from `SessionConfig.colors`, updated via `Session::set_default_colors`). OSC 52 clipboard reads are deliberately never answered.
- Cursor style: `SessionConfig.cursor` at spawn, `Session::set_default_cursor` on change. alacritty's `Term::set_options` replaces the whole config, so scrollback length must be passed back in with it.
- Selection (`start_selection`, `update_selection`, `selection_text`), search (`search`, `reset_search`, literal and smart-case) and `hyperlink_at` live on the trait and are exposed by `Session`. `GridPoint` is in visible-screen coordinates; the emulator applies the scroll offset.
- `is_openable` is the only gate for opening links: http(s) only, since terminal output is untrusted.

## Synchronized output

vte buffers a mode 2026 update but only checks its timeout when more bytes arrive. The reader sends the deadline to `pty-sync`, which calls `Emulator::flush_sync` and wakes the UI if the update is still open, so a program that never closes it can't freeze the screen.

## Keys

Shift+Enter sends `ESC CR` (Claude Code's newline). Ctrl+letter uses the key, not the platform text. Ctrl+Alt with printable text is AltGr and sends the text. `encode_paste` brackets only when the program enabled it.

## Shells and environment

- Default: login shell (`new_default_prog`) on macOS/Linux. Windows prefers Git Bash (`bash.exe --login -i`), then `pwsh.exe`, then `powershell.exe`. Never pick `C:\Windows\System32\bash.exe` (WSL's launcher).
- Bash gets `CHERE_INVOKING=1` (else Git Bash's `/etc/profile` jumps to `$HOME`). On Windows a `PROMPT_COMMAND` emits OSC 7 using bash builtins only (forking per prompt is slow under MSYS) and drops the first prompt's leading newline; a `.bashrc` that replaces `PROMPT_COMMAND` disables both.
- SSH profiles (`[[ssh]]`, `SshProfile::ssh_args`) are terminals running `ssh` (found on `PATH`; Windows has OpenSSH in System32) through the same `Session`. The app tracks them in `App.ssh_tabs` (session to profile name). When `ssh` exits non-zero the tab stays: `Session::print` writes a dimmed "disconnected" line into the emulator, the session joins `App.disconnected`, and Enter there runs the profile again in the same tab and `SessionId` (`App::reconnect`). Exit code 0 closes the tab as usual.
- Every session gets `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=macaroni`, plus `SessionConfig::env` (the app adds `MACARONI_SOCKET`, `MACARONI_TAB_ID`; see [`control-api.md`](control-api.md)). Start directory is home unless `SessionConfig::cwd` is set.

## Windows: bundled ConPTY

`vendor/conpty/{x64,arm64}/` holds `conpty.dll` and `OpenConsole.exe` (update steps in `vendor/conpty/README.md`). `crates/app/build.rs` copies them next to the executable; portable-pty loads `conpty.dll` by bare name, so the app directory wins. The session's console host child is `OpenConsole.exe` when the bundled one is used. It is required: with the inbox ConPTY, MSYS2 programs (vim, less) don't redraw after resize.

## Traps

- **Answer `ESC[6n` or the inbox ConPTY hangs.** portable-pty uses `PSEUDOCONSOLE_INHERIT_CURSOR`, so ConPTY asks for the cursor position at startup and outputs nothing until answered.
- **Don't detect exit by reader EOF.** ConPTY keeps the pipe open after the child exits; exit comes from `pty-waiter`. Output can arrive after `Exited`.
- ConPTY swallows OSC 10/11 queries, so color queries are only answered on macOS. Size queries work on both.
- ConPTY enables win32-input-mode (`ESC[?9001h`); alacritty ignores it, so input stays plain VT.

## Tests and benchmarks

- `cargo test -p macaroni-term`: emulator tests feed bytes and check the grid (inline `insta` snapshots), encoding tests, and `session::tests::real_shell_round_trip` (real shell through the platform PTY, with a grace period for late output).
- `cargo bench -p macaroni-term` (`benches/emulator.rs`, criterion): parse throughput with and without the sniffer, and snapshot cost.
