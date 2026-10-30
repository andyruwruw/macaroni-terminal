//! Draws a [`Snapshot`] with iced's text renderer: [`GridRenderer`] (the per-view row cache,
//! kept in the [`super::TerminalView`]'s state) and [`Metrics`] (font and cell size, measured
//! once per font change by the app).
//!
//! Each row is split into runs: consecutive printable ASCII cells with the same style are shaped
//! together (the primary monospace font keeps them on the grid), while every other glyph (CJK,
//! emoji, symbols from fallback fonts) is its own run placed at its cell, so fallback glyphs can
//! never push the rest of the row off the grid. Box-drawing and block characters aren't text at
//! all: [`super::box_drawing`] turns them into rectangles. Row layouts are cached by content
//! hash, not by position, so a line that scrolled up reuses its layout.
//!
//! Draw order within the view's layer: backgrounds, decorations and boxes (quads), then text
//! (iced draws a layer's quads before its text whatever the call order). The cursor and IME
//! preedit go in a layer of their own on top, so the block cursor covers the glyph under it.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use iced::advanced::renderer::{Quad, Renderer as _};
use iced::advanced::text::{self, Paragraph as _, Renderer as _, Text};
use iced::{Border, Color, Font, Pixels, Point, Rectangle, Size, font};
use macaroni_term::{Cell, CellFlags, Color as TermColor, CursorShape, Rgb, Snapshot};

use super::box_drawing;
use crate::fonts;
use crate::theme::{Palette, to_iced};

/// The renderer's shaped, cacheable text block.
type Paragraph = <iced::Renderer as text::Renderer>::Paragraph;

/// How many cells wide the sample is that measures a cell, so one glyph's rounding averages out.
const MEASURE_CELLS: usize = 100;
/// Font size per pixel of underline and strikeout thickness.
const FONT_SIZE_PER_LINE_PX: f32 = 14.0;
/// How opaque dim (SGR 2) text is.
const DIM_ALPHA: f32 = 0.66;
/// The ANSI palette entry (blue) that underlines the link under the pointer.
const LINK_COLOR: usize = 4;
/// Thickness of the underline and beam cursors.
const CURSOR_BAR: f32 = 2.0;
/// How far above the cell's bottom the IME preedit's underline sits.
const PREEDIT_UNDERLINE_INSET: f32 = 2.0;

/// The font and cell size every view draws with. Built by [`Metrics::measure`] on start, font
/// changes and zoom; a change clears every [`GridRenderer`]'s cache.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// The terminal font (bold and italic are picked per cell from its family).
    pub font: Font,
    /// Font size, logical pixels.
    pub font_size: f32,
    /// One cell's advance, logical pixels; fractional, so long rows don't drift.
    pub cell_width: f32,
    /// One row's height, logical pixels, rounded to a whole pixel.
    pub cell_height: f32,
    /// Shape ASCII runs with full shaping so the font's programming ligatures apply.
    pub ligatures: bool,
    /// The row height as a multiple of `font_size` (`[font] line_height`).
    pub line_height: f32,
}

impl Metrics {
    /// Measures `font` at `font_size`: the cell is a hundredth of the width of 100 `M`s (one
    /// glyph's advance can round), and the row height is `line_height` times the size.
    pub fn measure(font: Font, font_size: f32, ligatures: bool, line_height: f32) -> Self {
        let sample = "M".repeat(MEASURE_CELLS);
        let cell_height = (font_size * line_height).round();
        let p = paragraph(&sample, font, font_size, cell_height, text::Shaping::Basic);
        Self {
            font,
            font_size,
            cell_width: p.min_width() / MEASURE_CELLS as f32,
            cell_height,
            ligatures,
            line_height,
        }
    }

    /// How ASCII runs are shaped: `Basic` keeps ligatures off, `Advanced` lets them apply.
    fn run_shaping(&self) -> text::Shaping {
        if self.ligatures {
            text::Shaping::Advanced
        } else {
            text::Shaping::Basic
        }
    }
}

