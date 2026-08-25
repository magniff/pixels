//! Bitmap text: a face is a table of bits, and drawing is blitting them.
//!
//! Using bitmap fonts is not just an aesthetic choice. It deletes glyph
//! rasterisation, hinting, subpixel positioning and font shaping from the
//! problem entirely — text becomes bit-blitting, and every glyph lands on exact
//! pixel boundaries by construction.
//!
//! The trade-off is real and worth stating plainly: this handles ASCII and
//! nothing else. Scripts that need shaping, and IME input, are out of scope.
//!
//! Which face is in use is a property of the process rather than something
//! threaded through every call. Drawing text is a free function reached from
//! everywhere — the caret, the scrollbar, a menu — and a parameter for it would
//! appear in a hundred signatures to be passed straight through.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::canvas::Canvas;
use crate::color::Color;

mod builtin;
mod cozette;
mod creep;
mod gohu;
mod tamzen;

/// One bitmap face.
///
/// Ninety-five glyphs of printable ASCII, `glyph_h` rows each, one bit per
/// column with the leftmost in bit `glyph_w - 1`. Baked from a BDF at
/// authoring time by `tools/bdf2rs.py`, or drawn by hand.
pub struct Face {
    pub name: &'static str,
    /// Width of the inked cell. Some faces overhang their advance by a pixel,
    /// which is how a bitmap font joins box drawing up.
    pub glyph_w: i32,
    pub glyph_h: i32,
    /// Horizontal step from one glyph origin to the next.
    pub advance: i32,
    /// Vertical step from one baseline to the next.
    pub line_h: i32,
    pub rows: &'static [u16],
}

/// Every face built in, in the order an application should offer them.
pub static FACES: &[&Face] = &[
    &builtin::BUILTIN,
    &creep::CREEP,
    &tamzen::TAMZEN,
    &cozette::COZETTE,
    &gohu::GOHU,
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The face being drawn with.
pub fn face() -> &'static Face {
    FACES[CURRENT.load(Ordering::Relaxed).min(FACES.len() - 1)]
}

/// Draw with this face from now on. Out-of-range indices keep the current one.
pub fn use_face(i: usize) {
    if i < FACES.len() {
        CURRENT.store(i, Ordering::Relaxed);
    }
}

/// The index of the face with this name, however it is typed.
pub fn face_named(name: &str) -> Option<usize> {
    FACES.iter().position(|f| f.name.eq_ignore_ascii_case(name))
}

/// Ink width of a glyph cell.
pub fn glyph_w() -> i32 {
    face().glyph_w
}

/// Ink height of a glyph cell.
pub fn glyph_h() -> i32 {
    face().glyph_h
}

/// Horizontal step from one glyph origin to the next.
pub fn advance() -> i32 {
    face().advance
}

/// Vertical step from one baseline to the next.
pub fn line_h() -> i32 {
    face().line_h
}

const FIRST: u32 = 32;

/// The rows of one glyph in the current face, falling back to `?`.
pub fn glyph(c: char) -> &'static [u16] {
    let f = face();
    let h = f.glyph_h as usize;
    let i = (c as u32).wrapping_sub(FIRST) as usize;
    let i = if (i + 1) * h <= f.rows.len() {
        i
    } else {
        ('?' as u32 - FIRST) as usize
    };
    &f.rows[i * h..(i + 1) * h]
}

/// Ink width of a single line: the glyphs plus the tracking *between* them,
/// with none trailing.
///
/// Not `n * advance()`. That would include the tracking after the final glyph,
/// which is exactly the sliver that makes centred text sit a pixel left of
/// where it should.
fn line_width(line: &str) -> i32 {
    let n = line.chars().count() as i32;
    if n == 0 {
        0
    } else {
        (n - 1) * advance() + glyph_w()
    }
}

/// Width of the widest line in `text`.
pub fn text_width(text: &str) -> i32 {
    text.split('\n').map(line_width).max().unwrap_or(0)
}

/// Total height of `text`, counting newlines.
pub fn text_height(text: &str) -> i32 {
    let lines = text.split('\n').count() as i32;
    glyph_h() + (lines - 1) * line_h()
}

/// Draw one glyph with its top-left at `(x, y)`.
pub fn draw_char(canvas: &mut Canvas, x: i32, y: i32, c: char, color: Color) {
    let w = glyph_w();
    for (dy, row) in glyph(c).iter().enumerate() {
        if *row == 0 {
            continue;
        }
        for dx in 0..w {
            if row & (1 << (w - 1 - dx)) != 0 {
                canvas.set_px(x + dx, y + dy as i32, color);
            }
        }
    }
}

/// Draw `text` with its top-left at `(x, y)`. Handles `\n`.
///
/// Returns the **advance**: where the next glyph would start, not how far the
/// ink reached. Those differ by the tracking, and returning the ink extent
/// makes every caller that accumulates widths — laying out styled runs, say —
/// creep a pixel to the left per run until the text falls off the character
/// grid. Use [`text_width`] when you want the ink, as centring does.
pub fn draw_text(canvas: &mut Canvas, x: i32, y: i32, text: &str, color: Color) -> i32 {
    let mut cy = y;
    for line in text.split('\n') {
        let mut cx = x;
        for c in line.chars() {
            draw_char(canvas, cx, cy, c, color);
            cx += advance();
        }
        cy += line_h();
    }
    advance_width(text)
}

/// How far `text` advances the pen: one advance per character of its widest
/// line, with no tracking trimmed off the end.
pub fn advance_width(text: &str) -> i32 {
    text.split('\n')
        .map(|l| l.chars().count() as i32 * advance())
        .max()
        .unwrap_or(0)
}

/// Ink extent of `text`, one pixel wider when `bold` — the double-strike
/// reaches a column further right.
///
/// The *advance* is unaffected by weight on purpose: bold text has to sit on
/// the same character grid as everything else, or a bold run in the middle of a
/// line would shunt the rest of it out of step with the caret.
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
    advance_width(text)
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
