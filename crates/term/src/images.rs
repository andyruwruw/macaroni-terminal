//! Inline images: iTerm2's OSC 1337 `File=` sequence, as `imgcat` and most image-printing tools
//! send it.
//!
//! An image takes up a block of cells, starting at the cursor, as if it were text. The reader
//! thread gets the request from the [`crate::OscSniffer`] ([`ImageRequest`]), works out how many
//! cells it covers at the grid's cell size ([`ImageRequest::place`]), stores it in the emulator,
//! and feeds the emulator [`InlineImage::anchors`] in its place: a hidden OSC 8 link on the
//! block's first cell in every row, then the cursor moved to the right of the block's last row,
//! as iTerm2 leaves it. Like prompt marks, the anchors live in the cells, so they scroll, wrap
//! into the scrollback and get erased along with the text, and the image with them. A snapshot
//! lists the images whose anchors are on screen ([`ImagePlacement`]); the app draws them over
//! the text.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;

use crate::grid::GridSize;

/// The URI prefix of an image's anchor links: `macaroni-image:<id>:<row in the image>`.
pub const IMAGE_PREFIX: &str = "macaroni-image:";

/// Largest image accepted, encoded, in bytes; anything bigger is dropped.
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

/// Most rows an image may take, so one sequence can't scroll away the whole scrollback.
const MAX_ROWS: u32 = 500;

/// Gives every image an id no other image in any session has, so the app can cache what it
/// built from one by id alone.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One of an image's dimensions as the sequence asked for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dimension {
    /// `auto` (or left out): from the image itself.
    Auto,
    /// `N`: this many cells.
    Cells(f32),
    /// `Npx`: this many pixels.
    Pixels(f32),
    /// `N%`: this share of the terminal's width or height.
    Percent(f32),
}

impl Dimension {
    /// Parses `auto`, `N`, `Npx` or `N%`; anything else is `Auto`.
    fn parse(value: &str) -> Self {
        let value = value.trim();
        let number = |s: &str| s.trim().parse::<f32>().ok().filter(|n| *n > 0.0);
        if let Some(n) = value.strip_suffix("px").and_then(number) {
            Self::Pixels(n)
        } else if let Some(n) = value.strip_suffix('%').and_then(number) {
            Self::Percent(n)
        } else if let Some(n) = number(value) {
            Self::Cells(n)
        } else {
            Self::Auto
        }
    }

    /// In pixels, given the terminal's extent and one cell's in the same direction; `None` for
    /// `Auto`.
    fn pixels(self, total: f32, cell: f32) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Cells(n) => Some(n * cell),
            Self::Pixels(n) => Some(n),
            Self::Percent(p) => Some(total * p / 100.0),
        }
    }
}

/// An image a program sent to be shown inline, before it has a place on the grid.
#[derive(Clone, PartialEq)]
pub struct ImageRequest {
    /// The file's bytes (PNG, JPEG, GIF and so on), decoded from base64.
    pub data: Vec<u8>,
    /// The width asked for.
    pub width: Dimension,
    /// The height asked for.
    pub height: Dimension,
    /// Keep the image's shape inside the width and height asked for (the default), rather than
    /// stretching it to them.
    pub preserve_aspect: bool,
}

impl fmt::Debug for ImageRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageRequest")
            .field("bytes", &self.data.len())
            .field("width", &self.width)
            .field("height", &self.height)
            .field("preserve_aspect", &self.preserve_aspect)
            .finish()
    }
}

