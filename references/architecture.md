# Architecture

## Mental model

One Cargo workspace, dependencies flowing one way:

```
app (macaroni, bin) ──► core    pure model + Command reducer
                    ──► term    sessions: PTY, I/O thread, emulator
                    ──► config  TOML settings, themes, keybindings
                    ──► ipc     control protocol + socket transport
                    ──► browser browser pages: system web view or CEF
cli (macaroni-cli)  ──► ipc
```

- `core`: Workspace › SplitNode › Pane › Tab, `Command`, the reducer, `Effect`. No UI, PTY or other project crate, so it tests without a window or shell. See [`model-and-commands.md`](model-and-commands.md).
- `term`: one `Session` per shell (`portable-pty`, I/O thread, `Emulator` over `alacritty_terminal`, key encoding, OSC sniffer). Knows only `SessionId`, never tabs.
- `config`, `ipc`: standalone; see [`config.md`](config.md), [`control-api.md`](control-api.md).
- `browser`: `Browser` over two engines, the system web view (`wry`; Windows and macOS) and Chromium (CEF; Windows). See [`browser.md`](browser.md).
- `cli`: the `macaroni-cli` client.
- `app`: the iced application; executes `Effect`s (spawning sessions, OS notifications).

Invariants: state changes only go through `Command` → reducer → `Effect`. Sessions live in a registry outside the layout, so layout changes never kill a shell. If `core` seems to need `term`, `config` or `iced`, the logic belongs in `app` as an `Effect` handler.

## Files

- `Cargo.toml`: members, `[workspace.package]`, `[workspace.dependencies]` (every shared version), `[workspace.lints]`.
- `rust-toolchain.toml`: pinned compiler plus rustfmt and clippy.
- `.github/workflows/ci.yml`: fmt, clippy, test and `cargo doc` (warnings denied) on Windows and macOS.
- `.github/workflows/release.yml`: on a push to `main` whose `Cargo.toml` version has no `v<version>` tag yet, runs `cargo xtask bundle` on Windows x64 and macOS arm64, zips the results (the `.app` ad-hoc signed) and publishes a GitHub release, which creates the tag, with that version's CHANGELOG section. Pre-release versions are marked as such.
- `install` (sh) and `install.cmd`: one-line wrappers for `cargo xtask install`, so `./install` works in a Unix shell, PowerShell and cmd alike (PowerShell resolves `.\install` to `install.cmd`; a `.ps1` beside it would win and hit the execution policy).
- `.cargo/config.toml`: the `cargo xtask` alias and `CEF_PATH = target/cef`.
- `xtask/src/main.rs` (`bundle`, which leaves Chromium's runtime out: the app downloads it; see [`browser.md`](browser.md)), `xtask/src/dev.rs` (`dev`), `xtask/src/install.rs` (`install`).
- `crates/app/build.rs`: embeds `macaroni.exe.manifest` and the icon, copies `vendor/conpty/<arch>/` next to the executable. See [`terminal-and-pty.md`](terminal-and-pty.md).
- `crates/app/assets/`: fonts (`include_bytes!`, see [`rendering-and-text.md`](rendering-and-text.md)), Lucide icons (`icons.rs`), app icon. `docs/logo.svg` is the logo source.
- `crates/term/benches/` (criterion); `crates/*/examples/`: manual probes.

## Conventions

- Declare each dependency version once in `[workspace.dependencies]`; crates use `dep.workspace = true`, internal crates too.
- Every crate has `[lints] workspace = true`. `unsafe_code` is denied; allow it narrowly at the site (`#[allow(unsafe_code)]`) with a reason. `missing_docs` and `missing_docs_in_private_items` warn, and CI denies warnings, so every item needs a doc.
- iced is pinned to one minor version; its minor releases break.
- Never hard-code the workspace version; `xtask` reads it (e.g. for `Info.plist`).

## Traps

- CEF's C++ wrapper builds with CMake and Ninja. On Windows Ninja must be on `PATH`; `cargo xtask` adds Visual Studio's Ninja itself (via `vswhere`), plain `cargo build` doesn't.
- `cargo xtask dev` runs the app from a copy in `target/dev/run` because Windows can't overwrite a running `.exe`. It stops the old instance with a normal close request (`taskkill` without `/F`) so the session is saved, and defaults `MACARONI_CONFIG_DIR` and `MACARONI_SOCKET` to dev values so it runs beside an everyday instance.
- macOS notifications need the `.app` from `cargo xtask bundle`; its bundle id (`BUNDLE_ID`) must stay stable.
- `cargo xtask install` run inside macaroni (`MACARONI_SOCKET` set) would die with the terminal it's closing, so after the build it restarts itself detached (`--detached`): a new console on Windows, its own process group with a log in the temp folder on macOS. It closes the installed app like `dev` does (a close request, then a kill after a timeout) and waits for every process from the install folder, Chromium's helpers included.
- Windows release builds use the GUI subsystem (no console), so the CLI is a separate binary.

## Commands

```sh
cargo run -p macaroni [-- <program> [args...]]
cargo xtask dev      # rebuild and restart on every save
cargo xtask bundle   # release package for this OS
./install            # cargo xtask install: bundle, install and start it
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
