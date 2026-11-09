//! The emulator boundary: the [`Emulator`] trait the rest of the crate uses, and
//! [`AlacrittyEmulator`], its only implementation, over `alacritty_terminal`.
//!
//! [`Session`](crate::Session)'s reader thread calls [`Emulator::feed`] with PTY output (after
//! [`OscSniffer`](crate::OscSniffer) has looked at it), and [`Session::snapshot`] calls
//! [`Emulator::snapshot`] under the session lock. Everything crossing the boundary is converted
//! into [`crate::grid`] types here (`convert_cell`, `convert_color`, `modes`), so callers never
//! see alacritty's. Events alacritty raises become [`EmulatorEvent`]s through a callback.
//!
//! [`Session::snapshot`]: crate::Session::snapshot

use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll as AlacrittyScroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, NamedColor, Processor, Rgb as AnsiRgb};

use crate::grid::{
    Cell, CellFlags, CellSide, Color, Cursor, CursorShape, DefaultColors, GridPoint, GridSize,
    Modes, Rgb, Scroll, SearchDirection, SelectionKind, Snapshot,
};
use crate::images::{IMAGE_PREFIX, ImagePlacement, InlineImage};

/// Something the emulator raised while parsing output. Delivered synchronously from inside
/// [`Emulator::feed`], so on the reader thread and with the session lock held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmulatorEvent {
    /// Bytes the emulator must send back to the shell, such as replies to cursor position queries.
    PtyWrite(String),
    /// The program set the window title (OSC 0/2).
    Title(String),
    /// The program reset the title to the default.
    ResetTitle,
    /// A BEL character.
    Bell,
    /// The program asked to copy text to the clipboard (OSC 52).
    ClipboardStore(String),
}

/// The boundary between macaroni and the terminal emulation library.
///
/// Positions are [`GridPoint`]s in visible-screen coordinates; implementations convert them using
/// the current scroll offset and clamp them to the grid.
pub trait Emulator: Send {
    /// Parses PTY output and applies it to the grid. May raise [`EmulatorEvent`]s.
    fn feed(&mut self, bytes: &[u8]);
    /// Resizes the grid (reflowing as the implementation does). Does not touch the PTY.
    fn resize(&mut self, size: GridSize);
    /// Moves the view through the scrollback.
    fn scroll(&mut self, scroll: Scroll);
    /// Copies the visible screen, cursor, modes and palette overrides.
    fn snapshot(&self) -> Snapshot;
    /// The current terminal modes, without copying the screen.
    fn modes(&self) -> Modes;

    /// Starts a new selection at `at`, replacing any existing one.
    fn start_selection(&mut self, kind: SelectionKind, at: GridPoint, side: CellSide);
    /// Extends the current selection to `at`; does nothing without one.
    fn update_selection(&mut self, at: GridPoint, side: CellSide);
    /// Drops the selection.
    fn clear_selection(&mut self);
    /// The selected text, or `None` when nothing (or only an empty range) is selected.
    fn selection_text(&self) -> Option<String>;
    /// The OSC 8 hyperlink target of the cell, if a program attached one. Prompt marks' and
    /// image anchors' internal links (`MARK_PREFIX`, `IMAGE_PREFIX`) don't count.
    fn hyperlink_at(&self, at: GridPoint) -> Option<String>;
    /// Scrolls the view so the previous (`up`) or next prompt the shell marked (OSC 133) is the
    /// top row; false, leaving the view, when there's none that way.
    fn jump_to_prompt(&mut self, up: bool) -> bool;
    /// Every line's text, scrollback and screen, trailing blank lines dropped.
    fn all_text(&self) -> String;
    /// The grid's size, as last set.
    fn size(&self) -> GridSize;
    /// Keeps an inline image for the snapshot to list where its anchors show
    /// ([`InlineImage::anchors`], fed separately). The oldest images are dropped once they take
    /// more than `MAX_KEPT_IMAGE_BYTES` (64 MiB) together.
    fn add_image(&mut self, image: InlineImage);
    /// The output of the last command the shell marked: from its output mark to the next prompt
    /// (or the cursor), trailing blank lines dropped. `None` without one.
    fn last_output(&self) -> Option<String>;
    /// The command typed at the last prompt the shell marked: the text from the prompt's end
    /// (OSC 133 `B`) to the cursor, for reading when the shell says its output starts (`C`),
    /// before any output moves the cursor. `None` when nothing was typed, or no prompt is in the
    /// `MAX_COMMAND_ROWS` (50) rows above without a command's output in between.
    fn typed_command(&self) -> Option<String>;
    /// How many characters the cursor moves right (negative: left) to reach the screen cell
    /// `at`, when `at` and the cursor are both in the command being typed at the last marked
    /// prompt: from the prompt's end (OSC 133 `B`) through the rows it soft-wraps onto. A cell
    /// before the command counts as its start, one past its text as its end. `None` when
    /// either is outside it or no prompt is marked (see [`Emulator::typed_command`]).
    fn cursor_moves(&self, at: GridPoint) -> Option<i32>;
    /// Selects and scrolls to the next match of `query` (plain text, case-insensitive unless it
    /// has capitals), continuing from the previous match. After [`Emulator::reset_search`],
    /// [`SearchDirection::Older`] starts from the bottom of the screen and
    /// [`SearchDirection::Newer`] from the cursor. An empty query resets the search and clears
    /// the selection. Returns whether anything matched.
    fn search(&mut self, query: &str, direction: SearchDirection) -> bool;
    /// Forgets the previous match, so the next search starts over.
    fn reset_search(&mut self);

    /// When an unfinished synchronized update (mode 2026) must be shown anyway.
    fn sync_deadline(&self) -> Option<Instant>;
    /// Applies a synchronized update whose end never arrived.
    fn flush_sync(&mut self);
    /// Sets the colors color queries are answered with; without them they go unanswered.
    fn set_default_colors(&mut self, colors: DefaultColors);
    /// Sets the cursor's shape for while the program hasn't asked for one (DECSCUSR).
    fn set_default_cursor(&mut self, shape: CursorShape);
}

