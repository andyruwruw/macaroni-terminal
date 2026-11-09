# Terminal sessions and PTY

## Mental model

`crates/term` (`macaroni-term`) turns "a program in a PTY, or a shell over SSH" into "a grid you can copy and draw". It knows nothing about workspaces, tabs or iced; the app owns sessions and reacts to their events.

```
Source::Local: shell ⇄ PTY (portable-pty; bundled ConPTY on Windows)
Source::Ssh:   remote shell ⇄ russh channel ⇄ task on the ssh runtime ⇄ pipe.rs (ChunkReader / InputWriter)
  both open to source::Opened { reader, writer, control (resize, ends on drop), waiter }
  pty-reader: read ─► OscSniffer ─► Lines + triggers ─► place images, AlacrittyEmulator::feed (Arc<FairMutex>) ─► write queued PtyWrite replies
              └─► notify(SessionEvent::Wakeup) once per snapshot ─► app calls Session::snapshot()
  pty-waiter: waiter() (child.wait(), or the SSH task's exit code) ─► notify(SessionEvent::Exited)
  pty-sync:   flushes a synchronized update (mode 2026) past its deadline
```

- `Session::spawn(config, notify)` opens `config.source` (`source::open_local` or `ssh::open`) and starts the threads. Nothing past `spawn` knows which source it is: a new source only has to produce an `Opened`. `notify` runs on those threads; the app forwards into its own channel.
- **Wakeups are merged.** An `AtomicBool` dirty flag allows one pending `Wakeup`; `snapshot()` clears it before copying.
- **The lock is held only to copy.** `snapshot()` copies visible rows, cursor, modes and color overrides into an owned `Snapshot`; rendering is lock-free.
- **PTY replies are written outside the emulator lock**, right after `feed` and before the next read. Writing under the lock would let a program that stops reading block `snapshot` on the UI thread; writing before the next read is what the ConPTY cursor trap needs.
- **Dropping a `Session` drops its `Control`**, which kills a local child (and closes the pseudoconsole) or tells the SSH task to close its channel.

## Files

- `session.rs`: `Session`, `SessionConfig`, the threads.
- `source.rs`: `Source`, `Opened`, `Control`, `open_local` (the PTY).
- `pipe.rs`: `ChunkReader` and `InputWriter`, blocking ends for a source on tokio; `Input` (data or a resize).
- `tmux.rs`: tmux control mode (`Gate`, `TmuxClient`, `Source::Tmux` panes, `Layout`); see [`tmux.md`](tmux.md).
- `ssh/`: macaroni's SSH client (see SSH below). `mod.rs` (`SshTarget`, `open`, the runtime, the connection pool, the host key check, `authenticate`, the shell pump), `config.rs` (`~/.ssh/config`), `prompt.rs` (`Term`: questions asked in the tab), `tests.rs` (against a russh server on localhost).
- `emulator.rs`: the `Emulator` trait and `AlacrittyEmulator`, its only implementation.
- `grid.rs`: our public types (`Cell`, `Color`, `CellFlags`, `Cursor`, `Modes`, `Snapshot`, `GridPoint`, `DefaultColors`). No `alacritty_terminal` type leaves the crate.
- `osc.rs`: `OscSniffer`, turns OSC 9/777/99 into `SessionEvent::Notify`, OSC 7 into `SessionEvent::Cwd`, OSC 133 into `OscEvent::Mark` (see Prompt marks below) and OSC 1337 `File=` into `OscEvent::Image` (see Inline images). See [`notifications.md`](notifications.md).
- `lines.rs`: `Lines`, output back into plain lines for triggers (see Triggers).
- `images.rs`: `ImageRequest` (parsing, `place`), `InlineImage` (`anchors`), `ImagePlacement`.
- `keys.rs` (`encode_key`, `encode_paste`), `mouse.rs` (`encode_mouse`), `links.rs` (`url_at`, `is_openable`, `file_at` for `path:line[:col]` and `path(line,col)` references and paths, which need an extension, and a bare name like `README.md` a letter in it): pure and unit tested. The app maps iced input to these in `crates/app/src/input.rs`.
- `shell.rs`: default shell, `available()`, `command()`, bash integration.

