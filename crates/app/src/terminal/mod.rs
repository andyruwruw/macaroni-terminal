//! The terminal view: [`TerminalView`], a custom iced widget that draws one session's
//! [`Snapshot`] and turns keyboard, mouse and IME input into [`ViewEvent`]s for the app.
//!
//! The app owns the session and its latest snapshot and builds a view for each visible terminal
//! tab (`view::panes`); the widget only keeps view state ([`State`]: the render cache, the IME
//! preedit, the scroll remainder, selection and mouse-report bookkeeping). It's a custom widget
//! rather than a canvas because it needs the IME (`Shell::request_input_method`), raw keys with
//! their physical codes, and a per-widget render cache that survives across frames.
//!
//! Drawing lives in [`render`] (rows split into runs, cached by content), [`box_drawing`]
//! (box and block characters as rectangles) and [`frame_stats`] (opt-in timing). Keys are
//! encoded with `macaroni_term::encode_key` after the [`Keymap`] had its look; mouse reports with
//! `macaroni_term::encode_mouse`.

mod box_drawing;
mod frame_stats;
mod render;

use std::cell::RefCell;
use std::time::{Duration, Instant};

use iced::advanced::input_method::{self, InputMethod, Preedit, Purpose};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, mouse};
use iced::keyboard::{self, Modifiers};
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector, window};
use macaroni_core::SessionId;
use macaroni_term::{
    CellSide, GridPoint, GridSize, Key, Link, Mods, MouseButton, MouseEvent, Scroll, SelectionKind,
    Snapshot, encode_key, encode_mouse, url_at,
};

pub use render::Metrics;
use render::{Frame, GridRenderer};

use crate::input;
use crate::keymap::{Action, Keymap, Zoom};
use crate::theme::Palette;

/// The longest gap between clicks that still makes a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Clicks in a row cycle through selecting from the cell, the word and the line.
const MAX_CLICKS: u8 = 3;
/// Lines scrolled per wheel notch, unless set with [`TerminalView::wheel_lines`].
const WHEEL_LINES: f32 = 3.0;
/// Space around the text inside the view, unless set with [`TerminalView::padding`].
const DEFAULT_PADDING: f32 = 6.0;

/// Ctrl+click opens links on Windows and Linux, Cmd+click on macOS.
fn link_chord(mods: Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        mods.logo()
    } else {
        mods.control()
    }
}

/// The button as the program sees it in mouse reports; `None` for buttons reports have no
/// code for (back, forward).
fn report_button(button: mouse::Button) -> Option<MouseButton> {
    match button {
        mouse::Button::Left => Some(MouseButton::Left),
        mouse::Button::Middle => Some(MouseButton::Middle),
        mouse::Button::Right => Some(MouseButton::Right),
        _ => None,
    }
}

/// A change to the session's selection, applied by the app to the emulator.
#[derive(Debug, Clone, Copy)]
pub enum SelectOp {
    /// Starts a new selection at a cell, replacing any other.
    Start {
        /// Cell by cell, by word, by line, or a block.
        kind: SelectionKind,
        /// The cell under the pointer, in screen coordinates (row 0 is the top visible line).
        at: GridPoint,
        /// Which half of the cell, so a selection can start or end between cells.
        side: CellSide,
    },
    /// Moves the selection's moving end while dragging.
    Update {
        /// The cell under the pointer.
        at: GridPoint,
        /// Which half of the cell.
        side: CellSide,
    },
    /// Drops the selection (a click without dragging).
    Clear,
}