/// The callback [`AlacrittyEmulator`] reports [`EmulatorEvent`]s to.
type EventSink = Arc<dyn Fn(EmulatorEvent) + Send + Sync>;

/// alacritty's [`EventListener`], translating the events macaroni cares about into
/// [`EmulatorEvent`]s and dropping the rest.
#[derive(Clone)]
struct Listener {
    /// Where translated events go.
    sink: EventSink,
    /// Queries that need the terminal's state to answer, which the listener can't see. The
    /// emulator answers them once the parser hands control back (see `answer_queries`).
    queries: Arc<Mutex<Vec<Query>>>,
}

/// A query alacritty raised that [`AlacrittyEmulator`] answers itself.
enum Query {
    /// OSC 4, 10, 11 or 12: the color at this index of alacritty's color table, written back
    /// with the formatter.
    Color(usize, Arc<dyn Fn(AnsiRgb) -> String + Send + Sync>),
    /// `CSI 14t` / `CSI 18t`: the text area's size, written back with the formatter.
    TextAreaSize(Arc<dyn Fn(WindowSize) -> String + Send + Sync>),
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        // The rest is dropped: `Wakeup` and `ChildExit` have their own paths in `Session`, and
        // OSC 52 clipboard reads (`ClipboardLoad`) aren't supported, since any program could
        // read the clipboard that way.
        let event = match event {
            Event::ColorRequest(index, format) => return self.queue(Query::Color(index, format)),
            Event::TextAreaSizeRequest(format) => return self.queue(Query::TextAreaSize(format)),
            Event::PtyWrite(text) => EmulatorEvent::PtyWrite(text),
            Event::Title(title) => EmulatorEvent::Title(title),
            Event::ResetTitle => EmulatorEvent::ResetTitle,
            Event::Bell => EmulatorEvent::Bell,
            Event::ClipboardStore(_, text) => EmulatorEvent::ClipboardStore(text),
            _ => return,
        };
        (self.sink)(event);
    }
}

impl Listener {
    /// Keeps a query for the emulator to answer after the current `feed`.
    fn queue(&self, query: Query) {
        if let Ok(mut queries) = self.queries.lock() {
            queries.push(query);
        }
    }
}

/// The size handed to alacritty's `Term`: the grid with no scrollback of its own (alacritty adds
/// the history from its config).
struct Dims {
    /// Columns, at least 1.
    cols: usize,
    /// Screen rows, at least 1.
    rows: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

impl From<GridSize> for Dims {
    fn from(size: GridSize) -> Self {
        Self {
            cols: usize::from(size.cols.max(1)),
            rows: usize::from(size.rows.max(1)),
        }
    }
}

/// [`Emulator`] over `alacritty_terminal`'s `Term` and vte's `Processor`.
pub struct AlacrittyEmulator {
    /// The grid, scrollback, modes and selection.
    term: Term<Listener>,
    /// The escape sequence parser. It also buffers synchronized updates (mode 2026).
    parser: Processor,
    /// The match the last [`Emulator::search`] selected, where the next one continues from.
    last_match: Option<Match>,
    /// Queries the listener queued during parsing, answered by `answer_queries`.
    queries: Arc<Mutex<Vec<Query>>>,
    /// Where answers go, as [`EmulatorEvent::PtyWrite`].
    sink: EventSink,
    /// The grid size, for text area size queries.
    size: GridSize,
    /// What color queries are answered with; `None` leaves them unanswered.
    colors: Option<DefaultColors>,
    /// Lines of history, kept to pass back to alacritty with every config change.
    scrollback: usize,
    /// Inline images by id (oldest first), whatever their anchors' state: an image whose
    /// anchors were erased or trimmed stays until it's among the oldest past the limit.
    images: std::collections::BTreeMap<u64, Arc<InlineImage>>,
}

/// How many bytes of inline images (encoded) an emulator keeps.
pub const MAX_KEPT_IMAGE_BYTES: usize = 64 * 1024 * 1024;

impl AlacrittyEmulator {
    /// Creates an emulator of `size` keeping `scrollback` lines of history. `on_event` is called
    /// from inside [`Emulator::feed`] for each [`EmulatorEvent`].
    pub fn new(
        size: GridSize,
        scrollback: usize,
        on_event: impl Fn(EmulatorEvent) + Send + Sync + 'static,
    ) -> Self {
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        let sink: EventSink = Arc::new(on_event);
        let queries = Arc::new(Mutex::new(Vec::new()));
        let listener = Listener {
            sink: sink.clone(),
            queries: queries.clone(),
        };
        let term = Term::new(config, &Dims::from(size), listener);
        Self {
            term,
            parser: Processor::new(),
            last_match: None,
            queries,
            sink,
            size,
            colors: None,
            scrollback,
            images: std::collections::BTreeMap::new(),
        }
    }

    /// Answers the queries parsing raised, as [`EmulatorEvent::PtyWrite`]s.
    fn answer_queries(&mut self) {
        let queries = match self.queries.lock() {
            Ok(mut queries) => std::mem::take(&mut *queries),
            Err(_) => return,
        };
        for query in queries {
            let reply = match query {
                Query::Color(index, format) => {
                    let Some(color) = self.query_color(index) else {
                        continue;
                    };
                    format(color)
                }
                Query::TextAreaSize(format) => format(WindowSize {
                    num_lines: self.size.rows.max(1),
                    num_cols: self.size.cols.max(1),
                    cell_width: self.size.cell_width,
                    cell_height: self.size.cell_height,
                }),
            };
            (self.sink)(EmulatorEvent::PtyWrite(reply));
        }
    }

