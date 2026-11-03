# Rendering and text

## Mental model

`crates/app/src/terminal/` is a custom iced widget, `TerminalView`, that draws a `Snapshot` and turns input into `ViewEvent`s (`Resize`, `Write`, `Scroll`, `Focus`, `Select`, `Link`, `Action`, ...). The app owns the session and the latest snapshot; the widget owns only view state (render cache, IME preedit, focus, scroll remainder).

## Key files

- `mod.rs`: the widget. `update` routes to `on_window`, `on_key` (`intercept_key` for app shortcuts), `on_ime` and `on_mouse` (`on_press`, `on_move`, `on_reported_release`, `on_wheel`, `on_wheel_zoom`). Builders: `transparent`, `wheel_lines`, `copy_on_select`, `highlights`. Keys go through `macaroni_term::encode_key`, mouse reports through `encode_mouse`, using the snapshot's modes.
- `render.rs`: `Metrics` (`Metrics::measure`: cell size from the font, row height from `line_height`), `GridRenderer::draw`, `layout_row` (`flush_run`, `push_decorations`), `RowLayout::draw`, `draw_highlights`, `GridRenderer::draw_images`, `draw_cursor` (`draw_preedit`, `draw_under_block`). Sizes are named constants at the top.
- `box_drawing.rs`: U+2500 to U+259F as rectangles (`rects`, `is_procedural`), rounded corners via `arc`.
- `frame_stats.rs`: `MACARONI_FRAME_STATS=1` prints layout/record timings to stderr (run from a shell).
- `crates/app/src/theme.rs`: `Palette` (theme, xterm 256-color cube and gray ramp, overrides, `selection`) and `Chrome`.
- `crates/app/src/fonts.rs`: `load`, `resolve`, `set_fallback`, `fallback_for`, `PRIMARY`, `SYMBOLS`, `is_private_use`. Bundled fonts and licenses in `crates/app/assets/fonts/`.

## Invariants

- The widget computes its grid from bounds and `Metrics` every redraw and emits `Resize` on change. The app spawns a tab's session only after the first `Resize`, so the shell starts at its real size.
- Grid alignment: consecutive printable ASCII cells with the same color and font form one run, shaped `Shaping::Basic`, on a monospace font whose width defines the cell. Every other glyph is its own run at its own cell, centered, shaped `Shaping::Advanced` for fallback, so odd fallback metrics can't shift the row. Ligatures (`Metrics` carries the flag) switch ASCII runs to Advanced.
- Box and block characters are rectangles so lines join across cells regardless of font. `╭ ╮ ╯ ╰` are an oversized stroked rounded quad clipped to the cell with a layer so its edges sit on neighbouring `─`/`│`; each arc costs one layer.
- Row layouts are cached by a hash of cells plus palette overrides (`row_key`), not by position, so scrolled rows reuse layouts. Each frame builds a fresh map from the previous one (off-screen rows drop out); the cache resets when metrics change.
- Draw order: background quads (only non-default backgrounds), trigger highlights, decorations, boxes, text, then inline images in a layer of their own, then cursor and preedit in another. `transparent` (glass) skips the default background so the pane card shows through.
- Trigger highlights (`Frame::highlights`, from `App.triggers`, see [`config.md`](config.md)) are matched per visible row every frame, not cached with the row layout, so a config change needs no cache reset. Each row is turned into text with a byte-to-column map (wide spacers skipped); a match can't span a wrapped line.
- Inline images (`Snapshot::images`, see [`terminal-and-pty.md`](terminal-and-pty.md)) are fitted to their block of cells (aspect kept unless the image asked to stretch), so a font zoom refits them, and clipped to the grid inside the padding. `GridRenderer.images` keeps one `image::Handle` per image id, rebuilt each frame like the row cache: iced caches decoded images by handle id, and a fresh `Handle::from_bytes` every frame would decode and upload again.
- Mouse reporting: when the program enabled it and Shift isn't held, input goes to the program (motion only on reaching a new cell) instead of selection, links and scrollback. Shift restores local behaviour.
- IME: on every `RedrawRequested` while focused, the widget calls `shell.request_input_method` with `Purpose::Terminal`, the cursor cell rect and the preedit. Preedit is drawn inline and never sent to the PTY; `Commit` emits `Write`. Key presses are ignored while a non-empty preedit exists.
- `fonts::load()` runs first in `main`, synchronously into iced's global font system, so `Metrics::measure` sees the bundled fonts.

## Traps

- **`fill_paragraph` holds the paragraph weakly.** A `Paragraph` built and dropped inside `draw` renders nothing. Cached run paragraphs are fine; one-off draws (cursor character, preedit) use `fill_text` with `owned_text`.
- **Don't use the system "monospace" family.** With a Japanese language pack, Windows resolves it to a font that draws `\` as `¥`. Use `fonts::PRIMARY` (or the resolved `font.family`).
- **Private Use Area code points must use `fonts::SYMBOLS` explicitly.** Windows icon fonts share those code points, so fallback picks wrong glyphs.
- `[font] fallback`: `font_for` asks `fonts::fallback_for` for every non-ASCII cell, which answers from the cmaps (`as_swash().charmap()`) of the terminal font and each listed family, kept as cosmic-text fonts in a static. It comes before the PUA rule, so a listed Nerd Font beats the bundled symbols. Row layouts are cached by cell content, so `Metrics.fallback` (the list's generation) makes a new list clear the cache like a font change.
- iced's `Preedit` can be `Some` with empty content. Treat empty as not composing.
- iced 0.14 custom widgets need the `advanced` feature (set in `[workspace.dependencies]`).

## Checking it visually

`cargo run -p macaroni -- <program> [args...]` runs a program instead of the default shell (test patterns, TUIs like Git for Windows' `vim.exe`). `cargo run -p macaroni --example ime_probe` is a standalone IME and font fallback check.
