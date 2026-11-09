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
- `term`: one `Session` per shell, local (`portable-pty`) or remote (its own SSH client on `russh` and tokio), with I/O threads, the `Emulator` over `alacritty_terminal`, key encoding and the OSC sniffer. Knows only `SessionId`, never tabs.
- `config`, `ipc`: standalone; see [`config.md`](config.md), [`control-api.md`](control-api.md).
- `browser`: `Browser` over two engines, the system web view (`wry`; Windows and macOS) and Chromium (CEF; Windows). See [`browser.md`](browser.md).
- `cli`: the `macaroni-cli` client.
- `app`: the iced application; executes `Effect`s (spawning sessions, OS notifications). Screen readers go through AccessKit (`accesskit_windows`, `accesskit_macos`); see [`accessibility.md`](accessibility.md).

Invariants: state changes only go through `Command` → reducer → `Effect`. Sessions live in a registry outside the layout, so layout changes never kill a shell. If `core` seems to need `term`, `config` or `iced`, the logic belongs in `app` as an `Effect` handler.

## Files

- `Cargo.toml`: members, `[workspace.package]`, `[workspace.dependencies]` (every shared version), `[workspace.lints]`.
- `rust-toolchain.toml`: pinned compiler plus rustfmt and clippy.
- `.github/workflows/ci.yml`: fmt, clippy, test and `cargo doc` (warnings denied) on Windows and macOS.
- `.github/workflows/release.yml`: on a push to `main` whose `Cargo.toml` version has no `v<version>` tag yet, runs `cargo xtask bundle` on Windows x64 and macOS arm64, zips the results (the `.app` ad-hoc signed) and publishes a GitHub release, which creates the tag, with that version's CHANGELOG section. Pre-release versions are marked as such.
- `install` (sh) and `install.cmd`: one-line wrappers for `cargo xtask install`, so `./install` works in a Unix shell, PowerShell and cmd alike (PowerShell resolves `.\install` to `install.cmd`; a `.ps1` beside it would win and hit the execution policy).
- `.cargo/config.toml`: the `cargo xtask` alias and `CEF_PATH = target/cef`.
- `xtask/src/main.rs` (`bundle`, which leaves Chromium's runtime out: the app downloads it; see [`browser.md`](browser.md)), `xtask/src/dev.rs` (`dev`), `xtask/src/install.rs` (`install`).
- `crates/app/src/crash.rs`: once `main` knows it isn't a Chromium helper, it sets a panic hook that writes each panic (thread, message, place, backtrace) and the trail to `crash.log` in the local data folder (`%LOCALAPPDATA%\macaroni` on Windows, `~/Library/Application Support/macaroni` on macOS). The first panic of a run replaces the file, so it holds the latest crash. The trail is the last 300 messages `App::update` handled (`note_message`, plus `note` for anything else), each with seconds since start; frames, ticks, hovers, wakeups, keys and typed input are skipped, and `redact` blanks every quoted string and number list in the rest (typed text, paths, titles, bytes). A message carrying something private that isn't a string or bytes needs the same care. A main-thread panic exits without a Windows Error Reporting entry and the bundled app has no console, so this is the only record; check it first when the app vanished.
- `crates/app/build.rs`: embeds `macaroni.exe.manifest` and the icon, copies `vendor/conpty/<arch>/` next to the executable. See [`terminal-and-pty.md`](terminal-and-pty.md).
- `crates/app/assets/`: fonts (`include_bytes!`, see [`rendering-and-text.md`](rendering-and-text.md)), Lucide icons (`icons.rs`), app icon. `docs/logo.svg` is the logo source.
- `crates/term/benches/` (criterion); `crates/*/examples/`: manual probes.

## Conventions

- Declare each dependency version once in `[workspace.dependencies]`; crates use `dep.workspace = true`, internal crates too.
- Every crate has `[lints] workspace = true`. `unsafe_code` is denied; allow it narrowly at the site (`#[allow(unsafe_code)]`) with a reason. `missing_docs` and `missing_docs_in_private_items` warn, and CI denies warnings, so every item needs a doc.
- iced is pinned to one minor version; its minor releases break.
- Never hard-code the workspace version; `xtask` reads it (e.g. for `Info.plist`).

## Traps

- CEF's C++ wrapper builds with CMake and Ninja, both on `PATH`. `cargo xtask` (so `./install` too) uses whichever is on `PATH`, else Visual Studio's copy on Windows (via `vswhere`), else downloads the official portable release into `target/tools` (`xtask/src/tools.rs`: pinned versions and SHA-256s, the system's `curl` and `tar`). Only if that fails does it stop, before building, with install instructions. Plain `cargo build` does none of this. To bump a tool, change its URL, folder and checksum together; `cargo test -p xtask -- --ignored` downloads and runs both.
- `cargo xtask dev` runs the app from a copy in `target/dev/run` because Windows can't overwrite a running `.exe`. It stops the old instance with a normal close request (`taskkill` without `/F`) so the session is saved, and defaults `MACARONI_CONFIG_DIR` and `MACARONI_SOCKET` to dev values so it runs beside an everyday instance.
- `target` only grows: cargo never deletes the builds a dependency, feature or compiler change replaces. `[profile.dev]` keeps line tables only for workspace crates and no debug info for dependencies (full debug info on Windows made `target/debug` over 100 GB), and before building, `cargo xtask dev` and `bundle` run `cargo clean --profile` on their profile's folder (`target/debug`, `target/release`) once it's past `PROFILE_CAP` (40 GB; `trim` in `xtask/src/main.rs`), so that build starts over. `target/cef` and `target/tools` sit outside those folders and survive. Plain `cargo` commands aren't trimmed, and a throwaway `CARGO_TARGET_DIR` is a whole second copy: delete it after use.
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
