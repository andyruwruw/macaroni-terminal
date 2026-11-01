# Contributing to macaroni

Thanks for helping out. This page covers how to build, what has to pass before a change lands, and the standards the code and docs are held to.

## Setup

- Install [rustup](https://rustup.rs). `rust-toolchain.toml` pins the toolchain (with rustfmt and clippy), and it installs itself on the first build.
- Windows: Visual Studio Build Tools with "Desktop development with C++", plus CMake and Ninja on `PATH`. Visual Studio ships both under `Common7\IDE\CommonExtensions\Microsoft\CMake`. The first build downloads Chromium (CEF, about 170 MB) into `target/cef`; see [`references/browser.md`](references/browser.md) if it fails.
- macOS: Xcode Command Line Tools.

`cargo xtask dev` rebuilds and restarts the app on every save, with its own settings folder and socket so it runs beside an everyday macaroni. `./install` installs a release build as your everyday macaroni.

## Releasing

Bump `version` in the root `Cargo.toml` and move the changelog's Unreleased entries under the new version. When that reaches `main`, the release workflow builds the bundles on Windows and macOS and publishes them on GitHub Releases, tagged `v<version>`, with that changelog section; a version like `0.1.0-alpha.2` becomes a pre-release. Pushes to `main` that keep an already released version don't publish anything.

## Checks

Every change must pass these, as CI does on Windows and macOS:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
```

For changes to input, rendering, the PTY layer or anything else tests can't reach, try them by hand in `cargo xtask dev` on both platforms.

## Code standards

- **Crate boundaries**: `macaroni-core` is pure (no I/O, no UI) and every crate depends only in the directions [`references/architecture.md`](references/architecture.md) lists.
- **Unsafe**: `unsafe_code` is denied workspace-wide. The few modules that need FFI allow it locally, and every `unsafe` block has a `// SAFETY:` comment.
- **Layout and colors**: every size, color and spacing of the UI comes from `Chrome` (`c.s.*` for sizes, named colors for colors), so users can change it. A new look value means a new `[style.*]` setting or color, not a literal in the view.
- **Style**: match the surrounding code. Files use LF line endings. Don't use em dashes, in code or prose.

## Documentation standard

The bar is that a competent engineer, or an agent, can open any file cold and understand it.

- Every module starts with a `//!` doc: what it owns, its main types, and how it fits with its neighbors.
- Every item has a `///` doc, private ones included: functions, types, fields, variants and constants. Lints enforce this (`missing_docs`, `clippy::missing_docs_in_private_items`).
- Function docs start with a one-line summary. Add parameters, the return value, `# Panics` and side effects (spawning, writing config, touching the GPU) where they apply. Link types and functions with intra-doc links (``[`Command::SplitPane`]``).
- Comments inside functions say *why*: invariants, ordering that matters, platform traps and non-obvious math. Don't narrate what the next line obviously does.
- [`references/`](references/) holds a per-domain guide (mental model, invariants, traps, key files). When you change a domain, update its reference in the same change; a new domain gets a new reference and a row in [`CLAUDE.md`](CLAUDE.md).
- Record user-visible changes under "Unreleased" in [CHANGELOG.md](CHANGELOG.md).
- In markdown, write full-length lines. Don't hard-wrap paragraphs.

## Commits

Keep commits focused, with a message that says what changed and why. Run the checks before pushing.