/// What a [`TerminalView`] tells the app; wrapped into a message by the view's `on_event`.
#[derive(Debug, Clone)]
pub enum ViewEvent {
    /// The grid's size in cells changed (or this is the view's first draw for this session). The
    /// app resizes the PTY, or spawns the session on its first `Resize` so the shell starts at
    /// its real size.
    Resize(GridSize),
    /// Bytes for the program: encoded keys, IME commits, mouse reports, alternate-scroll arrows.
    Write(Vec<u8>),
    /// Scrolls the scrollback.
    Scroll(Scroll),
    /// The window gained or lost focus while this view's pane was focused.
    Focus(bool),
    /// The view was clicked while its pane wasn't focused.
    Clicked,
    /// A key press matched an app shortcut instead of going to the shell.
    Action(Action),
    /// Changes the selection.
    Select(SelectOp),
    /// The open-link modifier (Ctrl, Cmd on macOS) was held while clicking this cell; the app
    /// looks for a link there and opens it.
    Link(GridPoint),
    /// Enter while the app has an item to rename (`enter_renames`).
    RenameItem,
}

/// One terminal on screen: borrows the session's snapshot, the palette and the keymap for one
/// frame. Fills its bounds; the grid is as many cells as fit inside the padding.
pub struct TerminalView<'a, M> {
    /// The session shown, which keys the reported size in [`State`].
    session: SessionId,
    /// The session's latest screen contents and modes.
    snapshot: &'a Snapshot,
    /// Colors for the cells.
    palette: &'a Palette,
    /// Font and cell size.
    metrics: Metrics,
    /// App shortcuts, looked up before a key goes to the shell.
    keymap: &'a Keymap,
    /// Whether this view's pane has keyboard focus within the app.
    focused: bool,
    /// Leave the default background unpainted, so what's under the view shows through.
    transparent: bool,
    /// Around the text, inside the widget's bounds.
    padding: f32,
    /// A plain Enter is reported as `RenameItem` instead of going to the shell.
    enter_renames: bool,
    /// Lines scrolled per wheel notch.
    wheel_lines: f32,
    /// Finishing a selection copies it.
    copy_on_select: bool,
    /// Wraps a [`ViewEvent`] into the app's message.
    on_event: Box<dyn Fn(ViewEvent) -> M + 'a>,
}

impl<'a, M> TerminalView<'a, M> {
    /// A view of `session` with the default padding, painting its background. `focused` is
    /// whether its pane has keyboard focus in the app; only a focused view takes keys and IME
    /// input.
    pub fn new(
        session: SessionId,
        snapshot: &'a Snapshot,
        palette: &'a Palette,
        metrics: Metrics,
        keymap: &'a Keymap,
        focused: bool,
        on_event: impl Fn(ViewEvent) -> M + 'a,
    ) -> Self {
        Self {
            session,
            snapshot,
            palette,
            metrics,
            keymap,
            focused,
            transparent: false,
            padding: DEFAULT_PADDING,
            enter_renames: false,
            wheel_lines: WHEEL_LINES,
            copy_on_select: false,
            on_event: Box::new(on_event),
        }
    }

    /// Leaves cells with the default background unpainted (the glass look), so the pane card
    /// shows through; cells with any other background still paint it.
    pub fn transparent(self, transparent: bool) -> Self {
        Self {
            transparent,
            ..self
        }
    }

    /// Sets the space between the widget's edges and the grid, in logical pixels.
    pub fn padding(self, padding: f32) -> Self {
        Self { padding, ..self }
    }

    /// Reports a plain Enter as [`ViewEvent::RenameItem`] instead of sending it, while the app
    /// has a clicked sidebar item or tab waiting for Enter to rename it.
    pub fn enter_renames(self, enter_renames: bool) -> Self {
        Self {
            enter_renames,
            ..self
        }
    }

    /// Sets how many lines a wheel notch scrolls (`[terminal] scroll_lines`).
    pub fn wheel_lines(self, wheel_lines: f32) -> Self {
        Self {
            wheel_lines,
            ..self
        }
    }

    /// Copies a selection as soon as it's finished (`[terminal] copy_on_select`).
    pub fn copy_on_select(self, copy_on_select: bool) -> Self {
        Self {
            copy_on_select,
            ..self
        }
    }

