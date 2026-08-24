//! A built-in 5x7 bitmap font covering printable ASCII.
//!
//! Using a bitmap font is not just an aesthetic choice. It deletes glyph
//! rasterisation, hinting, subpixel positioning and font shaping from the
//! problem entirely — text becomes bit-blitting, and every glyph lands on exact
//! pixel boundaries by construction.
//!
//! The trade-off is real and worth stating plainly: this handles ASCII and
//! nothing else. Scripts that need shaping, and IME input, are out of scope for
//! the built-in font.
//!
//! Each glyph is seven rows; bit 4 of each row is the leftmost column, so the
//! literals below are legible as pixels if you read `1` as ink.
//!
//! Seven rows leaves nowhere to put a descender, so `g j p q y` sit their bowl
//! at the x-height and drop the tail onto the baseline row. Filling the whole
//! cell instead — the obvious thing to do — makes them read as capitals.

use crate::canvas::Canvas;
use crate::color::Color;

/// Ink width of a glyph cell.
pub const GLYPH_W: i32 = 5;
/// Ink height of a glyph cell.
pub const GLYPH_H: i32 = 7;
/// Horizontal step from one glyph origin to the next (one column of gap).
pub const ADVANCE: i32 = 6;
/// Vertical step from one baseline to the next.
pub const LINE_H: i32 = 9;

const FIRST: u32 = 32;