/// Shapes `content` on one line, `line_height` tall, unbounded in width (rows never wrap). The
/// result can be cached and drawn with `fill_paragraph`.
fn paragraph(
    content: &str,
    font: Font,
    size: f32,
    line_height: f32,
    shaping: text::Shaping,
) -> Paragraph {
    Paragraph::with_text(Text {
        content,
        bounds: Size::new(f32::INFINITY, line_height),
        size: Pixels(size),
        line_height: text::LineHeight::Absolute(Pixels(line_height)),
        font,
        align_x: text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping,
        wrapping: text::Wrapping::None,
    })
}

/// For one-off draws: iced holds paragraphs passed to `fill_paragraph` weakly, so a temporary
/// paragraph is gone before the frame renders. `fill_text` owns its content instead.
fn owned_text(content: &str, font: Font, metrics: Metrics) -> Text<String> {
    Text {
        content: content.to_owned(),
        bounds: Size::new(f32::INFINITY, metrics.cell_height),
        size: Pixels(metrics.font_size),
        line_height: text::LineHeight::Absolute(Pixels(metrics.cell_height)),
        font,
        align_x: text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: text::Shaping::Advanced,
        wrapping: text::Wrapping::None,
    }
}

/// The font for one cell: the bundled symbols font for Private Use Area characters (Windows icon
/// fonts claim the same code points, so fallback would pick wrong glyphs), else `base` in the
/// cell's weight and style.
fn font_for(base: Font, ch: char, flags: CellFlags) -> Font {
    if fonts::is_private_use(ch) {
        return fonts::SYMBOLS;
    }
    Font {
        weight: if flags.contains(CellFlags::BOLD) {
            font::Weight::Bold
        } else {
            font::Weight::Normal
        },
        style: if flags.contains(CellFlags::ITALIC) {
            font::Style::Italic
        } else {
            font::Style::Normal
        },
        ..base
    }
}

/// One shaped piece of a row: an ASCII run or a single other glyph.
struct Run {
    /// The first column it covers.
    col: usize,
    /// How many columns it covers.
    span: usize,
    /// Its shaped text, kept alive by the cache (`fill_paragraph` holds it weakly).
    paragraph: Paragraph,
    /// Text color.
    color: Color,
    /// Centered in its span instead of left-aligned: single glyphs from fallback fonts, whose
    /// advance needn't match the cell.
    center: bool,
}

/// A line drawn through or under cells.
#[derive(Clone, Copy, PartialEq)]
enum Decoration {
    /// Near the cell's bottom.
    Underline,
    /// Through the middle of the cell.
    Strikeout,
}

/// Everything needed to draw one row, in columns, built once per distinct row content by
/// [`layout_row`] and positioned at draw time.
struct RowLayout {
    /// Background spans as (start column, end column exclusive, color), only where the
    /// background isn't the default, merged when adjacent cells match.
    backgrounds: Vec<(usize, usize, Color)>,
    /// Underline and strikeout spans, merged like backgrounds.
    decorations: Vec<(usize, usize, Color, Decoration)>,
    /// Box-drawing and block characters drawn as rectangles: column, character, color.
    boxes: Vec<(usize, char, Color)>,
    /// The row's text.
    runs: Vec<Run>,
}

/// What [`GridRenderer::draw`] needs for one frame, gathered by the view.
pub struct Frame<'a> {
    /// The screen to draw.
    pub snapshot: &'a Snapshot,
    /// Its colors.
    pub palette: &'a Palette,
    /// Font and cell size.
    pub metrics: Metrics,
    /// Where the grid's top-left cell starts, in window coordinates (inside the padding).
    pub origin: Point,
    /// Skip painting the default background.
    pub transparent: bool,
    /// Whether the view has keyboard focus and the window is focused; otherwise the cursor is a
    /// hollow block.
    pub focused: bool,
    /// The IME's composition to draw at the cursor; `None` when not composing (never empty).
    pub preedit: Option<&'a str>,
    /// The link under the pointer while the open-link modifier is held: row, first and last column.
    pub link: Option<(usize, usize, usize)>,
}