    /// The grid that fits in `bounds` inside the padding: at least 1x1, whole cells only (the
    /// leftover is empty space at the right and bottom).
    fn grid_size(&self, bounds: Rectangle) -> GridSize {
        let cells =
            |extent: f32, cell: f32| ((extent - self.padding * 2.0) / cell).floor().max(1.0) as u16;
        GridSize {
            cols: cells(bounds.width, self.metrics.cell_width),
            rows: cells(bounds.height, self.metrics.cell_height),
            cell_width: self.metrics.cell_width.round() as u16,
            cell_height: self.metrics.cell_height.round() as u16,
        }
    }

    /// The cell under window point `position` and which half of it, clamped to the grid, so a
    /// drag past the edges keeps selecting the edge cells.
    fn cell_at(&self, bounds: Rectangle, position: Point) -> (GridPoint, CellSide) {
        let x = ((position.x - bounds.x - self.padding) / self.metrics.cell_width).max(0.0);
        let y = ((position.y - bounds.y - self.padding) / self.metrics.cell_height).max(0.0);
        let rows = self.snapshot.rows.len().max(1);
        let cols = self.snapshot.cols.max(1);
        let side = if x.fract() < 0.5 {
            CellSide::Left
        } else {
            CellSide::Right
        };
        (
            GridPoint {
                row: (y as usize).min(rows - 1),
                col: (x as usize).min(cols - 1),
            },
            side,
        )
    }

    /// The URL under the cursor, as its row and the link, if the open-link modifier is held.
    /// `None` when it isn't, the cursor is outside, or there's no URL there.
    fn link_under(
        &self,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        mods: Modifiers,
    ) -> Option<(usize, Link)> {
        if !link_chord(mods) {
            return None;
        }
        let position = cursor.position_over(bounds)?;
        let (at, _) = self.cell_at(bounds, position);
        let link = url_at(self.snapshot.rows.get(at.row)?, at.col)?;
        Some((at.row, link))
    }

    /// Whether mouse input goes to the program: it enabled mouse reporting and Shift isn't held
    /// (Shift keeps local selection available, as in other terminals).
    fn reporting(&self, mods: Modifiers) -> bool {
        self.snapshot.modes.mouse_reporting() && !mods.shift()
    }

    /// Sends a mouse report for `event` at `at`, if the program's mouse mode reports it.
    fn report(&self, shell: &mut Shell<'_, M>, event: MouseEvent, at: GridPoint, mods: Modifiers) {
        if let Some(bytes) = encode_mouse(event, at, input::mods(mods), self.snapshot.modes) {
            shell.publish((self.on_event)(ViewEvent::Write(bytes)));
        }
    }

    /// The cursor's cell in window coordinates, for the IME's candidate window; `None` while the
    /// cursor is hidden.
    fn caret(&self, bounds: Rectangle) -> Option<Rectangle> {
        let cursor = self.snapshot.cursor?;
        Some(Rectangle::new(
            Point::new(
                bounds.x + self.padding + cursor.col as f32 * self.metrics.cell_width,
                bounds.y + self.padding + cursor.row as f32 * self.metrics.cell_height,
            ),
            Size::new(self.metrics.cell_width, self.metrics.cell_height),
        ))
    }
}

/// The widget's own state, kept by iced in the widget tree across frames.
struct State {
    /// The row layout cache; a `RefCell` because `draw` only gets `&Tree`.
    grid: RefCell<GridRenderer>,
    /// iced reuses widget state by tree position, so the reported size is keyed by session:
    /// a tab switched into this position still gets its own `Resize`.
    size: Option<(SessionId, GridSize)>,
    /// Whether the window has focus: the cursor is hollow and the IME is released without it.
    window_focused: bool,
    /// The IME's composition, drawn at the cursor and never sent. `Some` with empty content
    /// while the IME is open but nothing is composed.
    preedit: Option<Preedit>,
    /// The fraction of a wheel step left over from the last wheel event (touchpads send small
    /// pixel deltas).
    scroll_remainder: f32,
    /// The modifiers held, from `ModifiersChanged` (mouse events don't carry them).
    mods: Modifiers,
    /// Set while the left button is held after starting a selection; `true` once it moved.
    dragging: Option<bool>,
    /// When and where the last left click was, and how many clicks in a row it made.
    last_click: Option<(Instant, GridPoint, u8)>,
    /// The URL under the pointer while the open-link modifier is held.
    hover: Option<(usize, Link)>,
    /// The button held while the program receives mouse events, so its release is reported even
    /// outside the view.
    reported_button: Option<MouseButton>,
    /// The last cell reported, so motion is only sent when the pointer reaches a new cell.
    reported_cell: Option<GridPoint>,
}

