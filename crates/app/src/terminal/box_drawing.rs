//! Box-drawing and block characters drawn as rectangles sized to the cell, so lines join across
//! cells regardless of which font the glyph would otherwise fall back to.
//!
//! [`super::render`] asks [`is_procedural`] while laying out a row and draws those cells with
//! [`rects`] (lines from the [`LINES`] table, blocks and shades) or, for the four rounded corners,
//! [`arc`]. Everything here is pure geometry in window coordinates, so it's unit tested.
//!
//! The geometry: a light line is an eighth of the cell's width (at least 1px), heavy is twice
//! that, double is two light lines a light width either side of the center. Line positions are
//! rounded to whole pixels the same way in every function, so arms of neighbouring cells, and
//! arcs and straight lines, meet exactly.

use iced::{Point, Rectangle, Size};

/// Line weights for U+2500..=U+257F as up, down, left, right: 0 none, 1 light, 2 heavy, 3 double.
/// Dashed variants are drawn solid. Empty entries are either arcs (see [`arc`]) or diagonals, which
/// fall back to the font.
const LINES: [&str; 128] = [
    "0011", "0022", "1100", "2200", "0011", "0022", "1100", "2200", // 2500
    "0011", "0022", "1100", "2200", "0101", "0102", "0201", "0202", // 2508
    "0110", "0120", "0210", "0220", "1001", "1002", "2001", "2002", // 2510
    "1010", "1020", "2010", "2020", "1101", "1102", "2101", "1201", // 2518
    "2201", "2102", "1202", "2202", "1110", "1120", "2110", "1210", // 2520
    "2210", "2120", "1220", "2220", "0111", "0121", "0112", "0122", // 2528
    "0211", "0221", "0212", "0222", "1011", "1021", "1012", "1022", // 2530
    "2011", "2021", "2012", "2022", "1111", "1121", "1112", "1122", // 2538
    "2111", "1211", "2211", "2121", "2112", "1221", "1212", "2122", // 2540
    "1222", "2221", "2212", "2222", "0011", "0022", "1100", "2200", // 2548
    "0033", "3300", "0103", "0301", "0303", "0130", "0310", "0330", // 2550
    "1003", "3001", "3003", "1030", "3010", "3030", "1103", "3301", // 2558
    "3303", "1130", "3310", "3330", "0133", "0311", "0333", "1033", // 2560
    "3011", "3033", "1133", "3311", "3333", "", "", "", // 2568
    "", "", "", "", "0010", "1000", "0001", "0100", // 2570
    "0020", "2000", "0002", "0200", "0012", "1200", "0021", "2100", // 2578
];

/// Whether `ch` is drawn here instead of by a font: the lines that have a [`LINES`] entry, the
/// rounded corners and the block elements (U+2580 to U+259F). Diagonals aren't.
pub fn is_procedural(ch: char) -> bool {
    match ch as u32 {
        0x256d..=0x2570 | 0x2580..=0x259f => true,
        cp @ 0x2500..=0x257f => !LINES[(cp - 0x2500) as usize].is_empty(),
        _ => false,
    }
}

/// Rectangles (with an opacity each) that draw `ch` inside `cell`. Empty for characters that
/// aren't drawn this way, including the arcs (see [`arc`]).
pub fn rects(ch: char, cell: Rectangle) -> Vec<(Rectangle, f32)> {
    match ch as u32 {
        cp @ 0x2500..=0x257f => lines(LINES[(cp - 0x2500) as usize], cell),
        cp @ 0x2580..=0x259f => blocks(cp, cell),
        _ => Vec::new(),
    }
}

/// A rounded outline much larger than the cell. Clipped to the cell, only one of its corners is
/// visible, and its straight edges continue into the neighbouring `─` and `│` lines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arc {
    /// The outline's rectangle, mostly outside the cell.
    pub bounds: Rectangle,
    /// Corner radius: half the cell's smaller side, so the curve fits in the cell.
    pub radius: f32,
    /// The border width, a light line's.
    pub thickness: f32,
}