/// A view's renderer and row layout cache. Lives in the terminal widget's state.
#[derive(Default)]
pub struct GridRenderer {
    /// The metrics the cached layouts were built with; a change clears the cache.
    metrics: Option<Metrics>,
    /// Layouts used in the last frame, by row content hash. Rebuilt every frame from the previous
    /// one, so layouts for rows no longer on screen are dropped.
    rows: HashMap<u64, RowLayout>,
}

impl GridRenderer {
    /// Draws `frame` clipped to `clip` (the view's bounds), reusing cached row layouts and
    /// building the rest. Afterwards the cache holds exactly this frame's rows.
    pub fn draw(&mut self, renderer: &mut iced::Renderer, frame: &Frame<'_>, clip: Rectangle) {
        let Frame {
            snapshot,
            palette,
            metrics,
            origin,
            transparent,
            ..
        } = *frame;
        let overrides = snapshot.color_overrides.as_slice();

        if self.metrics != Some(metrics) {
            self.metrics = Some(metrics);
            self.rows.clear();
        }

        if !transparent {
            fill(
                renderer,
                clip,
                to_iced(palette.resolve(TermColor::Background, overrides), 1.0),
            );
        }

        // Rows move from last frame's map into this one, so the cache never outgrows the
        // screen; identical rows (blank lines) share one layout.
        let mut previous = std::mem::take(&mut self.rows);
        let current = &mut self.rows;
        for (i, cells) in snapshot.rows.iter().enumerate() {
            let key = row_key(cells, overrides);
            let row = current.entry(key).or_insert_with(|| {
                previous
                    .remove(&key)
                    .unwrap_or_else(|| layout_row(cells, palette, overrides, metrics))
            });
            let y = origin.y + i as f32 * metrics.cell_height;
            row.draw(renderer, origin.x, y, metrics, clip);
        }

        if let Some(link) = frame.link {
            draw_link(renderer, frame, link);
        }
        draw_cursor(renderer, frame, clip);
    }
}

/// What a row's layout is cached by: its cells and the palette overrides that color them.
fn row_key(cells: &[Cell], overrides: &[(u16, Rgb)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    cells.hash(&mut hasher);
    overrides.hash(&mut hasher);
    hasher.finish()
}

impl RowLayout {
    /// Draws the row with its left edge at `x0` and its top at `y`.
    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        x0: f32,
        y: f32,
        metrics: Metrics,
        clip: Rectangle,
    ) {
        let (cw, ch) = (metrics.cell_width, metrics.cell_height);
        let x_of = |col: usize| x0 + col as f32 * cw;
        for &(start, end, color) in &self.backgrounds {
            fill(
                renderer,
                Rectangle::new(
                    Point::new(x_of(start), y),
                    Size::new((end - start) as f32 * cw, ch),
                ),
                color,
            );
        }
        for &(start, end, color, kind) in &self.decorations {
            let thickness = line_thickness(metrics);
            let dy = match kind {
                Decoration::Underline => ch - thickness * 2.0,
                Decoration::Strikeout => (ch / 2.0).round(),
            };
            fill(
                renderer,
                Rectangle::new(
                    Point::new(x_of(start), y + dy),
                    Size::new((end - start) as f32 * cw, thickness),
                ),
                color,
            );
        }
        for &(col, glyph, color) in &self.boxes {
            draw_box(renderer, glyph, cell_rect(x_of(col), y, cw, ch), color);
        }
        for run in &self.runs {
            let mut x = x_of(run.col);
            if run.center {
                let slack = run.span as f32 * cw - run.paragraph.min_width();
                x += (slack / 2.0).max(0.0);
            }
            renderer.fill_paragraph(&run.paragraph, Point::new(x, y), run.color, clip);
        }
    }
}

/// How thick underlines and strikeouts are at the font's size.
fn line_thickness(metrics: Metrics) -> f32 {
    (metrics.font_size / FONT_SIZE_PER_LINE_PX).round().max(1.0)
}