impl<M> TerminalView<'_, M> {
    /// Sends `event` to the app.
    fn emit(&self, shell: &mut Shell<'_, M>, event: ViewEvent) {
        shell.publish((self.on_event)(event));
    }

    /// Redraws report the grid size and the IME caret; focus changes go to the program.
    fn on_window(
        &self,
        state: &mut State,
        event: &window::Event,
        bounds: Rectangle,
        shell: &mut Shell<'_, M>,
    ) {
        match event {
            window::Event::RedrawRequested(_) => self.on_redraw(state, bounds, shell),
            window::Event::Focused => self.on_window_focus(state, true, shell),
            window::Event::Unfocused => self.on_window_focus(state, false, shell),
            _ => {}
        }
    }

    /// Reports a new grid size, and asks for the IME at the cursor while focused.
    fn on_redraw(&self, state: &mut State, bounds: Rectangle, shell: &mut Shell<'_, M>) {
        // Checked on redraw rather than in `layout`: `update` can publish messages, `layout`
        // can't.
        let size = self.grid_size(bounds);
        if state.size != Some((self.session, size)) {
            state.size = Some((self.session, size));
            self.emit(shell, ViewEvent::Resize(size));
        }
        // The IME request lasts one frame, so it's renewed on every redraw.
        if self.focused
            && state.window_focused
            && let Some(caret) = self.caret(bounds)
        {
            shell.request_input_method(&InputMethod::Enabled {
                cursor: caret,
                purpose: Purpose::Terminal,
                preedit: state.preedit.as_ref().map(Preedit::as_ref),
            });
        }
    }

    /// Tells the program the window gained or lost focus, if this view's pane is focused.
    fn on_window_focus(&self, state: &mut State, focused: bool, shell: &mut Shell<'_, M>) {
        state.window_focused = focused;
        if self.focused {
            self.emit(shell, ViewEvent::Focus(focused));
        }
        shell.request_redraw();
    }

    /// Updates the link under the pointer, which shows while the open-link modifier is held.
    fn update_hover(
        &self,
        state: &mut State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) {
        let hover = self.link_under(bounds, cursor, state.mods);
        if hover != state.hover {
            state.hover = hover;
            shell.request_redraw();
        }
    }

    /// Presses, motion, releases and the wheel.
    fn on_mouse(
        &self,
        state: &mut State,
        event: &mouse::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) {
        match event {
            mouse::Event::ButtonPressed(button) if cursor.is_over(bounds) => {
                self.on_press(state, *button, bounds, cursor, shell);
            }
            mouse::Event::CursorMoved { position } => {
                self.on_move(state, *position, bounds, cursor, shell);
            }
            mouse::Event::ButtonReleased(button)
                if state.reported_button.is_some()
                    && state.reported_button == report_button(*button) =>
            {
                self.on_reported_release(state, *button, bounds, cursor, shell);
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => match state.dragging.take() {
                Some(false) => self.emit(shell, ViewEvent::Select(SelectOp::Clear)),
                // A drag, or a word or line: there's something selected now.
                Some(true) if self.copy_on_select => {
                    self.emit(shell, ViewEvent::Action(Action::Copy));
                }
                _ => {}
            },
            mouse::Event::WheelScrolled { delta }
                if cursor.is_over(bounds) && state.mods.command() =>
            {
                self.on_wheel_zoom(state, delta, shell);
            }
            mouse::Event::WheelScrolled { delta } if cursor.is_over(bounds) => {
                self.on_wheel(state, delta, bounds, cursor, shell);
            }
            _ => {}
        }
    }

    /// A press goes to the program if it reports the mouse; otherwise it opens a link, starts a
    /// selection, or copies or pastes.
    fn on_press(
        &self,
        state: &mut State,
        button: mouse::Button,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) {
        // Focusing the pane doesn't eat the click: it still selects, reports or opens a link.
        if !self.focused {
            self.emit(shell, ViewEvent::Clicked);
        }
        let Some(position) = cursor.position() else {
            return;
        };
        let (at, side) = self.cell_at(bounds, position);
        if self.reporting(state.mods) {
            if let Some(button) = report_button(button) {
                self.report(shell, MouseEvent::Press(button), at, state.mods);
                state.reported_button = Some(button);
                state.reported_cell = Some(at);
                shell.capture_event();
            }
            return;
        }
        match button {
            mouse::Button::Left if link_chord(state.mods) => {
                self.emit(shell, ViewEvent::Link(at));
            }
            mouse::Button::Left => self.start_selection(state, at, side, shell),
            // Windows Terminal convention: right-click copies a selection, else pastes.
            mouse::Button::Right => {
                let action = if self.snapshot.has_selection {
                    Action::Copy
                } else {
                    Action::Paste
                };
                self.emit(shell, ViewEvent::Action(action));
            }
            _ => return,
        }
        shell.capture_event();
    }

    /// A click selects from the cell, a double click the word, a triple click the line; Alt
    /// makes a single click's selection a block.
    fn start_selection(
        &self,
        state: &mut State,
        at: GridPoint,
        side: CellSide,
        shell: &mut Shell<'_, M>,
    ) {
        let now = Instant::now();
        let count = match state.last_click {
            Some((when, last, n)) if now - when < MULTI_CLICK && last == at => n % MAX_CLICKS + 1,
            _ => 1,
        };
        state.last_click = Some((now, at, count));
        // Word and line selections already cover something, so releasing without moving keeps
        // them; only a single click's empty selection is cleared on release.
        let kind = match count {
            2 => SelectionKind::Word,
            3 => SelectionKind::Line,
            _ if state.mods.alt() => SelectionKind::Block,
            _ => SelectionKind::Simple,
        };
        self.emit(shell, ViewEvent::Select(SelectOp::Start { kind, at, side }));
        state.dragging = Some(count > 1);
    }

    /// Reports motion to the program, extends a selection being dragged, and updates the link
    /// under the pointer.
    fn on_move(
        &self,
        state: &mut State,
        position: Point,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) {
        // While a reported button is held, motion outside the view still goes to the program
        // (clamped to the edge cell), as a drag would.
        if self.reporting(state.mods) && (state.reported_button.is_some() || cursor.is_over(bounds))
        {
            let (at, _) = self.cell_at(bounds, position);
            if state.reported_cell != Some(at) {
                state.reported_cell = Some(at);
                self.report(
                    shell,
                    MouseEvent::Motion(state.reported_button),
                    at,
                    state.mods,
                );
            }
        }
        if let Some(moved) = &mut state.dragging {
            *moved = true;
            let (at, side) = self.cell_at(bounds, position);
            self.emit(shell, ViewEvent::Select(SelectOp::Update { at, side }));
        }
        self.update_hover(state, bounds, cursor, shell);
    }

    /// Reports the release of the button the program saw pressed.
    fn on_reported_release(
        &self,
        state: &mut State,
        button: mouse::Button,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) {
        state.reported_button = None;
        if let Some(position) = cursor.position() {
            let (at, _) = self.cell_at(bounds, position);
            if let Some(button) = report_button(button) {
                self.report(shell, MouseEvent::Release(button), at, state.mods);
            }
        }
    }

    /// Whole wheel steps so far (lines times `per_line`, or pixels in cell heights), keeping
    /// the fraction for the next event.
    fn wheel_steps(&self, state: &mut State, delta: &mouse::ScrollDelta, per_line: f32) -> f32 {
        let steps = match delta {
            mouse::ScrollDelta::Lines { y, .. } => y * per_line,
            mouse::ScrollDelta::Pixels { y, .. } => y / self.metrics.cell_height,
        } + state.scroll_remainder;
        let whole = steps.trunc();
        state.scroll_remainder = steps - whole;
        whole
    }

    /// The wheel with the zoom modifier held zooms the font a step per line.
    fn on_wheel_zoom(
        &self,
        state: &mut State,
        delta: &mouse::ScrollDelta,
        shell: &mut Shell<'_, M>,
    ) {
        let whole = self.wheel_steps(state, delta, 1.0);
        let zoom = if whole > 0.0 { Zoom::In } else { Zoom::Out };
        for _ in 0..whole.abs() as usize {
            self.emit(shell, ViewEvent::Action(Action::Zoom(zoom)));
        }
        shell.capture_event();
    }

    /// The wheel goes to the program as wheel buttons if it reports the mouse, as arrow keys in
    /// a full-screen program that asked for that, and otherwise scrolls the scrollback.
    fn on_wheel(
        &self,
        state: &mut State,
        delta: &mouse::ScrollDelta,
        bounds: Rectangle,
        cursor: mouse::Cursor,
        shell: &mut Shell<'_, M>,
    ) {
        let whole = self.wheel_steps(state, delta, self.wheel_lines);
        if whole == 0.0 {
            return;
        }
        let modes = self.snapshot.modes;
        if self.reporting(state.mods)
            && let Some(position) = cursor.position()
        {
            let (at, _) = self.cell_at(bounds, position);
            let button = if whole > 0.0 {
                MouseButton::WheelUp
            } else {
                MouseButton::WheelDown
            };
            for _ in 0..whole.abs() as usize {
                self.report(shell, MouseEvent::Press(button), at, state.mods);
            }
        } else if modes.alt_screen && modes.alternate_scroll && !modes.mouse_reporting() {
            // The alternate screen has no scrollback; full-screen programs like `less` ask for
            // arrows instead (DECSET 1007).
            let key = if whole > 0.0 { Key::Up } else { Key::Down };
            let bytes: Vec<u8> = (0..whole.abs() as usize)
                .filter_map(|_| encode_key(key, None, Mods::default(), modes))
                .flatten()
                .collect();
            self.emit(shell, ViewEvent::Write(bytes));
        } else {
            self.emit(shell, ViewEvent::Scroll(Scroll::Lines(whole as i32)));
        }
        shell.capture_event();
    }

    /// Tracks the IME's composition, and writes committed text to the shell.
    fn on_ime(&self, state: &mut State, ime: &input_method::Event, shell: &mut Shell<'_, M>) {
        match ime {
            input_method::Event::Opened => state.preedit = Some(Preedit::new()),
            input_method::Event::Closed => state.preedit = None,
            input_method::Event::Preedit(content, selection) => {
                state.preedit = Some(Preedit {
                    content: content.clone(),
                    selection: selection.clone(),
                    text_size: None,
                });
            }
            input_method::Event::Commit(text) => {
                self.emit(shell, ViewEvent::Write(text.as_bytes().to_vec()));
            }
        }
        shell.capture_event();
        shell.request_redraw();
    }

    /// A key goes to the view's own handling first (see [`Self::intercept_key`]), else to the
    /// shell, encoded for its modes. Nothing goes out while the IME is composing.
    fn on_key(
        &self,
        state: &State,
        key: &keyboard::Key,
        physical_key: keyboard::key::Physical,
        text: Option<&str>,
        modifiers: Modifiers,
        shell: &mut Shell<'_, M>,
    ) {
        let composing = state
            .preedit
            .as_ref()
            .is_some_and(|p| !p.content.is_empty());
        if composing {
            return;
        }
        if let Some(event) = self.intercept_key(key, physical_key, modifiers) {
            self.emit(shell, event);
            shell.capture_event();
            return;
        }
        // Cmd/Win chords that aren't shortcuts belong to the OS, not the shell.
        if modifiers.logo() {
            return;
        }
        let Some(key) = input::key(key) else {
            return;
        };
        if let Some(bytes) = encode_key(key, text, input::mods(modifiers), self.snapshot.modes) {
            self.emit(shell, ViewEvent::Write(bytes));
            shell.capture_event();
        }
    }

    /// A key the view handles instead of the shell: copying, renaming, or an app shortcut.
    fn intercept_key(
        &self,
        key: &keyboard::Key,
        physical_key: keyboard::key::Physical,
        modifiers: Modifiers,
    ) -> Option<ViewEvent> {
        let ctrl_only =
            modifiers.control() && !modifiers.shift() && !modifiers.alt() && !modifiers.logo();
        let copy_key = matches!(
            key,
            keyboard::Key::Character(c) if c.eq_ignore_ascii_case("c") || c.eq_ignore_ascii_case("x")
        );
        if ctrl_only && copy_key && self.snapshot.has_selection && !cfg!(target_os = "macos") {
            // Ctrl+C copies when there's a selection (like Windows Terminal), else it's ^C.
            // Ctrl+X too: terminal text can't be cut, so it copies and drops the selection.
            return Some(ViewEvent::Action(Action::Copy));
        }
        if self.enter_renames
            && modifiers.is_empty()
            && *key == keyboard::Key::Named(keyboard::key::Named::Enter)
        {
            return Some(ViewEvent::RenameItem);
        }
        self.keymap
            .lookup(key, physical_key, modifiers)
            .map(ViewEvent::Action)
    }
}