    /// The color at `index` of alacritty's color table (0 to 255 the palette, then the
    /// foreground, background and cursor): the one the program set, else the theme's. `None`
    /// for other indexes, or for an unset color without [`DefaultColors`].
    fn query_color(&self, index: usize) -> Option<AnsiRgb> {
        if let Some(set) = self.term.colors()[index] {
            return Some(set);
        }
        let colors = self.colors.as_ref()?;
        let rgb = match index {
            0..=255 => colors.palette[index],
            i if i == NamedColor::Foreground as usize => colors.foreground,
            i if i == NamedColor::Background as usize => colors.background,
            i if i == NamedColor::Cursor as usize => colors.cursor,
            _ => return None,
        };
        Some(AnsiRgb {
            r: rgb.r,
            g: rgb.g,
            b: rgb.b,
        })
    }

    /// Converts a visible-screen position into alacritty's grid coordinates, clamped to the
    /// screen. alacritty numbers lines from the top of the live screen, so scrollback lines are
    /// negative: subtracting the display offset maps "row 0 of what's shown" onto them.
    fn point(&self, at: GridPoint) -> Point {
        let rows = self.term.screen_lines().saturating_sub(1);
        let cols = self.term.columns().saturating_sub(1);
        let offset = self.term.grid().display_offset() as i32;
        Point::new(
            Line(at.row.min(rows) as i32 - offset),
            Column(at.col.min(cols)),
        )
    }

    /// Whether the cell at `point` carries a prompt mark link of `kind` (`"prompt"` or
    /// `"output"`).
    fn marked(&self, point: Point, kind: &str) -> bool {
        self.term.grid()[point].hyperlink().is_some_and(|link| {
            link.uri()
                .strip_prefix(MARK_PREFIX)
                .is_some_and(|k| k.starts_with(kind))
        })
    }

    /// Where the command typed at the last marked prompt starts: the cell after the prompt's
    /// end (OSC 133 `B`), looking up from the cursor at most [`MAX_COMMAND_ROWS`] rows. `None`
    /// without a marked prompt, or with a command's output in between.
    fn command_start(&self) -> Option<Point> {
        let grid = self.term.grid();
        let cursor = grid.cursor.point;
        let columns = self.term.columns();
        let top = Line(cursor.line.0 - MAX_COMMAND_ROWS).max(grid.topmost_line());
        let mut line = cursor.line;
        let prompt_end = loop {
            // Output in the way means the prompt above belongs to an older command, and this
            // one's prompt wasn't marked.
            if (0..columns).any(|c| self.marked(Point::new(line, Column(c)), "output")) {
                return None;
            }
            if let Some(column) = (0..columns)
                .rev()
                .find(|&c| self.marked(Point::new(line, Column(c)), "prompt"))
            {
                break Point::new(line, Column(column));
            }
            if line <= top {
                return None;
            }
            line = Line(line.0 - 1);
        };
        Some(self.next_cell(prompt_end))
    }

    /// The cell after `point`, wrapping to the next row's first.
    fn next_cell(&self, point: Point) -> Point {
        if point.column.0 + 1 < self.term.columns() {
            Point::new(point.line, point.column + 1)
        } else {
            Point::new(point.line + 1, Column(0))
        }
    }
}

/// The URI prefix of the links the session's reader puts on OSC 133 marks (see `Marks` in
/// `session.rs`); [`Emulator::hyperlink_at`] hides them.
pub const MARK_PREFIX: &str = "macaroni-mark:";

/// How many rows above the cursor [`Emulator::typed_command`] looks for the prompt.
pub const MAX_COMMAND_ROWS: i32 = 50;

/// Where a mark starts: its first row, whether it's a prompt (else command output), and for a
/// prompt the exit code of the command before it.
struct MarkStart {
    /// The row, in alacritty's coordinates (negative in the scrollback).
    line: Line,
    /// A prompt (`macaroni-mark:prompt`), else output.
    prompt: bool,
    /// From `macaroni-mark:prompt:<code>`: how the command before this prompt ended.
    exit: Option<i32>,
}

impl AlacrittyEmulator {
    /// Every mark in the scrollback and screen, top to bottom.
    fn marks(&self) -> Vec<MarkStart> {
        let grid = self.term.grid();
        self.marks_between(grid.topmost_line(), grid.bottommost_line())
    }

    /// The marks starting in rows `top..=bottom`: each row whose marked cells start a different
    /// mark (by link id) than the row above ended with. The row above `top` is read too, so a
    /// mark that began above the range isn't taken to start at its top.
    fn marks_between(&self, top: Line, bottom: Line) -> Vec<MarkStart> {
        let grid = self.term.grid();
        let mut found = Vec::new();
        let mut previous = (top > grid.topmost_line())
            .then(|| self.row_marks(top - 1).1)
            .flatten()
            .map(|(id, _)| id);
        for line in top.0..=bottom.0 {
            let (first, last) = self.row_marks(Line(line));
            if let Some((id, uri)) = first
                && previous.as_deref() != Some(id.as_str())
            {
                let kind = &uri[MARK_PREFIX.len()..];
                found.push(MarkStart {
                    line: Line(line),
                    prompt: kind.starts_with("prompt"),
                    exit: kind
                        .strip_prefix("prompt:")
                        .and_then(|code| code.parse().ok()),
                });
            }
            if let Some((id, _)) = last {
                previous = Some(id);
            }
        }
        found
    }

    /// The images anchored in visible rows `0..lines` of the view `top` starts at: each one
    /// once, placed by the first of its anchors found.
    fn images_between(&self, top: Line, lines: usize) -> Vec<ImagePlacement> {
        let mut found: Vec<ImagePlacement> = Vec::new();
        if self.images.is_empty() {
            return found;
        }
        let grid = self.term.grid();
        for visible in 0..lines {
            let row = &grid[top + visible as i32];
            for column in 0..self.term.columns() {
                let Some(link) = row[Column(column)].hyperlink() else {
                    continue;
                };
                let Some((id, image_row)) = crate::images::anchor(link.uri()) else {
                    continue;
                };
                if found.iter().any(|p| p.image.id == id) {
                    continue;
                }
                if let Some(image) = self.images.get(&id) {
                    found.push(ImagePlacement {
                        image: image.clone(),
                        row: visible as i32 - i32::from(image_row),
                        col: column,
                    });
                }
            }
        }
        found
    }