## Emulator boundary

- `Color` is unresolved (`Foreground`, `Background`, `Indexed`, `Rgb`); the app resolves it against its palette plus `Snapshot::color_overrides`. alacritty's `Dim*` colors map to the base index with `CellFlags::DIM`.
- Color (OSC 4/10/11/12) and size (`CSI 14t`/`18t`) queries need state alacritty's listener can't see, so `AlacrittyEmulator` queues them and answers after parsing (`answer_queries`), using program-set colors, else `DefaultColors` (from `SessionConfig.colors`, updated via `Session::set_default_colors`). OSC 52 clipboard reads are deliberately never answered.
- Cursor style: `SessionConfig.cursor` at spawn, `Session::set_default_cursor` on change. alacritty's `Term::set_options` replaces the whole config, so scrollback length must be passed back in with it.
- Selection (`start_selection`, `update_selection`, `selection_text`), search (`search`, `reset_search`, literal and smart-case) and `hyperlink_at` live on the trait and are exposed by `Session`. `GridPoint` is in visible-screen coordinates; the emulator applies the scroll offset.
- `is_openable` is the only gate for opening links: http(s) only, since terminal output is untrusted. File references are opened by the app (`update/links.rs`: `App::clicked_at`, after URLs) only when `resolve_file` finds an existing file (relative to the tab's cwd, MSYS `/c/` paths converted), and only in a file tab, a VS Code tab or a program the user chose (`[files]`, see [`file-tab.md`](file-tab.md)), never the file's default handler.

## Prompt marks (OSC 133)

- `shell::integrate` adds them. bash: `PROMPT_COMMAND` appends `B` to `PS1` (as `\[...\]`, unless it's there), prints `D;<exit>` (from `$?`, captured first), OSC 7, then `A`, and `PS0` prints `C` before each command runs; on Windows it's `BASH_PROMPT_COMMAND` (Git Bash: MSYS paths, first-prompt newline), elsewhere `BASH_PROMPT_COMMAND_UNIX`. A `.bashrc` that sets its own `PROMPT_COMMAND` replaces ours. zsh: `ZDOTDIR` points at `<temp>/macaroni-zsh`, whose `.zshenv` (`zshenv.zsh`, rewritten when it changes) restores the user's `ZDOTDIR` from `MACARONI_ZDOTDIR` (empty = unset), sources their `.zshenv`, and for interactive shells adds `precmd` hooks (D, OSC 7, A; then `B` appended to `PS1`, from a hook that moves itself to the end of `precmd_functions` so themes that rebuild `PS1` run first) and a `preexec` (C) hook with `add-zsh-hook`; the rest of zsh's startup files then load from the user's own folder. The login shell on macOS/Linux (`new_default_prog`) gets both, since it could be either. PowerShell (`is_powershell`, only when started with no arguments of the user's own): `-NoExit -EncodedCommand` runs `prompt.ps1` after the profile, which wraps the current `prompt` to print D (after a command ran), OSC 7 and A before it and B after it, and replaces PSReadLine's Enter with one that sends C for a non-blank line (PowerShell has no preexec). Side effect: no copyright banner. A profile's own Enter key handler is replaced. fish only if it sends OSC 133 itself. The bundled ConPTY passes OSC 133 through in order. The zsh script has only been checked by reading: no zsh on the Windows dev machine.
- alacritty ignores OSC 133, and row numbers go stale when scrollback is trimmed, so marks live in the cells: `OscSniffer::feed_at` reports where each sequence ended in the chunk, and the reader's `Marks` inserts an OSC 8 link there (`with_inserts`) before feeding the emulator. `A` opens `macaroni-mark:prompt[:<exit of the previous command>]`, `C` opens `macaroni-mark:output`, `B` and `D` close the link; each gets a unique `id=mcr<n>`. The links are never drawn and `hyperlink_at` hides them (`MARK_PREFIX`). A program's own OSC 8 in its output replaces ours on those cells, which is fine: only a mark's first row counts.
- `AlacrittyEmulator::marks` scans scrollback and screen for rows where a new mark id starts. `jump_to_prompt` scrolls the previous or next prompt to the top row (relative to the current top); `last_output` copies from the last output mark to the next prompt or the cursor.
- `Snapshot::prompts` lists the visible prompt rows with the previous command's exit code (`marks_between` over just the visible rows plus the one above, since it runs on every wakeup); the renderer's `draw_prompt_marks` draws a 2px bar in the left padding, palette red for a failure and faded palette green for success.
- At each `C` the reader feeds the emulator only up to the mark and reads `Emulator::typed_command`, the text between the prompt's link and the cursor, sending `SessionEvent::CommandStarted` (see [`command-history.md`](command-history.md)).
- `Marks` also times `C` to `D` and sends `SessionEvent::CommandFinished { exit, elapsed }`; a `D` with no `C` before it (Enter on an empty line, Ctrl+C at the prompt) sends nothing. The app turns a finish past `[notifications] long_command` seconds into `App::attention`.
- Click to move the cursor: a single left click with no drag, Shift or Alt sends `ViewEvent::MoveCursor`; `App::move_cursor` writes `Session::click_to_cursor`'s arrow keys (`CSI`, or `SS3` under DECCKM). It needs `Session.at_prompt`, which the reader sets at `B` and clears at any other mark, so a running command never gets arrows. `Emulator::cursor_moves` counts characters (a wide character's spacer cell isn't one) from the cursor to the click within the command: from `command_start` (the cell after the prompt's link, shared with `typed_command`) through the rows it soft-wraps onto (`WRAPLINE`), clamped to the prompt's end and one past the last non-blank cell. A multi-line command (a continuation prompt) only reaches its first line.
- App side: `previous_prompt`, `next_prompt`, `copy_output` (`Action::Prompt`, `Action::CopyOutput` in `update/shortcuts.rs`).