#[rustfmt::skip]
const GLYPHS: [[u8; 7]; 95] = [
    [0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b00000], // ' '
    [0b00100,0b00100,0b00100,0b00100,0b00100,0b00000,0b00100], // !
    [0b01010,0b01010,0b00000,0b00000,0b00000,0b00000,0b00000], // "
    [0b01010,0b01010,0b11111,0b01010,0b11111,0b01010,0b01010], // #
    [0b00100,0b01111,0b10100,0b01110,0b00101,0b11110,0b00100], // $
    [0b11001,0b11001,0b00010,0b00100,0b01000,0b10011,0b10011], // %
    [0b01100,0b10010,0b10100,0b01000,0b10101,0b10010,0b01101], // &
    [0b00100,0b00100,0b00000,0b00000,0b00000,0b00000,0b00000], // '
    [0b00010,0b00100,0b01000,0b01000,0b01000,0b00100,0b00010], // (
    [0b01000,0b00100,0b00010,0b00010,0b00010,0b00100,0b01000], // )
    [0b00000,0b10101,0b01110,0b11111,0b01110,0b10101,0b00000], // *
    [0b00000,0b00100,0b00100,0b11111,0b00100,0b00100,0b00000], // +
    [0b00000,0b00000,0b00000,0b00000,0b00000,0b00100,0b01000], // ,
    [0b00000,0b00000,0b00000,0b11111,0b00000,0b00000,0b00000], // -
    [0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b00100], // .
    [0b00001,0b00001,0b00010,0b00100,0b01000,0b10000,0b10000], // /
    [0b01110,0b10001,0b10011,0b10101,0b11001,0b10001,0b01110], // 0
    [0b00100,0b01100,0b00100,0b00100,0b00100,0b00100,0b01110], // 1
    [0b01110,0b10001,0b00001,0b00010,0b00100,0b01000,0b11111], // 2
    [0b11111,0b00010,0b00100,0b00010,0b00001,0b10001,0b01110], // 3
    [0b00010,0b00110,0b01010,0b10010,0b11111,0b00010,0b00010], // 4
    [0b11111,0b10000,0b11110,0b00001,0b00001,0b10001,0b01110], // 5
    [0b00110,0b01000,0b10000,0b11110,0b10001,0b10001,0b01110], // 6
    [0b11111,0b10001,0b00001,0b00010,0b00100,0b00100,0b00100], // 7
    [0b01110,0b10001,0b10001,0b01110,0b10001,0b10001,0b01110], // 8
    [0b01110,0b10001,0b10001,0b01111,0b00001,0b00010,0b01100], // 9
    [0b00000,0b00000,0b00100,0b00000,0b00000,0b00100,0b00000], // :
    [0b00000,0b00000,0b00100,0b00000,0b00100,0b00100,0b01000], // ;
    [0b00010,0b00100,0b01000,0b10000,0b01000,0b00100,0b00010], // <
    [0b00000,0b00000,0b11111,0b00000,0b11111,0b00000,0b00000], // =
    [0b01000,0b00100,0b00010,0b00001,0b00010,0b00100,0b01000], // >
    [0b01110,0b10001,0b00001,0b00010,0b00100,0b00000,0b00100], // ?
    [0b01110,0b10001,0b10111,0b10101,0b10111,0b10000,0b01110], // @
    [0b01110,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001], // A
    [0b11110,0b10001,0b10001,0b11110,0b10001,0b10001,0b11110], // B
    [0b01110,0b10001,0b10000,0b10000,0b10000,0b10001,0b01110], // C
    [0b11100,0b10010,0b10001,0b10001,0b10001,0b10010,0b11100], // D
    [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b11111], // E
    [0b11111,0b10000,0b10000,0b11110,0b10000,0b10000,0b10000], // F
    [0b01110,0b10001,0b10000,0b10111,0b10001,0b10001,0b01110], // G
    [0b10001,0b10001,0b10001,0b11111,0b10001,0b10001,0b10001], // H
    [0b01110,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110], // I
    [0b00111,0b00010,0b00010,0b00010,0b00010,0b10010,0b01100], // J
    [0b10001,0b10010,0b10100,0b11000,0b10100,0b10010,0b10001], // K
    [0b10000,0b10000,0b10000,0b10000,0b10000,0b10000,0b11111], // L
    [0b10001,0b11011,0b10101,0b10101,0b10001,0b10001,0b10001], // M
    [0b10001,0b11001,0b10101,0b10011,0b10001,0b10001,0b10001], // N
    [0b01110,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110], // O
    [0b11110,0b10001,0b10001,0b11110,0b10000,0b10000,0b10000], // P
    [0b01110,0b10001,0b10001,0b10001,0b10101,0b10010,0b01101], // Q
    [0b11110,0b10001,0b10001,0b11110,0b10100,0b10010,0b10001], // R
    [0b01111,0b10000,0b10000,0b01110,0b00001,0b00001,0b11110], // S
    [0b11111,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100], // T
    [0b10001,0b10001,0b10001,0b10001,0b10001,0b10001,0b01110], // U
    [0b10001,0b10001,0b10001,0b10001,0b10001,0b01010,0b00100], // V
    [0b10001,0b10001,0b10001,0b10101,0b10101,0b11011,0b10001], // W
    [0b10001,0b10001,0b01010,0b00100,0b01010,0b10001,0b10001], // X
    [0b10001,0b10001,0b01010,0b00100,0b00100,0b00100,0b00100], // Y
    [0b11111,0b00001,0b00010,0b00100,0b01000,0b10000,0b11111], // Z
    [0b01110,0b01000,0b01000,0b01000,0b01000,0b01000,0b01110], // [
    [0b10000,0b10000,0b01000,0b00100,0b00010,0b00001,0b00001], // backslash
    [0b01110,0b00010,0b00010,0b00010,0b00010,0b00010,0b01110], // ]
    [0b00100,0b01010,0b10001,0b00000,0b00000,0b00000,0b00000], // ^
    [0b00000,0b00000,0b00000,0b00000,0b00000,0b00000,0b11111], // _
    [0b01000,0b00100,0b00000,0b00000,0b00000,0b00000,0b00000], // `
    [0b00000,0b00000,0b01110,0b00001,0b01111,0b10001,0b01111], // a
    [0b10000,0b10000,0b11110,0b10001,0b10001,0b10001,0b11110], // b
    [0b00000,0b00000,0b01110,0b10001,0b10000,0b10001,0b01110], // c
    [0b00001,0b00001,0b01111,0b10001,0b10001,0b10001,0b01111], // d
    [0b00000,0b00000,0b01110,0b10001,0b11111,0b10000,0b01110], // e
    [0b00110,0b01001,0b01000,0b11100,0b01000,0b01000,0b01000], // f
    [0b00000,0b00000,0b01110,0b10001,0b01111,0b00001,0b01110], // g
    [0b10000,0b10000,0b11110,0b10001,0b10001,0b10001,0b10001], // h
    [0b00100,0b00000,0b01100,0b00100,0b00100,0b00100,0b01110], // i
    [0b00000,0b00010,0b00000,0b00010,0b00010,0b00010,0b01100], // j
    [0b10000,0b10000,0b10010,0b10100,0b11000,0b10100,0b10010], // k
    [0b01100,0b00100,0b00100,0b00100,0b00100,0b00100,0b01110], // l
    [0b00000,0b00000,0b11010,0b10101,0b10101,0b10001,0b10001], // m
    [0b00000,0b00000,0b11110,0b10001,0b10001,0b10001,0b10001], // n
    [0b00000,0b00000,0b01110,0b10001,0b10001,0b10001,0b01110], // o
    [0b00000,0b00000,0b11110,0b10001,0b10001,0b11110,0b10000], // p
    [0b00000,0b00000,0b01111,0b10001,0b10001,0b01111,0b00001], // q
    [0b00000,0b00000,0b10110,0b11001,0b10000,0b10000,0b10000], // r
    [0b00000,0b00000,0b01111,0b10000,0b01110,0b00001,0b11110], // s
    [0b01000,0b01000,0b11110,0b01000,0b01000,0b01001,0b00110], // t
    [0b00000,0b00000,0b10001,0b10001,0b10001,0b10011,0b01101], // u
    [0b00000,0b00000,0b10001,0b10001,0b10001,0b01010,0b00100], // v
    [0b00000,0b00000,0b10001,0b10001,0b10101,0b10101,0b01010], // w
    [0b00000,0b00000,0b10001,0b01010,0b00100,0b01010,0b10001], // x
    [0b00000,0b00000,0b10001,0b10001,0b01111,0b00001,0b01110], // y
    [0b00000,0b00000,0b11111,0b00010,0b00100,0b01000,0b11111], // z
    [0b00011,0b00100,0b00100,0b01000,0b00100,0b00100,0b00011], // {
    [0b00100,0b00100,0b00100,0b00100,0b00100,0b00100,0b00100], // |
    [0b11000,0b00100,0b00100,0b00010,0b00100,0b00100,0b11000], // }
    [0b00000,0b00000,0b01001,0b10101,0b10010,0b00000,0b00000], // ~
];