    /// A row's first and last marked cells, as (link id, URI).
    #[allow(clippy::type_complexity)]
    fn row_marks(&self, line: Line) -> (Option<(String, String)>, Option<(String, String)>) {
        let row = &self.term.grid()[line];
        let mut first = None;
        let mut last = None;
        for column in 0..self.term.columns() {
            if let Some(link) = row[Column(column)].hyperlink()
                && link.uri().starts_with(MARK_PREFIX)
            {
                let mark = (link.id().to_owned(), link.uri().to_owned());
                first.get_or_insert_with(|| mark.clone());
                last = Some(mark);
            }
        }
        (first, last)
    }
}

/// Converts a [`CellSide`] into alacritty's [`Side`].
fn side(side: CellSide) -> Side {
    match side {
        CellSide::Left => Side::Left,
        CellSide::Right => Side::Right,
    }
}

/// Escapes regex syntax so the search box matches text literally.
fn literal(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

impl Emulator for AlacrittyEmulator {
    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        self.answer_queries();
    }

    fn resize(&mut self, size: GridSize) {
        self.size = size;
        self.term.resize(Dims::from(size));
    }

    fn scroll(&mut self, scroll: Scroll) {
        self.term.scroll_display(match scroll {
            Scroll::Lines(lines) => AlacrittyScroll::Delta(lines),
            Scroll::PageUp => AlacrittyScroll::PageUp,
            Scroll::PageDown => AlacrittyScroll::PageDown,
            Scroll::Top => AlacrittyScroll::Top,
            Scroll::Bottom => AlacrittyScroll::Bottom,
        });
    }

    fn snapshot(&self) -> Snapshot {
        let content = self.term.renderable_content();
        let cols = self.term.columns();
        let lines = self.term.screen_lines();
        let offset = content.display_offset;

        let selection = content.selection;
        let mut rows = vec![Vec::with_capacity(cols); lines];
        for indexed in content.display_iter {
            // display_iter yields grid lines (negative in scrollback); adding the offset turns
            // them into visible rows. Anything outside the screen is skipped rather than trusted.
            let Ok(row) = usize::try_from(indexed.point.line.0 + offset as i32) else {
                continue;
            };
            if let Some(cells) = rows.get_mut(row) {
                let mut cell = convert_cell(indexed.cell);
                if selection.is_some_and(|s| s.contains(indexed.point)) {
                    cell.flags.insert(CellFlags::SELECTED);
                }
                cells.push(cell);
            }
        }

        // A hidden cursor, or one scrolled below the view, is `None`.
        let cursor = cursor_shape(content.cursor.shape).and_then(|shape| {
            let row = usize::try_from(content.cursor.point.line.0 + offset as i32).ok()?;
            (row < lines).then_some(Cursor {
                row,
                col: content.cursor.point.column.0,
                shape,
            })
        });

        // alacritty's runtime color table is indexed like `NamedColor`: 0-255 palette, then
        // Foreground (256) and Background (257). Only entries a program changed are `Some`.
        let color_overrides = (0..=NamedColor::Background as usize)
            .filter_map(|i| content.colors[i].map(|c| (i as u16, Rgb::new(c.r, c.g, c.b))))
            .collect();

        // Only the visible rows are scanned for marks (this runs on every wakeup).
        let top = Line(-(offset as i32));
        let prompts = self
            .marks_between(top, top + (lines as i32 - 1))
            .into_iter()
            .filter(|m| m.prompt)
            .filter_map(|m| Some(((m.line - top).0 as usize, m.exit?)))
            .collect();
        let images = self.images_between(top, lines);

        Snapshot {
            cols,
            rows,
            cursor,
            display_offset: offset,
            modes: modes(content.mode),
            has_selection: selection.is_some(),
            color_overrides,
            prompts,
            images,
        }
    }

    fn modes(&self) -> Modes {
        modes(*self.term.mode())
    }

    fn start_selection(&mut self, kind: SelectionKind, at: GridPoint, cell_side: CellSide) {
        let ty = match kind {
            SelectionKind::Simple => SelectionType::Simple,
            SelectionKind::Block => SelectionType::Block,
            SelectionKind::Word => SelectionType::Semantic,
            SelectionKind::Line => SelectionType::Lines,
        };
        let point = self.point(at);
        self.term.selection = Some(Selection::new(ty, point, side(cell_side)));
    }

    fn update_selection(&mut self, at: GridPoint, cell_side: CellSide) {
        let point = self.point(at);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side(cell_side));
        }
    }

    fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    fn hyperlink_at(&self, at: GridPoint) -> Option<String> {
        let point = self.point(at);
        self.term.grid()[point]
            .hyperlink()
            .map(|link| link.uri().to_owned())
            .filter(|uri| !uri.starts_with(MARK_PREFIX) && !uri.starts_with(IMAGE_PREFIX))
    }

    fn size(&self) -> GridSize {
        self.size
    }

    fn add_image(&mut self, image: InlineImage) {
        self.images.insert(image.id, Arc::new(image));
        let mut kept: usize = self.images.values().map(|i| i.data.len()).sum();
        while kept > MAX_KEPT_IMAGE_BYTES
            && let Some((_, oldest)) = self.images.pop_first()
        {
            kept -= oldest.data.len();
        }
    }

    fn jump_to_prompt(&mut self, up: bool) -> bool {
        let top = -(self.term.grid().display_offset() as i32);
        let mut prompts = self
            .marks()
            .into_iter()
            .filter(|m| m.prompt)
            .map(|m| m.line);
        let target = if up {
            prompts.rfind(|l| l.0 < top)
        } else {
            prompts.find(|l| l.0 > top)
        };
        let Some(line) = target else {
            return false;
        };
        // The top row is `-display_offset`, so showing `line` there means an offset of
        // `-line`, clamped to the scrollback (a prompt on the live screen just scrolls to the
        // bottom).
        let history = self.term.grid().history_size() as i32;
        let want = (-line.0).clamp(0, history);
        let delta = want - self.term.grid().display_offset() as i32;
        self.term.scroll_display(AlacrittyScroll::Delta(delta));
        true
    }

    fn all_text(&self) -> String {
        let grid = self.term.grid();
        let last_column = Column(self.term.columns().saturating_sub(1));
        self.term
            .bounds_to_string(
                Point::new(grid.topmost_line(), Column(0)),
                Point::new(grid.bottommost_line(), last_column),
            )
            .trim_end()
            .to_owned()
    }

    fn last_output(&self) -> Option<String> {
        let marks = self.marks();
        let at = marks.iter().rposition(|m| !m.prompt)?;
        let start = marks[at].line;
        let end = marks
            .get(at + 1)
            .map_or(self.term.grid().cursor.point.line, |next| next.line);
        if end <= start {
            return Some(String::new());
        }
        let last_column = Column(self.term.columns().saturating_sub(1));
        let text = self.term.bounds_to_string(
            Point::new(start, Column(0)),
            Point::new(end - 1, last_column),
        );
        Some(text.trim_end().to_owned())
    }

    fn typed_command(&self) -> Option<String> {
        let start = self.command_start()?;
        let cursor = self.term.grid().cursor.point;
        let columns = self.term.columns();
        // The cursor sits after the command, on its last row or (once Enter's newline is out)
        // at the start of the next.
        let end = if cursor.column.0 > 0 {
            Point::new(cursor.line, cursor.column - 1)
        } else {
            Point::new(Line(cursor.line.0 - 1), Column(columns.saturating_sub(1)))
        };
        if end < start {
            return None;
        }
        let command = self
            .term
            .bounds_to_string(start, end)
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned();
        (!command.is_empty()).then_some(command)
    }

    fn cursor_moves(&self, at: GridPoint) -> Option<i32> {
        let start = self.command_start()?;
        let grid = self.term.grid();
        let cursor = grid.cursor.point;
        let columns = self.term.columns();
        let last_column = Column(columns.saturating_sub(1));
        let bottom = Line(self.term.screen_lines() as i32 - 1);
        // The command runs on through soft wraps; a hard newline (a continuation prompt) ends
        // what arrow keys can reach on this line.
        let mut last = start.line;
        while last < bottom
            && grid[Point::new(last, last_column)]
                .flags
                .contains(Flags::WRAPLINE)
        {
            last += 1;
        }
        let target = self.point(at);
        let inside = |p: Point| p.line >= start.line && p.line <= last;
        if !inside(cursor) || !inside(target) {
            return None;
        }
        // One past the last character typed: a click beyond it lands there, not in blank cells
        // the shell would refuse to move into.
        let mut end = start;
        let mut p = start;
        while p.line <= last {
            if grid[p].c != ' ' {
                end = self.next_cell(p);
            }
            p = self.next_cell(p);
        }
        let target = target.max(start).min(end.max(cursor));
        // Arrows move by character: a wide character's spacer cell isn't one.
        let (from, to) = (cursor.min(target), cursor.max(target));
        let mut moves = 0;
        let mut p = from;
        while p < to {
            let flags = grid[p].flags;
            if !flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                moves += 1;
            }
            p = self.next_cell(p);
        }
        Some(if target < cursor { -moves } else { moves })
    }

    fn search(&mut self, query: &str, direction: SearchDirection) -> bool {
        if query.is_empty() {
            self.reset_search();
            self.term.selection = None;
            return false;
        }
        let Ok(mut regex) = RegexSearch::new(&literal(query)) else {
            return false;
        };
        // Start one cell past the previous match so repeating the search moves on instead of
        // finding the same text again.
        let (dir, origin) = match (direction, &self.last_match) {
            (SearchDirection::Older, Some(m)) => (
                Direction::Left,
                m.start().sub(&self.term, Boundary::None, 1),
            ),
            (SearchDirection::Newer, Some(m)) => {
                (Direction::Right, m.end().add(&self.term, Boundary::None, 1))
            }
            (SearchDirection::Older, None) => {
                let bottom = Line(self.term.screen_lines() as i32 - 1);
                (Direction::Left, Point::new(bottom, self.term.last_column()))
            }
            (SearchDirection::Newer, None) => (Direction::Right, self.term.grid().cursor.point),
        };
        // The origin cell itself counts when searching from its far side.
        let side = match dir {
            Direction::Left => Side::Right,
            Direction::Right => Side::Left,
        };
        match self.term.search_next(&mut regex, origin, dir, side, None) {
            Some(found) => {
                let mut selection =
                    Selection::new(SelectionType::Simple, *found.start(), Side::Left);
                selection.update(*found.end(), Side::Right);
                self.term.selection = Some(selection);
                self.term.scroll_to_point(*found.start());
                self.last_match = Some(found);
                true
            }
            None => false,
        }
    }

    fn reset_search(&mut self) {
        self.last_match = None;
    }

    fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    fn flush_sync(&mut self) {
        self.parser.stop_sync(&mut self.term);
        self.answer_queries();
    }

    fn set_default_colors(&mut self, colors: DefaultColors) {
        self.colors = Some(colors);
    }

    fn set_default_cursor(&mut self, shape: CursorShape) {
        let shape = match shape {
            CursorShape::Block => ansi::CursorShape::Block,
            CursorShape::HollowBlock => ansi::CursorShape::HollowBlock,
            CursorShape::Underline => ansi::CursorShape::Underline,
            CursorShape::Beam => ansi::CursorShape::Beam,
        };
        // `set_options` replaces the whole config, so the scrollback goes back in with it; the
        // same length leaves the history alone.
        self.term.set_options(Config {
            scrolling_history: self.scrollback,
            default_cursor_style: ansi::CursorStyle {
                shape,
                blinking: false,
            },
            ..Config::default()
        });
    }
}

