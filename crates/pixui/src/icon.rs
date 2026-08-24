//! Small monochrome icons.
//!
//! Written out as text like the pointer sprites: `#` is ink, anything else is
//! nothing. At seven pixels a side an icon is a drawing, not a path, and this
//! is the format a drawing can be edited in.

use crate::canvas::Canvas;
use crate::color::Color;

/// A magnifying glass, for search fields.
#[rustfmt::skip]
pub const SEARCH: &[&str] = &[
    ".###..",
    "#...#.",
    "#...#.",
    "#...#.",
    ".###..",
    "...##.",
    "....##",
];

/// A cross, for clearing or dismissing.
#[rustfmt::skip]
pub const CROSS: &[&str] = &[
    "#...#",
    ".#.#.",
    "..#..",
    ".#.#.",
    "#...#",
];

/// A tick.
#[rustfmt::skip]
pub const CHECK: &[&str] = &[
    ".....#",
    "....##",
    "#..##.",
    "####..",
    ".##...",
];

/// A right-pointing chevron, for disclosure.
#[rustfmt::skip]
pub const CHEVRON: &[&str] = &[
    "#..",
    "##.",
    ".##",
    "##.",
    "#..",
];

/// The pixel size of an icon.
pub fn size(rows: &[&str]) -> (i32, i32) {
    let w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as i32;
    (w, rows.len() as i32)
}

/// Draw an icon with its top-left at `(x, y)`.
pub fn draw(canvas: &mut Canvas, x: i32, y: i32, rows: &[&str], color: Color) {
    for (dy, row) in rows.iter().enumerate() {
        for (dx, ch) in row.chars().enumerate() {
            if ch == '#' {
                canvas.set_px(x + dx as i32, y + dy as i32, color);
            }
        }
    }
}

/// Draw an icon centred in `rect`.
pub fn draw_centered(canvas: &mut Canvas, rect: crate::geom::Rect, rows: &[&str], color: Color) {
    let (w, h) = size(rows);
    draw(
        canvas,
        rect.x + (rect.w - w) / 2,
        rect.y + (rect.h - h) / 2,
        rows,
        color,
    );
}