/// Underlines the link under the pointer: its row, first and last column.
fn draw_link(
    renderer: &mut iced::Renderer,
    frame: &Frame<'_>,
    (row, start, end): (usize, usize, usize),
) {
    let Frame {
        palette,
        metrics,
        origin,
        ..
    } = *frame;
    let (cw, ch) = (metrics.cell_width, metrics.cell_height);
    let thickness = line_thickness(metrics);
    fill(
        renderer,
        Rectangle::new(
            Point::new(
                origin.x + start as f32 * cw,
                origin.y + (row + 1) as f32 * ch - thickness * 2.0,
            ),
            Size::new((end + 1 - start) as f32 * cw, thickness),
        ),
        to_iced(palette.ansi[LINK_COLOR], 1.0),
    );
}

/// A run of printable ASCII cells waiting to be shaped together: first column, text, color and
/// font.
type PendingRun = (usize, String, Color, Font);

/// Splits one row's cells into backgrounds, decorations, boxes and text runs (see the module
/// docs).
fn layout_row(
    cells: &[Cell],
    palette: &Palette,
    overrides: &[(u16, Rgb)],
    metrics: Metrics,
) -> RowLayout {
    let mut row = RowLayout {
        backgrounds: Vec::new(),
        decorations: Vec::new(),
        boxes: Vec::new(),
        runs: Vec::new(),
    };
    let mut pending: Option<PendingRun> = None;

    for (col, cell) in cells.iter().enumerate() {
        // The second half of a wide character; the wide cell before it covers both.
        if cell.flags.contains(CellFlags::WIDE_SPACER) {
            continue;
        }
        let (fg, bg) = cell_colors(cell, palette, overrides);
        let span = if cell.flags.contains(CellFlags::WIDE) {
            2
        } else {
            1
        };

        if let Some(bg) = bg {
            extend_or_push(&mut row.backgrounds, col, col + span, bg);
        }
        push_decorations(&mut row.decorations, cell.flags, col, span, fg);

        if cell.flags.contains(CellFlags::HIDDEN) || (cell.ch == ' ' && cell.zerowidth.is_empty()) {
            // Blanks join a pending run so the text after them keeps its column; trailing ones
            // are trimmed when the run is flushed.
            if let Some((_, text, _, _)) = &mut pending {
                text.push(' ');
            }
            continue;
        }
        if box_drawing::is_procedural(cell.ch) {
            flush_run(&mut pending, &mut row.runs, metrics);
            row.boxes.push((col, cell.ch, fg));
            continue;
        }

        let font = font_for(metrics.font, cell.ch, cell.flags);
        if cell.ch.is_ascii_graphic() && cell.zerowidth.is_empty() {
            match &mut pending {
                Some((_, text, color, f)) if *color == fg && *f == font => text.push(cell.ch),
                _ => {
                    flush_run(&mut pending, &mut row.runs, metrics);
                    pending = Some((col, cell.ch.to_string(), fg, font));
                }
            }
        } else {
            flush_run(&mut pending, &mut row.runs, metrics);
            row.runs.push(Run {
                col,
                span,
                paragraph: paragraph(
                    &cell_text(cell),
                    font,
                    metrics.font_size,
                    metrics.cell_height,
                    text::Shaping::Advanced,
                ),
                color: fg,
                center: true,
            });
        }
    }
    flush_run(&mut pending, &mut row.runs, metrics);
    row
}

/// Shapes the pending ASCII run, if it has more than blanks, and adds it to `runs`.
fn flush_run(pending: &mut Option<PendingRun>, runs: &mut Vec<Run>, metrics: Metrics) {
    if let Some((col, text, color, font)) = pending.take() {
        let text = text.trim_end();
        if !text.is_empty() {
            runs.push(Run {
                col,
                span: text.len(),
                paragraph: paragraph(
                    text,
                    font,
                    metrics.font_size,
                    metrics.cell_height,
                    metrics.run_shaping(),
                ),
                color,
                center: false,
            });
        }
    }
}