/// Converts alacritty's mode bits into [`Modes`].
fn modes(mode: TermMode) -> Modes {
    Modes {
        app_cursor: mode.contains(TermMode::APP_CURSOR),
        app_keypad: mode.contains(TermMode::APP_KEYPAD),
        bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
        alt_screen: mode.contains(TermMode::ALT_SCREEN),
        focus_events: mode.contains(TermMode::FOCUS_IN_OUT),
        alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        mouse_click: mode.contains(TermMode::MOUSE_REPORT_CLICK),
        mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
        mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
        sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
        utf8_mouse: mode.contains(TermMode::UTF8_MOUSE),
    }
}

/// Converts alacritty's cursor shape; `None` for a hidden cursor.
fn cursor_shape(shape: ansi::CursorShape) -> Option<CursorShape> {
    match shape {
        ansi::CursorShape::Block => Some(CursorShape::Block),
        ansi::CursorShape::HollowBlock => Some(CursorShape::HollowBlock),
        ansi::CursorShape::Underline => Some(CursorShape::Underline),
        ansi::CursorShape::Beam => Some(CursorShape::Beam),
        ansi::CursorShape::Hidden => None,
    }
}

/// Copies an alacritty cell into a [`Cell`]. Hyperlinks aren't copied; they're looked up on
/// demand with [`Emulator::hyperlink_at`].
fn convert_cell(cell: &alacritty_terminal::term::cell::Cell) -> Cell {
    /// alacritty flags and the [`CellFlags`] they become. Several underline styles collapse
    /// into one.
    const FLAG_MAP: [(Flags, CellFlags); 9] = [
        (Flags::BOLD, CellFlags::BOLD),
        (Flags::ITALIC, CellFlags::ITALIC),
        (Flags::ALL_UNDERLINES, CellFlags::UNDERLINE),
        (Flags::STRIKEOUT, CellFlags::STRIKEOUT),
        (Flags::INVERSE, CellFlags::INVERSE),
        (Flags::DIM, CellFlags::DIM),
        (Flags::HIDDEN, CellFlags::HIDDEN),
        (Flags::WIDE_CHAR, CellFlags::WIDE),
        (Flags::WIDE_CHAR_SPACER, CellFlags::WIDE_SPACER),
    ];

    let mut flags = CellFlags::default();
    for (from, to) in FLAG_MAP {
        if cell.flags.intersects(from) {
            flags.insert(to);
        }
    }
    // The blank alacritty leaves at the end of a line when a wide character didn't fit; the
    // renderer treats it like any other spacer.
    if cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
        flags.insert(CellFlags::WIDE_SPACER);
    }

    Cell {
        ch: cell.c,
        zerowidth: cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
        fg: convert_color(cell.fg),
        bg: convert_color(cell.bg),
        flags,
    }
}