impl ImageRequest {
    /// Parses what follows `1337;` in the sequence: `File=<key>=<value>;...:<base64>`. `None`
    /// unless it's a file with `inline=1` (anything else is a download, which isn't supported),
    /// or when the payload doesn't decode or is too big.
    pub fn parse(rest: &[u8]) -> Option<Self> {
        let rest = rest.strip_prefix(b"File=")?;
        let colon = rest.iter().position(|&b| b == b':')?;
        let (args, payload) = (&rest[..colon], &rest[colon + 1..]);
        if payload.len() > MAX_IMAGE_BYTES / 3 * 4 + 4 {
            return None;
        }
        let mut request = Self {
            data: Vec::new(),
            width: Dimension::Auto,
            height: Dimension::Auto,
            preserve_aspect: true,
        };
        let mut inline = false;
        for arg in String::from_utf8_lossy(args).split(';') {
            match arg.split_once('=') {
                Some(("inline", v)) => inline = v.trim() == "1",
                Some(("width", v)) => request.width = Dimension::parse(v),
                Some(("height", v)) => request.height = Dimension::parse(v),
                Some(("preserveAspectRatio", v)) => request.preserve_aspect = v.trim() != "0",
                _ => {}
            }
        }
        if !inline {
            return None;
        }
        // Some senders wrap the base64 in lines.
        let payload: Vec<u8> = payload
            .iter()
            .copied()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        request.data = base64::engine::general_purpose::STANDARD
            .decode(&payload)
            .ok()
            .filter(|d| !d.is_empty())?;
        Some(request)
    }

    /// The image on a grid of `size`: how many cells it covers. `None` when the data isn't an
    /// image this build can read.
    ///
    /// The size asked for is worked out in pixels; with only one dimension given (or both,
    /// keeping the aspect ratio) the other follows from the image's shape. An image wider than
    /// the terminal is scaled down to fit it; a tall one is left to scroll, up to [`MAX_ROWS`].
    pub fn place(self, size: GridSize) -> Option<InlineImage> {
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(&self.data))
            .with_guessed_format()
            .ok()?
            .into_dimensions()
            .ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        let (cell_w, cell_h) = (
            f32::from(size.cell_width.max(1)),
            f32::from(size.cell_height.max(1)),
        );
        let (area_w, area_h) = (
            f32::from(size.cols.max(1)) * cell_w,
            f32::from(size.rows.max(1)) * cell_h,
        );
        let (iw, ih) = (width as f32, height as f32);
        let (mut w, mut h) = match (
            self.width.pixels(area_w, cell_w),
            self.height.pixels(area_h, cell_h),
        ) {
            (Some(w), Some(h)) if self.preserve_aspect => {
                let scale = (w / iw).min(h / ih);
                (iw * scale, ih * scale)
            }
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, ih * w / iw),
            (None, Some(h)) => (iw * h / ih, h),
            (None, None) => (iw, ih),
        };
        if w > area_w {
            h *= area_w / w;
            w = area_w;
        }
        let cols = ((w / cell_w).ceil() as u32).clamp(1, u32::from(size.cols.max(1)));
        let rows = ((h / cell_h).ceil() as u32).clamp(1, MAX_ROWS);
        Some(InlineImage {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            data: self.data.into(),
            width,
            height,
            cols: cols as u16,
            rows: rows as u16,
            stretch: !self.preserve_aspect,
        })
    }
}

/// An image on the grid: its file and the block of cells it covers.
#[derive(Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// Unique among every session's images.
    pub id: u64,
    /// The file's bytes, still encoded.
    pub data: Arc<[u8]>,
    /// Its width in pixels.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// Columns of the block it's drawn in.
    pub cols: u16,
    /// Rows of the block it's drawn in.
    pub rows: u16,
    /// Fill the block exactly instead of fitting inside it with its own shape.
    pub stretch: bool,
}

impl fmt::Debug for InlineImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InlineImage")
            .field("id", &self.id)
            .field("bytes", &self.data.len())
            .field("pixels", &(self.width, self.height))
            .field("cells", &(self.cols, self.rows))
            .finish()
    }
}