/// The rounded corners `╭ ╮ ╯ ╰` as an [`Arc`] to stroke and clip to `cell` (one layer each);
/// `None` for anything else.
pub fn arc(ch: char, cell: Rectangle) -> Option<Arc> {
    let (opens_right, opens_down) = match ch {
        '╭' => (true, true),
        '╮' => (false, true),
        '╯' => (false, false),
        '╰' => (true, false),
        _ => return None,
    };
    let light = (cell.width / 8.0).round().max(1.0);
    // Centers of the light lines this arc joins, computed exactly as in `lines`.
    let x = (cell.x + cell.width / 2.0 - light / 2.0).round() + light / 2.0;
    let y = (cell.y + cell.height / 2.0 - light / 2.0).round() + light / 2.0;
    // Big enough that the outline's other corners are far outside the cell.
    let extent = (cell.width + cell.height) * 2.0;
    // iced draws the border inside the bounds, so its center sits half a thickness in.
    let left = if opens_right {
        x - light / 2.0
    } else {
        x + light / 2.0 - extent
    };
    let top = if opens_down {
        y - light / 2.0
    } else {
        y + light / 2.0 - extent
    };
    Some(Arc {
        bounds: Rectangle::new(Point::new(left, top), Size::new(extent, extent)),
        radius: (cell.width / 2.0).min(cell.height / 2.0),
        thickness: light,
    })
}

/// The rectangles for one [`LINES`] entry: an arm from the cell's center to each edge with a
/// weight. Empty for an empty entry.
fn lines(weights: &str, cell: Rectangle) -> Vec<(Rectangle, f32)> {
    let w: Vec<u8> = weights.bytes().map(|b| b - b'0').collect();
    if w.len() != 4 {
        return Vec::new();
    }
    let light = (cell.width / 8.0).round().max(1.0);
    let thickness = |weight: u8| if weight == 2 { light * 2.0 } else { light };
    // The left and top edges of a light line through the center, on whole pixels so every cell
    // (and `arc`) puts it in the same place.
    let cx = (cell.x + cell.width / 2.0 - light / 2.0).round();
    let cy = (cell.y + cell.height / 2.0 - light / 2.0).round();
    let gap = light;
    let mut out = Vec::new();

    let mut vertical = |y0: f32, y1: f32, weight: u8| {
        let t = thickness(weight);
        // Heavy lines widen around the light line's center.
        let x = cx - (t - light) / 2.0;
        let offsets: &[f32] = if weight == 3 { &[-gap, gap] } else { &[0.0] };
        for dx in offsets {
            out.push(Rectangle::new(
                Point::new(x + dx, y0),
                Size::new(t, y1 - y0),
            ));
        }
    };
    let [up, down, left, right] = [w[0], w[1], w[2], w[3]];
    // Vertical arms overlap the center by a light width so up and down meet without a gap.
    if up > 0 {
        vertical(cell.y, cy + light, up);
    }
    if down > 0 {
        vertical(cy, cell.y + cell.height, down);
    }

    let mut horizontal = |x0: f32, x1: f32, weight: u8| {
        let t = thickness(weight);
        let y = cy - (t - light) / 2.0;
        let offsets: &[f32] = if weight == 3 { &[-gap, gap] } else { &[0.0] };
        for dy in offsets {
            out.push(Rectangle::new(
                Point::new(x0, y + dy),
                Size::new(x1 - x0, t),
            ));
        }
    };
    // The left arm runs past the center far enough to cover a heavy vertical, so a corner or
    // tee has no notch where the arms meet.
    let overlap = if up.max(down) == 2 {
        light * 2.0
    } else {
        light
    };
    if left > 0 {
        horizontal(cell.x, cx + overlap, left);
    }
    if right > 0 {
        horizontal(cx, cell.x + cell.width, right);
    }

    out.into_iter().map(|r| (r, 1.0)).collect()
}