## Triggers

- The app's `[[trigger]]` rules that notify or badge reach the session as one `regex::RegexSet` (`SessionConfig.triggers`, `Session::set_triggers` on config change). Highlights never come here: the renderer matches the visible rows itself (see [`rendering-and-text.md`](rendering-and-text.md)).
- The reader feeds each chunk to `Lines` only while a set is present, and sends `SessionEvent::Triggered { trigger, line }` (index into the set) at most once per trigger per read, for its first matching line.
- `Lines` works on the byte stream, not the grid: escapes are dropped, a CR not followed by LF starts the line over (progress bars), and since ConPTY writes spaces as `CSI n C` and moves lines with `CSI H`, the first becomes spaces and the second ends the line. Lines are capped at 4 KiB.
- With OSC 133, lines ending between `A` and `C` (the prompt and the typed command) are dropped: the reader splits the chunk at each mark's offset and calls `Lines::set_quiet`. Without shell integration the typed command can match.

## Inline images (OSC 1337)

- Only iTerm2's `File=...;inline=1:<base64>` in one sequence (not `MultipartFile`, not Kitty's protocol). vte with `std` buffers an OSC of any length, so the sniffer gets the whole payload; `ImageRequest::parse` drops anything over 32 MiB or not `inline=1`.
- The reader places images **under the emulator lock** (`ImageRequest::place(emulator.size())`), since the cell count depends on the grid's size: `width`/`height` as cells, `px`, `%` or `auto`, aspect kept unless `preserveAspectRatio=0`, wider than the terminal scaled to fit, at most 500 rows. Only the image's header is read (`image::ImageReader::into_dimensions`); the app decodes.
- `InlineImage::anchors` replaces the sequence in the emulator's input (merged into `with_inserts`, sorted stably by offset): per row a blank cell carrying the OSC 8 link `macaroni-image:<id>:<row>`, `CSI D`, LF (scrolling at the bottom is what reserves the room), and finally `CSI <cols> C` so the cursor ends right of the block's last row, as iTerm2 leaves it. Like prompt marks, the anchors live in the cells: they scroll, reflow and get erased with the text, and `hyperlink_at` hides them (`IMAGE_PREFIX`).
- `AlacrittyEmulator::add_image` keeps the `Arc<InlineImage>` by id (ids are unique across sessions, from a static counter) and drops the oldest past 64 MiB together. `images_between` scans the visible rows for anchors and lists each image once in `Snapshot::images`, with its top row (negative when it starts above the view) and column.
- ConPTY passes OSC 1337 through in order and the image shows, but ConPTY's own buffer doesn't know the block's rows exist; a program that repaints with absolute positions could draw over it.