impl<M> Widget<M, Theme, iced::Renderer> for TerminalView<'_, M> {
    // `window_focused` starts true: the window is focused when the first view appears, and
    // `Focused` only arrives on a change.
    fn tag(&self) -> widget::tree::Tag {
        widget::tree::Tag::of::<State>()
    }

    fn state(&self) -> widget::tree::State {
        widget::tree::State::new(State {
            grid: RefCell::new(GridRenderer::default()),
            size: None,
            window_focused: true,
            preedit: None,
            scroll_remainder: 0.0,
            mods: Modifiers::default(),
            dragging: None,
            last_click: None,
            hover: None,
            reported_button: None,
            reported_cell: None,
        })
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        // Takes all the room it's given; the grid adapts to it, not the other way round.
        layout::Node::new(limits.max())
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        // Mouse and window events reach every view; keys and IME input only the focused one.
        match event {
            Event::Window(event) => self.on_window(state, event, bounds, shell),
            Event::Keyboard(keyboard::Event::ModifiersChanged(mods)) => {
                state.mods = *mods;
                self.update_hover(state, bounds, cursor, shell);
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                physical_key,
                text,
                modifiers,
                ..
            }) if self.focused => {
                self.on_key(
                    state,
                    key,
                    *physical_key,
                    text.as_deref(),
                    *modifiers,
                    shell,
                );
            }
            Event::InputMethod(ime) if self.focused => self.on_ime(state, ime, shell),
            Event::Mouse(event) => self.on_mouse(state, event, bounds, cursor, shell),
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        // A hand over a link, an arrow while the program gets the mouse, else an I-beam (also
        // while a selection drag has left the view).
        if state.hover.is_some() {
            mouse::Interaction::Pointer
        } else if self.reporting(state.mods) && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Idle
        } else if state.dragging.is_some() || cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::None
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let frame = Frame {
            snapshot: self.snapshot,
            palette: self.palette,
            metrics: self.metrics,
            origin: bounds.position() + Vector::new(self.padding, self.padding),
            transparent: self.transparent,
            focused: self.focused && state.window_focused,
            preedit: state
                .preedit
                .as_ref()
                .map(|p| p.content.as_str())
                .filter(|p| !p.is_empty()),
            link: state
                .hover
                .as_ref()
                .map(|(row, link)| (*row, link.start, link.end)),
        };
        let started = frame_stats::start();
        state.grid.borrow_mut().draw(renderer, &frame, bounds);
        frame_stats::finish(started);
    }
}

impl<'a, M: 'a> From<TerminalView<'a, M>> for Element<'a, M> {
    fn from(view: TerminalView<'a, M>) -> Self {
        Element::new(view)
    }
}