/// Adds the cell's underline and strikeout, extending the previous cell's when they match.
fn push_decorations(
    decorations: &mut Vec<(usize, usize, Color, Decoration)>,
    flags: CellFlags,
    col: usize,
    span: usize,
    fg: Color,
) {
    for (flag, kind) in [
        (CellFlags::UNDERLINE, Decoration::Underline),
        (CellFlags::STRIKEOUT, Decoration::Strikeout),
    ] {
        if flags.contains(flag) {
            match decorations.last_mut() {
                Some(last) if last.1 == col && last.2 == fg && last.3 == kind => {
                    last.1 = col + span;
                }
                _ => decorations.push((col, col + span, fg, kind)),
            }
        }
    }
}

/// The cell's character with any combining characters after it.
fn cell_text(cell: &Cell) -> String {
    std::iter::once(cell.ch)
        .chain(cell.zerowidth.iter().copied())
        .collect()
}

/// Adds the span `start..end` in `color`, extending the last span if it ends at `start` in the
/// same color.
fn extend_or_push(runs: &mut Vec<(usize, usize, Color)>, start: usize, end: usize, color: Color) {
    match runs.last_mut() {
        Some(last) if last.1 == start && last.2 == color => last.1 = end,
        _ => runs.push((start, end, color)),
    }
}

/// Foreground, plus a background only when it differs from the default (the cleared screen).
/// Inverse swaps them before that test, dim lowers the foreground's alpha, and selected cells
/// always get the palette's selection background.
fn cell_colors(cell: &Cell, palette: &Palette, overrides: &[(u16, Rgb)]) -> (Color, Option<Color>) {
    let mut fg = palette.resolve(cell.fg, overrides);
    let mut bg = palette.resolve(cell.bg, overrides);
    if cell.flags.contains(CellFlags::INVERSE) {
        std::mem::swap(&mut fg, &mut bg);
    }
    let alpha = if cell.flags.contains(CellFlags::DIM) {
        DIM_ALPHA
    } else {
        1.0
    };
    if cell.flags.contains(CellFlags::SELECTED) {
        return (to_iced(fg, alpha), Some(to_iced(palette.selection, 1.0)));
    }
    let default_bg = palette.resolve(TermColor::Background, overrides);
    (
        to_iced(fg, alpha),
        (bg != default_bg).then(|| to_iced(bg, 1.0)),
    )
}

/// Draws the IME preedit at the cursor, then the cursor after it, in a layer above the rows.
/// Nothing when the cursor is hidden.
fn draw_cursor(renderer: &mut iced::Renderer, frame: &Frame<'_>, clip: Rectangle) {
    let Some(cursor) = frame.snapshot.cursor else {
        return;
    };
    let Frame {
        snapshot,
        palette,
        metrics,
        origin,
        ..
    } = *frame;
    let overrides = snapshot.color_overrides.as_slice();
    let (cw, ch) = (metrics.cell_width, metrics.cell_height);
    let cell = snapshot
        .rows
        .get(cursor.row)
        .and_then(|r| r.get(cursor.col));
    let mut x = origin.x + cursor.col as f32 * cw;
    let y = origin.y + cursor.row as f32 * ch;
    let fg = to_iced(palette.resolve(TermColor::Foreground, overrides), 1.0);
    let bg = to_iced(palette.resolve(TermColor::Background, overrides), 1.0);
    let cursor_color = to_iced(palette.cursor, 1.0);

    renderer.with_layer(clip, |renderer| {
        if let Some(preedit) = frame.preedit.filter(|p| !p.is_empty()) {
            x += draw_preedit(renderer, preedit, Point::new(x, y), metrics, fg, bg, clip);
        }

        let width = if cell.is_some_and(|c| c.flags.contains(CellFlags::WIDE)) {
            cw * 2.0
        } else {
            cw
        };
        let shape = if frame.focused {
            cursor.shape
        } else {
            CursorShape::HollowBlock
        };
        match shape {
            CursorShape::Block => {
                fill(renderer, cell_rect(x, y, width, ch), cursor_color);
                if let Some(cell) = cell.filter(|c| c.ch != ' ' && frame.preedit.is_none()) {
                    draw_under_block(renderer, cell, Point::new(x, y), metrics, bg, clip);
                }
            }
            CursorShape::HollowBlock => renderer.fill_quad(
                Quad {
                    bounds: cell_rect(x, y, width, ch),
                    border: Border {
                        color: cursor_color,
                        width: 1.0,
                        radius: 0.0.into(),
                    },
                    ..Quad::default()
                },
                Color::TRANSPARENT,
            ),
            CursorShape::Underline => fill(
                renderer,
                Rectangle::new(
                    Point::new(x, y + ch - CURSOR_BAR),
                    Size::new(width, CURSOR_BAR),
                ),
                cursor_color,
            ),
            CursorShape::Beam => fill(
                renderer,
                Rectangle::new(Point::new(x, y), Size::new(CURSOR_BAR, ch)),
                cursor_color,
            ),
        }
    });
}