## Synchronized output

vte buffers a mode 2026 update but only checks its timeout when more bytes arrive. The reader sends the deadline to `pty-sync`, which calls `Emulator::flush_sync` and wakes the UI if the update is still open, so a program that never closes it can't freeze the screen.

## Keys

Shift+Enter sends `ESC CR` (Claude Code's newline). Ctrl+letter uses the key, not the platform text. Ctrl+Alt with printable text is AltGr and sends the text. `encode_paste` brackets only when the program enabled it.

## Shells and environment

- Default: login shell (`new_default_prog`) on macOS/Linux. Windows prefers Git Bash (`bash.exe --login -i`), then `pwsh.exe`, then `powershell.exe`. Never pick `C:\Windows\System32\bash.exe` (WSL's launcher).
- Bash gets `CHERE_INVOKING=1` (else Git Bash's `/etc/profile` jumps to `$HOME`). Prompt integration for bash and zsh: see Prompt marks. On Windows a `PROMPT_COMMAND` emits OSC 7 as a Windows path (`/c/x` becomes `C:/x` with builtins only, since forking per prompt is slow under MSYS; other mounts like `/tmp` go through `pwd -W`, the one fork; the app, `git -C` and restored tabs can't use MSYS paths) and drops the first prompt's leading newline; a `.bashrc` that replaces `PROMPT_COMMAND` disables both.
- SSH profiles (`[[ssh]]`) are sessions with `Source::Ssh` (see SSH below). The app tracks them in `App.ssh_tabs` (session to profile name) and `spawn` turns the profile into an `SshTarget` (`update::menus::ssh_target`). When the session exits non-zero (255 for a failed or dropped connection) the tab stays: `Session::print` writes a dimmed "disconnected" line into the emulator, the session joins `App.disconnected`, and Enter there connects the profile again in the same tab and `SessionId` (`App::reconnect`). The remote shell's exit 0 closes the tab as usual.
- Every session gets `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=macaroni`, plus `SessionConfig::env` (the app adds `MACARONI_SOCKET`, `MACARONI_TAB_ID`; see [`control-api.md`](control-api.md)). Start directory is home unless `SessionConfig::cwd` is set.

## SSH (`ssh/`)

macaroni's own client, on russh (with `ring`, so no C toolchain), not the `ssh` program. `ssh::open` returns the session's ends at once; a task on `RUNTIME` (a two-thread tokio runtime) connects and then pumps the channel.

- **Everything `ssh` would print or ask happens in the tab** (`prompt::Term`): progress lines (dimmed), the unknown host question, passphrases, passwords and keyboard-interactive prompts, errors (red). `Term::ask` reads what's typed until Enter (hidden unless echoing), skips escape sequences, and keeps any typing past the answer as typeahead for the shell. Ctrl+C or Ctrl+D cancels, which ends the session with 255.
- **Where to connect** is the target combined with `~/.ssh/config` (`config::resolve`): `Host` patterns, first value wins, `Include` followed, `Match` skipped; `HostName`, `User`, `Port`, `IdentityFile`, `ProxyJump`. A profile's own fields win, as `ssh`'s command-line options do. The default user is the local one, the default keys `id_ed25519`, `id_ecdsa`, `id_rsa`.
- **Connections are shared** (`POOL`, keyed by user, address, port and jump hosts): a second tab to the same place opens a channel on the live connection without signing in. Sessions hold the `Arc<Connection>`; the pool keeps a `Weak`, so the connection closes with its last channel. Each slot has an async lock held while connecting, so tabs opening together sign in once.
- **ProxyJump**: the last jump host's connection (made the same way, pooled, through the ones before it) opens a `direct-tcpip` channel, and the handshake runs over `channel.into_stream()`. `Connection._via` keeps the jump open.
- **Host keys** (`Client::check_server_key`, in russh's handler): known in `~/.ssh/known_hosts` passes; unknown asks (recorded on yes, creating `~/.ssh` if needed); changed refuses with OpenSSH's warning. The handler can't read the terminal, so it sends a `HostKeyNote` to `handshake`, which runs the handshake and answers notes from the tab in one `select!`. Later key exchanges on the connection must show the key accepted first.
- **Signing in** (`authenticate`): the agent's keys (Windows: OpenSSH's `\\.\pipe\openssh-ssh-agent`, then Pageant; elsewhere `SSH_AUTH_SOCK`), each identity file (asking up to 3 times for an encrypted one's passphrase), keyboard-interactive, then up to 3 passwords. RSA keys sign with the server's best supported hash.
- **The shell**: `SshTarget::command` (a profile's `tmux`) runs instead of the login shell with an `exec` request. Otherwise: a pty request (`xterm-256color`, the session's size), `LANG`/`LC_*` and `COLORTERM`/`TERM_PROGRAM` as env requests (servers drop what `AcceptEnv` doesn't list), then `shell`. Resizes become `window-change`. The exit code is the remote's `exit-status`; a channel closed without one is 255.
- **Ending**: the session's reader and sync threads hold the writer, so the input channel never closes by itself; `SshControl`'s drop fires a oneshot that makes the task close the channel.

## Windows: bundled ConPTY

`vendor/conpty/{x64,arm64}/` holds `conpty.dll` and `OpenConsole.exe` (update steps in `vendor/conpty/README.md`). `crates/app/build.rs` copies them next to the executable; portable-pty loads `conpty.dll` by bare name, so the app directory wins. The session's console host child is `OpenConsole.exe` when the bundled one is used. It is required: with the inbox ConPTY, MSYS2 programs (vim, less) don't redraw after resize.

## Traps

- **SSH tests use a scratch home** (`config::TEST_HOME`, test builds only): `home()` would otherwise read and write the real `~/.ssh/known_hosts`. Setting `HOME` instead isn't an option (`set_var` is unsafe, and the crate denies unsafe code).

- **Answer `ESC[6n` or the inbox ConPTY hangs.** portable-pty uses `PSEUDOCONSOLE_INHERIT_CURSOR`, so ConPTY asks for the cursor position at startup and outputs nothing until answered.
- **Don't detect exit by reader EOF.** ConPTY keeps the pipe open after the child exits; exit comes from `pty-waiter`. Output can arrive after `Exited`.
- ConPTY swallows OSC 10/11 queries, so color queries are only answered on macOS. Size queries work on both.
- ConPTY enables win32-input-mode (`ESC[?9001h`); alacritty ignores it, so input stays plain VT.

## Tests and benchmarks

- `cargo test -p macaroni-term`: emulator tests feed bytes and check the grid (inline `insta` snapshots), encoding tests, `lines` and `images` unit tests, and `session::tests::real_shell_round_trip` (real shell through the platform PTY, with a grace period for late output, and a trigger that must fire on the echoed line).
- `cargo bench -p macaroni-term` (`benches/emulator.rs`, criterion): parse throughput with and without the sniffer, and snapshot cost.
