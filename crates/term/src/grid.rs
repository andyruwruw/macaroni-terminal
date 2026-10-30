//! The crate's own screen types: what a [`Snapshot`] is made of, plus the small value types the
//! [`Emulator`](crate::Emulator) trait takes (positions, selection kinds, scroll requests).
//!
//! These mirror `alacritty_terminal` concepts without exposing its types, so the app and its
//! renderer depend only on this module. [`AlacrittyEmulator`](crate::AlacrittyEmulator) converts
//! into them in `emulator.rs`.

/// The size of a terminal, in cells and in pixels per cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSize {
    /// Columns of cells. The emulator treats 0 as 1.
    pub cols: u16,
    /// Rows of cells. The emulator treats 0 as 1.
    pub rows: u16,
    /// Width of one cell in pixels; only used to tell the PTY its pixel size.
    pub cell_width: u16,
    /// Height of one cell in pixels; only used to tell the PTY its pixel size.
    pub cell_height: u16,
}

/// A 24-bit color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Rgb {
    /// Builds a color from its channels.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// The colors a program is told when it asks for one it hasn't set itself (OSC 4, 10, 11 and
/// 12). Only the app knows the theme, so it hands them to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultColors {
    /// The 256-color palette (OSC 4).
    pub palette: [Rgb; 256],
    /// The default text color (OSC 10).
    pub foreground: Rgb,
    /// The default background (OSC 11).
    pub background: Rgb,
    /// The cursor (OSC 12).
    pub cursor: Rgb,
}

/// A cell color as the program set it, not yet resolved against a palette.
///
/// The app resolves it against its theme plus [`Snapshot::color_overrides`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    /// The default foreground. alacritty's bright and dim foreground and its cursor color map
    /// here too.
    Foreground,
    /// The default background.
    Background,
    /// An entry of the 256-color palette. alacritty's `Dim*` colors map to their base index
    /// (0-7); the dimming is carried by [`CellFlags::DIM`].
    Indexed(u8),
    /// A truecolor value (`SGR 38;2` / `48;2`).
    Rgb(Rgb),
}

/// Rendering attributes of a cell, as a small bit set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CellFlags(u16);

impl CellFlags {
    /// Bold (`SGR 1`).
    pub const BOLD: Self = Self(1);
    /// Italic (`SGR 3`).
    pub const ITALIC: Self = Self(1 << 1);
    /// Any underline style (single, double, curly, dotted, dashed).
    pub const UNDERLINE: Self = Self(1 << 2);
    /// Strikethrough (`SGR 9`).
    pub const STRIKEOUT: Self = Self(1 << 3);
    /// Swap foreground and background (`SGR 7`).
    pub const INVERSE: Self = Self(1 << 4);
    /// Faint (`SGR 2`).
    pub const DIM: Self = Self(1 << 5);
    /// Invisible text (`SGR 8`).
    pub const HIDDEN: Self = Self(1 << 6);
    /// The first cell of a double-width character.
    pub const WIDE: Self = Self(1 << 7);
    /// The second cell of a double-width character, or the padding cell left at the end of a
    /// line when a wide character wrapped to the next one. Renderers skip it.
    pub const WIDE_SPACER: Self = Self(1 << 8);
    /// Inside the current selection. Set by the snapshot, never by the program.
    pub const SELECTED: Self = Self(1 << 9);

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Sets the flags in `other`.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// One cell of the screen.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cell {
    /// The character in the cell; a space when empty.
    pub ch: char,
    /// Combining marks, variation selectors and ZWJ sequence members that render with `ch`.
    pub zerowidth: Vec<char>,
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Rendering attributes.
    pub flags: CellFlags,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            zerowidth: Vec::new(),
            fg: Color::Foreground,
            bg: Color::Background,
            flags: CellFlags::default(),
        }
    }
}