/// Converts an alacritty color into an unresolved [`Color`]. Dim named colors map to their base
/// index 0-7, since dimming is already carried by [`CellFlags::DIM`].
fn convert_color(color: ansi::Color) -> Color {
    match color {
        ansi::Color::Spec(c) => Color::Rgb(Rgb::new(c.r, c.g, c.b)),
        ansi::Color::Indexed(i) => Color::Indexed(i),
        ansi::Color::Named(named) => match named {
            NamedColor::Background => Color::Background,
            NamedColor::Foreground
            | NamedColor::BrightForeground
            | NamedColor::DimForeground
            | NamedColor::Cursor => Color::Foreground,
            NamedColor::DimBlack
            | NamedColor::DimRed
            | NamedColor::DimGreen
            | NamedColor::DimYellow
            | NamedColor::DimBlue
            | NamedColor::DimMagenta
            | NamedColor::DimCyan
            | NamedColor::DimWhite => {
                Color::Indexed((named as usize - NamedColor::DimBlack as usize) as u8)
            }
            base => Color::Indexed(base as u8),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const SIZE: GridSize = GridSize {
        cols: 20,
        rows: 4,
        cell_width: 8,
        cell_height: 16,
    };

    fn emulator() -> (AlacrittyEmulator, Arc<Mutex<Vec<EmulatorEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emulator = AlacrittyEmulator::new(SIZE, 100, move |e| sink.lock().unwrap().push(e));
        (emulator, events)
    }

    fn dump(snapshot: &Snapshot) -> String {
        snapshot
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .filter(|c| !c.flags.contains(CellFlags::WIDE_SPACER))
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn plain_text_lands_on_the_grid() {
        let (mut emu, _) = emulator();
        emu.feed(b"hello\r\nworld");
        let snap = emu.snapshot();
        insta::assert_snapshot!(dump(&snap), @r"
        hello
        world
        ");
        assert_eq!(
            snap.cursor,
            Some(Cursor {
                row: 1,
                col: 5,
                shape: CursorShape::Block
            })
        );
    }

    #[test]
    fn sgr_colors_and_flags_are_converted() {
        let (mut emu, _) = emulator();
        emu.feed(b"\x1b[1;31ma\x1b[0;38;2;1;2;3;48;5;200mb\x1b[0;7mc");
        let row = &emu.snapshot().rows[0];
        assert_eq!(row[0].fg, Color::Indexed(1));
        assert!(row[0].flags.contains(CellFlags::BOLD));
        assert_eq!(row[1].fg, Color::Rgb(Rgb::new(1, 2, 3)));
        assert_eq!(row[1].bg, Color::Indexed(200));
        assert!(row[2].flags.contains(CellFlags::INVERSE));
        assert_eq!(row[3].fg, Color::Foreground);
    }

    #[test]
    fn wide_chars_take_two_cells() {
        let (mut emu, _) = emulator();
        emu.feed("日本x".as_bytes());
        let row = &emu.snapshot().rows[0];
        assert!(row[0].flags.contains(CellFlags::WIDE));
        assert!(row[1].flags.contains(CellFlags::WIDE_SPACER));
        assert_eq!(row[4].ch, 'x');
    }

    #[test]
    fn cursor_position_query_is_answered() {
        let (mut emu, events) = emulator();
        emu.feed(b"ab\x1b[6n");
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [EmulatorEvent::PtyWrite("\x1b[1;3R".into())]
        );
    }

    /// Colors are answered from the defaults until the program sets its own, and not at all
    /// without defaults.
    #[test]
    fn color_queries_are_answered() {
        let (mut emu, events) = emulator();
        emu.feed(b"\x1b]11;?\x07");
        assert!(events.lock().unwrap().is_empty());

        emu.set_default_colors(DefaultColors {
            palette: [Rgb::new(0, 0, 0); 256],
            foreground: Rgb::new(0xee, 0xee, 0xee),
            background: Rgb::new(0x12, 0x34, 0x56),
            cursor: Rgb::new(0xff, 0, 0),
        });
        emu.feed(b"\x1b]11;?\x07");
        emu.feed(b"\x1b]11;#ffffff\x07\x1b]11;?\x07");
        let replies: Vec<_> = events
            .lock()
            .unwrap()
            .iter()
            .map(|e| match e {
                EmulatorEvent::PtyWrite(text) => text.clone(),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(replies.len(), 2);
        assert!(
            replies[0].starts_with("\x1b]11;rgb:1212/3434/5656"),
            "{:?}",
            replies[0]
        );
        assert!(
            replies[1].starts_with("\x1b]11;rgb:ffff/ffff/ffff"),
            "{:?}",
            replies[1]
        );
    }

    #[test]
    fn text_area_size_query_is_answered() {
        let (mut emu, events) = emulator();
        emu.feed(b"\x1b[14t");
        assert_eq!(
            events.lock().unwrap().as_slice(),
            [EmulatorEvent::PtyWrite("\x1b[4;64;160t".into())]
        );
    }

    #[test]
    fn alt_screen_and_cursor_modes_are_reported() {
        let (mut emu, _) = emulator();
        emu.feed(b"\x1b[?1049h\x1b[?1h\x1b[?2004h\x1b[?25l");
        let snap = emu.snapshot();
        assert!(snap.modes.alt_screen);
        assert!(snap.modes.app_cursor);
        assert!(snap.modes.bracketed_paste);
        assert_eq!(snap.cursor, None);
    }

    #[test]
    fn scrollback_moves_the_view() {
        let (mut emu, _) = emulator();
        for i in 0..10 {
            emu.feed(format!("line{i}\r\n").as_bytes());
        }
        emu.scroll(Scroll::Lines(2));
        let snap = emu.snapshot();
        assert_eq!(snap.display_offset, 2);
        insta::assert_snapshot!(dump(&snap), @r"
        line5
        line6
        line7
        line8
        ");
    }

    #[test]
    fn selection_marks_cells_and_copies_text() {
        let (mut emu, _) = emulator();
        emu.feed(b"hello world\r\nsecond");
        let at = |row, col| GridPoint { row, col };
        emu.start_selection(SelectionKind::Simple, at(0, 6), CellSide::Left);
        emu.update_selection(at(1, 2), CellSide::Right);
        let snap = emu.snapshot();
        assert!(snap.has_selection);
        assert!(!snap.rows[0][5].flags.contains(CellFlags::SELECTED));
        assert!(snap.rows[0][6].flags.contains(CellFlags::SELECTED));
        assert!(snap.rows[1][2].flags.contains(CellFlags::SELECTED));
        assert_eq!(emu.selection_text().as_deref(), Some("world\nsec"));
        emu.clear_selection();
        assert!(!emu.snapshot().has_selection);
    }

    #[test]
    fn word_selection_expands() {
        let (mut emu, _) = emulator();
        emu.feed(b"cargo build --release");
        let at = GridPoint { row: 0, col: 8 };
        emu.start_selection(SelectionKind::Word, at, CellSide::Left);
        assert_eq!(emu.selection_text().as_deref(), Some("build"));
    }

    #[test]
    fn search_walks_matches_and_treats_the_query_literally() {
        let (mut emu, _) = emulator();
        for i in 0..8 {
            emu.feed(format!("item {i} (x)\r\n").as_bytes());
        }
        assert!(emu.search("(x)", SearchDirection::Older));
        assert_eq!(emu.selection_text().as_deref(), Some("(x)"));
        let first = emu.snapshot();
        assert!(emu.search("(x)", SearchDirection::Older));
        let second = emu.snapshot();
        assert_ne!(first.rows, second.rows, "the view or highlight moved");
        assert!(!emu.search("nothing here", SearchDirection::Older));
        assert!(
            !emu.search("ITEM 3", SearchDirection::Older),
            "capitals make it case-sensitive"
        );
        assert!(emu.search("item 3", SearchDirection::Older));
        assert_eq!(emu.selection_text().as_deref(), Some("item 3"));
        emu.reset_search();
        assert!(!emu.search("", SearchDirection::Older));
        assert!(!emu.snapshot().has_selection);
    }

    #[test]
    fn the_typed_command_is_read_from_after_the_prompt() {
        let (mut emu, _) = emulator();
        let prompt = "\x1b]8;id=p1;macaroni-mark:prompt\x07$ \x1b]8;;\x07";
        emu.feed(format!("{prompt}cargo  test -p x").as_bytes());
        assert_eq!(emu.typed_command().as_deref(), Some("cargo  test -p x"));
        // Enter's newline is out: the cursor is on the next row.
        emu.feed(b"\r\n");
        assert_eq!(emu.typed_command().as_deref(), Some("cargo  test -p x"));
        emu.feed(b"\x1b]8;id=o1;macaroni-mark:output\x07done\r\n\x1b]8;;\x07");
        assert_eq!(
            emu.typed_command(),
            None,
            "output stands between it and the prompt"
        );
        let next = "\x1b]8;id=p2;macaroni-mark:prompt:0\x07$ \x1b]8;;\x07";
        emu.feed(next.as_bytes());
        assert_eq!(emu.typed_command(), None, "nothing typed");
    }

    #[test]
    fn clicks_in_the_typed_command_count_cursor_moves() {
        let (mut emu, _) = emulator();
        let prompt = "\x1b]8;id=p1;macaroni-mark:prompt\x07$ \x1b]8;;\x07";
        // 22 cells on a 20-column grid: the command wraps onto the second row.
        emu.feed(format!("{prompt}echo hello world abc").as_bytes());
        let at = |row, col| GridPoint { row, col };
        assert_eq!(emu.cursor_moves(at(0, 5)), Some(-17));
        assert_eq!(
            emu.cursor_moves(at(0, 0)),
            Some(-20),
            "the prompt counts as the start"
        );
        assert_eq!(
            emu.cursor_moves(at(1, 10)),
            Some(0),
            "past the text is its end"
        );
        assert_eq!(emu.cursor_moves(at(2, 0)), None, "below the command");

        let (mut emu, _) = emulator();
        emu.feed(format!("{prompt}漢x").as_bytes());
        assert_eq!(
            emu.cursor_moves(at(0, 2)),
            Some(-2),
            "a wide character is one move"
        );
    }

    #[test]
    fn prompt_marks_jump_and_copy_output() {
        let (mut emu, _) = emulator();
        // What the session's reader feeds for two commands: each prompt and output carries a
        // mark link with an id of its own.
        let command = |n: u32, output: &str| {
            format!(
                "\x1b]8;id=p{n};macaroni-mark:prompt\x07$ cmd{n}\r\n\x1b]8;id=o{n};macaroni-mark:output\x07{output}\x1b]8;;\x07"
            )
        };
        emu.feed(command(1, "a\r\nb\r\nc\r\n").as_bytes());
        emu.feed(command(2, "one\r\ntwo\r\n").as_bytes());
        emu.feed(b"\x1b]8;id=p3;macaroni-mark:prompt:0\x07$ ");

        assert_eq!(emu.last_output().as_deref(), Some("one\ntwo"));
        // Only the third prompt has an exit code before it, and it's on the bottom row.
        assert_eq!(emu.snapshot().prompts, vec![(3, 0)]);
        // The marks' links are internal, not links to open.
        assert_eq!(emu.hyperlink_at(GridPoint { row: 3, col: 0 }), None);

        // 4 rows on screen: "$ cmd2" is the top row, and the first prompt is up in the
        // scrollback.
        assert!(dump(&emu.snapshot()).starts_with("$ cmd2"));
        assert!(emu.jump_to_prompt(true));
        assert!(dump(&emu.snapshot()).starts_with("$ cmd1"));
        assert!(!emu.jump_to_prompt(true), "no prompt above the first");
        assert!(emu.jump_to_prompt(false));
        assert!(dump(&emu.snapshot()).starts_with("$ cmd2"));
    }

    #[test]
    fn images_show_where_their_anchors_are() {
        let (mut emu, _) = emulator();
        let image = InlineImage {
            id: 7,
            data: Arc::from(vec![0u8]),
            width: 24,
            height: 32,
            cols: 3,
            rows: 2,
            stretch: false,
        };
        emu.feed(b"ab");
        emu.feed(&image.anchors());
        emu.add_image(image);
        emu.feed(b"!");
        let snapshot = emu.snapshot();
        assert_eq!(snapshot.images.len(), 1);
        assert_eq!((snapshot.images[0].row, snapshot.images[0].col), (0, 2));
        // The cursor ends right of the block's last row, and the anchors aren't links.
        assert_eq!(snapshot.rows[1][5].ch, '!');
        assert_eq!(emu.hyperlink_at(GridPoint { row: 0, col: 2 }), None);
        // Scrolled so only its last row shows, it starts a row above the view.
        emu.feed(b"\r\n\r\n\r\n");
        let snapshot = emu.snapshot();
        assert_eq!((snapshot.images[0].row, snapshot.images[0].col), (-1, 2));
        // Gone once both rows have scrolled off.
        emu.feed(b"\r\n");
        assert!(emu.snapshot().images.is_empty());
    }

    #[test]
    fn osc8_hyperlinks_are_readable() {
        let (mut emu, _) = emulator();
        emu.feed(b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ plain");
        assert_eq!(
            emu.hyperlink_at(GridPoint { row: 0, col: 1 }).as_deref(),
            Some("https://example.com")
        );
        assert_eq!(emu.hyperlink_at(GridPoint { row: 0, col: 7 }), None);
    }

    #[test]
    fn synchronized_updates_are_held_until_they_end_or_time_out() {
        let (mut emu, _) = emulator();
        emu.feed(b"\x1b[?2026hhidden");
        assert!(emu.sync_deadline().is_some());
        assert_eq!(
            emu.snapshot().rows[0][0].ch,
            ' ',
            "held back while the update is open"
        );
        emu.flush_sync();
        assert_eq!(emu.sync_deadline(), None);
        assert_eq!(emu.snapshot().rows[0][0].ch, 'h');

        emu.feed(b"\x1b[?2026h!\x1b[?2026l");
        assert_eq!(
            emu.sync_deadline(),
            None,
            "a finished update needs no timer"
        );
    }

    #[test]
    fn osc_palette_changes_are_exposed() {
        let (mut emu, _) = emulator();
        emu.feed(b"\x1b]11;rgb:10/20/30\x1b\\");
        assert_eq!(
            emu.snapshot().color_overrides,
            vec![(257, Rgb::new(0x10, 0x20, 0x30))]
        );
    }
}