/// Draws the IME's text being composed, underlined, at `at`, over a background-colored box
/// that hides the cells under it. Returns its width.
fn draw_preedit(
    renderer: &mut iced::Renderer,
    preedit: &str,
    at: Point,
    metrics: Metrics,
    fg: Color,
    bg: Color,
    clip: Rectangle,
) -> f32 {
    let ch = metrics.cell_height;
    let width = paragraph(
        preedit,
        metrics.font,
        metrics.font_size,
        ch,
        text::Shaping::Advanced,
    )
    .min_width();
    fill(renderer, cell_rect(at.x, at.y, width, ch), bg);
    renderer.fill_text(owned_text(preedit, metrics.font, metrics), at, fg, clip);
    fill(
        renderer,
        Rectangle::new(
            Point::new(at.x, at.y + ch - PREEDIT_UNDERLINE_INSET),
            Size::new(width, 1.0),
        ),
        fg,
    );
    width
}

/// Redraws the glyph under a block cursor in the background color, so it shows through.
fn draw_under_block(
    renderer: &mut iced::Renderer,
    cell: &Cell,
    at: Point,
    metrics: Metrics,
    bg: Color,
    clip: Rectangle,
) {
    if box_drawing::is_procedural(cell.ch) {
        let rect = cell_rect(at.x, at.y, metrics.cell_width, metrics.cell_height);
        draw_box(renderer, cell.ch, rect, bg);
    } else {
        renderer.fill_text(
            owned_text(
                &cell_text(cell),
                font_for(metrics.font, cell.ch, cell.flags),
                metrics,
            ),
            at,
            bg,
            clip,
        );
    }
}

/// A rectangle from its top-left corner and size.
fn cell_rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
    Rectangle::new(Point::new(x, y), Size::new(width, height))
}

/// Fills `bounds` with `color`, no border.
fn fill(renderer: &mut iced::Renderer, bounds: Rectangle, color: Color) {
    renderer.fill_quad(
        Quad {
            bounds,
            ..Quad::default()
        },
        color,
    );
}

/// Draws a box-drawing or block character into `cell`: a clipped rounded quad for the arcs,
/// else [`box_drawing::rects`] (whose alphas make the shades).
fn draw_box(renderer: &mut iced::Renderer, glyph: char, cell: Rectangle, color: Color) {
    if let Some(arc) = box_drawing::arc(glyph, cell) {
        // A layer is the only way to clip a quad to the cell.
        renderer.with_layer(cell, |renderer| {
            renderer.fill_quad(
                Quad {
                    bounds: arc.bounds,
                    border: Border {
                        color,
                        width: arc.thickness,
                        radius: arc.radius.into(),
                    },
                    ..Quad::default()
                },
                Color::TRANSPARENT,
            );
        });
        return;
    }
    for (rect, alpha) in box_drawing::rects(glyph, cell) {
        fill(
            renderer,
            rect,
            Color {
                a: color.a * alpha,
                ..color
            },
        );
    }
}