/// The rectangles for a block element (U+2580 to U+259F): eighths from the bottom or left,
/// halves, quadrants, and the three shades as the full cell at partial opacity.
fn blocks(cp: u32, cell: Rectangle) -> Vec<(Rectangle, f32)> {
    let Rectangle {
        x,
        y,
        width: w,
        height: h,
    } = cell;
    let rect = |fx: f32, fy: f32, fw: f32, fh: f32| {
        Rectangle::new(
            Point::new(x + w * fx, y + h * fy),
            Size::new(w * fw, h * fh),
        )
    };
    let quads = |ul: bool, ur: bool, ll: bool, lr: bool| {
        [
            (ul, rect(0.0, 0.0, 0.5, 0.5)),
            (ur, rect(0.5, 0.0, 0.5, 0.5)),
            (ll, rect(0.0, 0.5, 0.5, 0.5)),
            (lr, rect(0.5, 0.5, 0.5, 0.5)),
        ]
        .into_iter()
        .filter_map(|(on, r)| on.then_some((r, 1.0)))
        .collect()
    };
    match cp {
        0x2580 => vec![(rect(0.0, 0.0, 1.0, 0.5), 1.0)],
        0x2581..=0x2588 => {
            let f = (cp - 0x2580) as f32 / 8.0;
            vec![(rect(0.0, 1.0 - f, 1.0, f), 1.0)]
        }
        0x2589..=0x258f => {
            let f = (0x2590 - cp) as f32 / 8.0;
            vec![(rect(0.0, 0.0, f, 1.0), 1.0)]
        }
        0x2590 => vec![(rect(0.5, 0.0, 0.5, 1.0), 1.0)],
        0x2591 => vec![(cell, 0.25)],
        0x2592 => vec![(cell, 0.5)],
        0x2593 => vec![(cell, 0.75)],
        0x2594 => vec![(rect(0.0, 0.0, 1.0, 0.125), 1.0)],
        0x2595 => vec![(rect(0.875, 0.0, 0.125, 1.0), 1.0)],
        0x2596 => quads(false, false, true, false),
        0x2597 => quads(false, false, false, true),
        0x2598 => quads(true, false, false, false),
        0x2599 => quads(true, false, true, true),
        0x259a => quads(true, false, false, true),
        0x259b => quads(true, true, true, false),
        0x259c => quads(true, true, false, true),
        0x259d => quads(false, true, false, false),
        0x259e => quads(false, true, true, false),
        0x259f => quads(false, true, true, true),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 8.0,
        height: 16.0,
    };

    fn union(ch: char) -> Rectangle {
        rects(ch, CELL)
            .into_iter()
            .map(|(r, _)| r)
            .reduce(|a, b| a.union(&b))
            .unwrap()
    }

    #[test]
    fn horizontal_line_spans_the_cell() {
        let r = union('─');
        assert_eq!((r.x, r.width), (0.0, 8.0));
        assert!(r.height <= 2.0);
    }

    #[test]
    fn vertical_line_spans_the_cell() {
        let r = union('│');
        assert_eq!((r.y, r.height), (0.0, 16.0));
        assert!(r.width <= 2.0);
    }

    #[test]
    fn cross_has_four_arms() {
        assert_eq!(rects('┼', CELL).len(), 4);
        assert_eq!(rects('╬', CELL).len(), 8);
    }

    #[test]
    fn blocks_and_shades() {
        assert_eq!(rects('█', CELL), vec![(CELL, 1.0)]);
        assert_eq!(rects('▒', CELL), vec![(CELL, 0.5)]);
        assert_eq!(rects('▙', CELL).len(), 3);
    }

    #[test]
    fn arcs_line_up_with_straight_lines() {
        let vertical = union('│');
        let horizontal = union('─');
        let v_center = vertical.x + vertical.width / 2.0;
        let h_center = horizontal.y + horizontal.height / 2.0;

        let down_right = arc('╭', CELL).unwrap();
        assert_eq!(down_right.bounds.x + down_right.thickness / 2.0, v_center);
        assert_eq!(down_right.bounds.y + down_right.thickness / 2.0, h_center);
        assert!(down_right.bounds.x + down_right.bounds.width > CELL.x + CELL.width);
        assert!(down_right.bounds.y + down_right.bounds.height > CELL.y + CELL.height);

        let up_left = arc('╯', CELL).unwrap();
        let right_edge = up_left.bounds.x + up_left.bounds.width;
        let bottom_edge = up_left.bounds.y + up_left.bounds.height;
        assert_eq!(right_edge - up_left.thickness / 2.0, v_center);
        assert_eq!(bottom_edge - up_left.thickness / 2.0, h_center);
        assert!(up_left.bounds.x < CELL.x && up_left.bounds.y < CELL.y);

        assert!(rects('╭', CELL).is_empty());
        assert!(arc('┌', CELL).is_none());
    }

    #[test]
    fn diagonals_fall_back_to_the_font() {
        assert!(!is_procedural('╱'));
        assert!(is_procedural('╭'));
        assert!(!is_procedural('a'));
    }
}