impl InlineImage {
    /// What the emulator is fed in the image's place: in each row of the block, a blank cell
    /// carrying the anchor link (`macaroni-image:<id>:<row>`), the cursor stepping back onto
    /// it and then down a line (scrolling when at the bottom, which is what makes room); after
    /// the last row, the cursor moves past the block's right edge.
    pub fn anchors(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for row in 0..self.rows {
            if row > 0 {
                // A bare line feed: down a row in the same column.
                out.push(b'\n');
            }
            out.extend_from_slice(
                format!(
                    "\x1b]8;id=mci{id}-{row};{IMAGE_PREFIX}{id}:{row}\x07 \x1b]8;;\x07\x1b[D",
                    id = self.id
                )
                .as_bytes(),
            );
        }
        out.extend_from_slice(format!("\x1b[{}C", self.cols).as_bytes());
        out
    }
}

/// Reads an anchor link's URI: the image's id and which of its rows the cell is in.
pub(crate) fn anchor(uri: &str) -> Option<(u64, u16)> {
    let (id, row) = uri.strip_prefix(IMAGE_PREFIX)?.split_once(':')?;
    Some((id.parse().ok()?, row.parse().ok()?))
}

/// An image with at least one row on screen, for the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePlacement {
    /// The image.
    pub image: Arc<InlineImage>,
    /// The visible row its top is on; negative when it starts above the view.
    pub row: i32,
    /// The column its left edge is on.
    pub col: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4x2 PNG.
    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(4, 2)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    /// A grid of 80x24 cells, each 10x20 pixels.
    const SIZE: GridSize = GridSize {
        cols: 80,
        rows: 24,
        cell_width: 10,
        cell_height: 20,
    };

    /// The sequence's text after `1337;` for `png` with `args`.
    fn sequence(args: &str) -> Vec<u8> {
        let data = base64::engine::general_purpose::STANDARD.encode(png());
        format!("File={args}:{data}").into_bytes()
    }

    #[test]
    fn only_inline_files_parse() {
        assert!(ImageRequest::parse(&sequence("inline=1")).is_some());
        assert!(ImageRequest::parse(&sequence("name=eA==;inline=0")).is_none());
        assert!(ImageRequest::parse(b"File=inline=1:!!!").is_none());
        assert!(ImageRequest::parse(b"SetMark").is_none());
    }

    #[test]
    fn sizes_come_from_the_image_or_the_request() {
        let place = |args: &str| {
            let image = ImageRequest::parse(&sequence(args))
                .unwrap()
                .place(SIZE)
                .unwrap();
            (image.cols, image.rows)
        };
        // 4x2 pixels is under one cell.
        assert_eq!(place("inline=1"), (1, 1));
        // 40 columns wide is 400 pixels, so 200 high: 10 rows.
        assert_eq!(place("inline=1;width=40"), (40, 10));
        assert_eq!(place("inline=1;width=100%"), (80, 20));
        assert_eq!(place("inline=1;height=100px"), (20, 5));
        // Kept to its shape inside 40x40 cells, or stretched to them.
        assert_eq!(place("inline=1;width=40;height=40"), (40, 10));
        assert_eq!(
            place("inline=1;width=40;height=40;preserveAspectRatio=0"),
            (40, 40)
        );
        // Wider than the terminal shrinks to fit.
        assert_eq!(place("inline=1;width=2000px"), (80, 20));
    }

    #[test]
    fn anchors_reserve_the_block() {
        let image =
            ImageRequest::parse(&sequence("inline=1;width=3;height=2;preserveAspectRatio=0"))
                .unwrap()
                .place(SIZE)
                .unwrap();
        let anchors = String::from_utf8(image.anchors()).unwrap();
        assert_eq!(anchors.matches(IMAGE_PREFIX).count(), 2);
        assert_eq!(anchors.matches('\n').count(), 1);
        assert!(anchors.ends_with("\x1b[3C"));
        let uri = format!("{IMAGE_PREFIX}{}:1", image.id);
        assert_eq!(anchor(&uri), Some((image.id, 1)));
    }
}