/// Bit rows for `c`, or the rows for `?` if it is not in the ASCII range.
pub fn glyph(c: char) -> &'static [u8; 7] {
    let i = (c as u32).wrapping_sub(FIRST) as usize;
    GLYPHS
        .get(i)
        .unwrap_or(&GLYPHS[('?' as u32 - FIRST) as usize])
}

/// Width in pixels of a single line, with no trailing gap.
fn line_width(line: &str) -> i32 {
    let n = line.chars().count() as i32;
    if n == 0 {
        0
    } else {
        n * ADVANCE - 1
    }
}

/// Width of the widest line in `text`.
pub fn text_width(text: &str) -> i32 {
    text.split('\n').map(line_width).max().unwrap_or(0)
}

/// Total height of `text`, counting newlines.
pub fn text_height(text: &str) -> i32 {
    let lines = text.split('\n').count() as i32;
    GLYPH_H + (lines - 1) * LINE_H
}

/// Draw one glyph with its top-left at `(x, y)`.
pub fn draw_char(canvas: &mut Canvas, x: i32, y: i32, c: char, color: Color) {
    let rows = glyph(c);
    for (dy, row) in rows.iter().enumerate() {
        if *row == 0 {
            continue;
        }
        for dx in 0..GLYPH_W {
            if row & (1 << (GLYPH_W - 1 - dx)) != 0 {
                canvas.set_px(x + dx, y + dy as i32, color);
            }
        }
    }
}

/// Draw `text` with its top-left at `(x, y)`. Handles `\n`. Returns the width drawn.
pub fn draw_text(canvas: &mut Canvas, x: i32, y: i32, text: &str, color: Color) -> i32 {
    let mut cy = y;
    for line in text.split('\n') {
        let mut cx = x;
        for c in line.chars() {
            draw_char(canvas, cx, cy, c, color);
            cx += ADVANCE;
        }
        cy += LINE_H;
    }
    text_width(text)
}

/// Width of `text`, one pixel wider per glyph when `bold`.
pub fn text_width_styled(text: &str, bold: bool) -> i32 {
    let w = text_width(text);
    if bold && w > 0 {
        w + 1
    } else {
        w
    }
}

/// Draw `text`, optionally emboldened.
///
/// A 5x7 font has no room for a second weight, so bold is faked the way bitmap
/// fonts have always faked it: draw the glyphs twice, one pixel apart. At this
/// scale that reads as heavier without turning into a blob, which is enough to
/// make a markdown heading stand out from body text.
pub fn draw_text_styled(
    canvas: &mut Canvas,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    bold: bool,
) -> i32 {
    draw_text(canvas, x, y, text, color);
    if bold {
        draw_text(canvas, x + 1, y, text, color);
    }
    text_width_styled(text, bold)
}

/// Draw `text` with a one-pixel drop shadow beneath it. On a busy background
/// this is the difference between legible and not.
pub fn draw_text_shadow(
    canvas: &mut Canvas,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
    shadow: Color,
) -> i32 {
    draw_text(canvas, x, y + 1, text, shadow);
    draw_text(canvas, x, y, text, color)
}
