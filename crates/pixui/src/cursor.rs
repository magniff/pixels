//! A mouse pointer drawn into the canvas.
//!
//! The system pointer is rendered by the compositor at the display's real
//! resolution, so next to chunky upscaled pixels it looks like it belongs to a
//! different program. Hiding it and drawing our own puts the pointer on the
//! same grid as everything else — it magnifies with the UI, and it re-colours
//! with the theme.
//!
//! Sprites are written out as text: `X` is outline, `#` is fill, `.` is
//! transparent. A pointer is a drawing, and this is the format you can actually
//! edit a drawing in.

use crate::canvas::Canvas;
use crate::color::Color;
use crate::geom::Point;
use crate::input::Cursor;

/// A pointer shape and the pixel within it that sits under the true position.
pub struct Sprite {
    pub rows: &'static [&'static str],
    /// The pixel that lands exactly on the pointer position.
    pub hotspot: (i32, i32),
}

#[rustfmt::skip]
const ARROW: Sprite = Sprite {
    rows: &[
        "X........",
        "XX.......",
        "X#X......",
        "X##X.....",
        "X###X....",
        "X####X...",
        "X#####X..",
        "X######X.",
        "X#######X",
        "X####XXXX",
        "X##X##X..",
        "X#X.X##X.",
        "XX..X##X.",
        ".....XXX.",
    ],
    hotspot: (0, 0),
};

#[rustfmt::skip]
const HAND: Sprite = Sprite {
    rows: &[
        "..XX.....",
        "..X#X....",
        "..X#X....",
        "..X#X....",
        "..X#XX...",
        "..X#X#XX.",
        "..X#X#X#X",
        "..X#####X",
        ".XX#####X",
        ".X######X",
        ".X#####X.",
        "..XXXXX..",
    ],
    hotspot: (3, 0),
};

#[rustfmt::skip]
const GRAB: Sprite = Sprite {
    rows: &[
        "..XXX....",
        ".X###XX..",
        ".X#####X.",
        "XX######X",
        "X#######X",
        "X#######X",
        "X######X.",
        ".X#####X.",
        "..XXXXX..",
    ],
    hotspot: (4, 4),
};

/// Filled, like the rest. Drawn as bare outline it vanished against a dark
/// editor pane, which is precisely where a text cursor is needed.
#[rustfmt::skip]
const BEAM: Sprite = Sprite {
    rows: &[
        "XXXXXXX",
        "X#####X",
        "XXX#XXX",
        "..X#X..",
        "..X#X..",
        "..X#X..",
        "..X#X..",
        "..X#X..",
        "..X#X..",
        "..X#X..",
        "XXX#XXX",
        "X#####X",
        "XXXXXXX",
    ],
    hotspot: (3, 6),
};

/// A filled body with a derived outline, like the other pointers. Drawn as
/// pure outline it came out dark, which is the opposite of what a pointer
/// wants to be over a dark editor pane.
#[rustfmt::skip]
const RESIZE_H: Sprite = Sprite {
    rows: &[
        ".............",
        "..XXX...XXX..",
        ".XX#X...X#XX.",
        "XX##XXXXX##XX",
        "X###########X",
        "XX##XXXXX##XX",
        ".XX#X...X#XX.",
        "..XXX...XXX..",
        ".............",
    ],
    hotspot: (6, 4),
};

/// A filled body with a derived outline, like the other pointers. Drawn as
/// pure outline it came out dark, which is the opposite of what a pointer
/// wants to be over a dark editor pane.
#[rustfmt::skip]
const RESIZE_V: Sprite = Sprite {
    rows: &[
        "...XXX...",
        "..XX#XX..",
        ".XX###XX.",
        ".X#####X.",
        ".XXX#XXX.",
        "...X#X...",
        "...X#X...",
        "...X#X...",
        ".XXX#XXX.",
        ".X#####X.",
        ".XX###XX.",
        "..XX#XX..",
        "...XXX...",
    ],
    hotspot: (4, 6),
};

/// The sprite for a pointer shape.
pub fn sprite(kind: Cursor) -> &'static Sprite {
    match kind {
        Cursor::Default => &ARROW,
        Cursor::Pointer => &HAND,
        Cursor::Grab => &GRAB,
        Cursor::Text => &BEAM,
        Cursor::ResizeH => &RESIZE_H,
        Cursor::ResizeV => &RESIZE_V,
    }
}

/// Draw the pointer with its hotspot at `at`.
///
/// Called after everything else, so the pointer is never drawn over — which is
/// also why it deliberately ignores any clip that was in force during the
/// frame.
pub fn draw(canvas: &mut Canvas, at: Point, kind: Cursor, fill: Color, outline: Color) {
    let sprite = sprite(kind);
    for (dy, row) in sprite.rows.iter().enumerate() {
        for (dx, ch) in row.chars().enumerate() {
            let color = match ch {
                'X' => outline,
                '#' => fill,
                _ => continue,
            };
            canvas.set_px(
                at.x - sprite.hotspot.0 + dx as i32,
                at.y - sprite.hotspot.1 + dy as i32,
                color,
            );
        }
    }
}