/// How the cursor is drawn (`DECSCUSR`). A hidden cursor is a [`Snapshot::cursor`] of `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    /// A filled cell.
    Block,
    /// An outlined cell.
    HollowBlock,
    /// A line under the cell.
    Underline,
    /// A vertical bar at the left of the cell.
    Beam,
}

/// The cursor's position on the visible screen and its shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Row in the visible screen, already adjusted for the scroll offset.
    pub row: usize,
    /// Column.
    pub col: usize,
    /// How to draw it.
    pub shape: CursorShape,
}

/// Terminal modes the program has switched on. They change how keys, pastes and mouse events are
/// encoded and whether the app handles the mouse itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modes {
    /// DECCKM (`?1`): arrows, Home and End send `SS3` (`ESC O`) instead of `CSI`.
    pub app_cursor: bool,
    /// DECKPAM: application keypad.
    pub app_keypad: bool,
    /// Mode 2004: pastes are wrapped in `ESC[200~` / `ESC[201~`.
    pub bracketed_paste: bool,
    /// The alternate screen is active (full-screen programs), so there is no scrollback.
    pub alt_screen: bool,
    /// Mode 1004: the program wants `ESC[I` / `ESC[O` on focus changes.
    pub focus_events: bool,
    /// Mode 1007: on the alternate screen, the wheel should send arrow keys instead of scrolling.
    pub alternate_scroll: bool,
    /// Mouse mode 1000: report presses and releases.
    pub mouse_click: bool,
    /// Mouse mode 1002: also report motion while a button is held.
    pub mouse_drag: bool,
    /// Mouse mode 1003: report all motion.
    pub mouse_motion: bool,
    /// Mode 1006: SGR mouse encoding (`CSI < b;x;y M` / `m`).
    pub sgr_mouse: bool,
    /// Mode 1005: UTF-8 mouse encoding.
    pub utf8_mouse: bool,
}

impl Modes {
    /// Whether the program wants mouse events instead of local selection and scrolling.
    pub fn mouse_reporting(&self) -> bool {
        self.mouse_click || self.mouse_drag || self.mouse_motion
    }
}

/// A cell position in the visible screen (row 0 is the top row currently shown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridPoint {
    /// Row in the visible screen.
    pub row: usize,
    /// Column.
    pub col: usize,
}

/// Which half of a cell the pointer is over; decides whether that cell is included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellSide {
    /// The left half.
    Left,
    /// The right half.
    Right,
}

/// How a selection grows as the pointer moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// Character by character, wrapping across lines.
    Simple,
    /// A rectangle of cells.
    Block,
    /// Expands to whole words (double-click).
    Word,
    /// Expands to whole lines (triple-click).
    Line,
}

/// Which way [`Emulator::search`](crate::Emulator::search) walks from the previous match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDirection {
    /// Towards the top of the scrollback.
    Older,
    /// Towards the bottom of the screen.
    Newer,
}

/// A request to move the view through the scrollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    /// Scroll by this many lines; positive moves towards older output.
    Lines(i32),
    /// Up one screen.
    PageUp,
    /// Down one screen.
    PageDown,
    /// The oldest line in the scrollback.
    Top,
    /// Back to the live screen.
    Bottom,
}

/// A copy of the visible screen, taken under the emulator lock and rendered without it.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Columns in each row.
    pub cols: usize,
    /// The visible rows, top to bottom.
    pub rows: Vec<Vec<Cell>>,
    /// The cursor, or `None` when it's hidden or scrolled out of view.
    pub cursor: Option<Cursor>,
    /// How many lines the view is scrolled back from the live screen (0 = live).
    pub display_offset: usize,
    /// The program's terminal modes.
    pub modes: Modes,
    /// Whether anything is selected (the selected cells carry [`CellFlags::SELECTED`]).
    pub has_selection: bool,
    /// Palette entries changed at runtime via OSC 4/10/11: indices 0-255, 256 = foreground,
    /// 257 = background.
    pub color_overrides: Vec<(u16, Rgb)>,
}
