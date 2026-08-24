//! Rendering a parsed markdown document.
//!
//! The source view highlights *lines*, because that is what an editor draws and
//! what a caret sits in. A rendering has to work in blocks: a paragraph is not
//! a line, a list is not its items, and a table is not a row. So this measures
//! each block, allocates that much room, and draws into it.
//!
//! Measuring and drawing are deliberately the same arithmetic in the same
//! order. A layout that measures one way and paints another is a layout that
//! will eventually disagree with itself.

use pixui::{font, icon, Color, Rect, Theme, Ui};

use crate::markdown::{slice_spans, wrap_ranges};
use crate::markdown::{Block, CellAlign, Item, Marker, Span, Tok};

const LINE_H: i32 = font::LINE_H;
const ADVANCE: i32 = font::ADVANCE;

/// Space under each kind of block, so the document breathes without a blank
/// line having to be typed for it.
const PARA_GAP: i32 = 5;
const HEADING_TOP: i32 = 6;

/// Everything the layout needs that does not change between blocks.
struct Ctx {
    th: Theme,
    /// Width available for content, in pixels.
    width: i32,
}

impl Ctx {
    /// How many characters fit in `width` pixels, less an indent.
    fn cols(&self, indent: i32) -> usize {
        (((self.width - indent) / ADVANCE).max(4)) as usize
    }
}

/// Wrap styled runs into lines.
///
/// Wrapping is computed from the concatenated text and the runs are then sliced
/// to match, so the line breaks and the styling cannot disagree about where a
/// character is.
fn wrap(spans: &[Span], cols: usize) -> Vec<Vec<Span>> {
    let text: String = spans.iter().map(|s| s.text.as_str()).collect();
    wrap_ranges(&text, cols)
        .into_iter()
        .map(|(a, b)| slice_spans(spans, a, b))
        .collect()
}

fn token_color(th: &Theme, tok: Tok) -> Color {
    match tok {
        Tok::Text | Tok::Bold => th.ink_light,
        Tok::Heading => th.accent.hi,
        Tok::Italic => th.info.hi,
        Tok::Code => th.positive.face,
        Tok::Link => th.info.face,
        Tok::Quote => th.ink_soft.lerp(th.ink_light, 0.35),
        Tok::Marker => th.ink_soft,
    }
}

/// Draw one wrapped line of runs, positioning by character offset.
fn draw_line(ui: &mut Ui, x: i32, y: i32, spans: &[Span], ctx: &Ctx) {
    let mut col = 0i32;
    for span in spans {
        let sx = x + col * ADVANCE;
        let len = span.text.chars().count() as i32;
        let color = token_color(&ctx.th, span.tok);

        match span.tok {
            // A code span gets a slab behind it, which is what makes it read as
            // code rather than as differently coloured prose.
            Tok::Code => {
                ui.canvas.fill_rect(
                    Rect::new(sx - 1, y - 1, len * ADVANCE, LINE_H),
                    ctx.th.well.shade(0.10),
                );
            }
            Tok::Link => {
                ui.canvas
                    .hline(sx, y + font::GLYPH_H, len * ADVANCE - 2, color);
            }
            _ => {}
        }
        font::draw_text_styled(ui.canvas, sx, y, &span.text, color, span.bold);
        col += len;
    }
}

// ------------------------------------------------------------------ measuring

fn measure(block: &Block, ctx: &Ctx) -> i32 {
    match block {
        Block::Heading { level, spans } => {
            let lines = wrap(spans, ctx.cols(0)).len().max(1) as i32;
            let rule = if *level <= 2 { 3 } else { 0 };
            HEADING_TOP + lines * LINE_H + rule + 2
        }
        Block::Paragraph(spans) => wrap(spans, ctx.cols(0)).len().max(1) as i32 * LINE_H + PARA_GAP,
        Block::List(items) => {
            items
                .iter()
                .map(|it| {
                    let indent = item_indent(it);
                    wrap(&it.spans, ctx.cols(indent)).len().max(1) as i32 * LINE_H
                })
                .sum::<i32>()
                + PARA_GAP
        }
        Block::Quote(lines) => {
            let indent = 10;
            lines
                .iter()
                .map(|l| wrap(l, ctx.cols(indent)).len().max(1) as i32 * LINE_H)
                .sum::<i32>()
                + 4
                + PARA_GAP
        }
        Block::Code { lines, .. } => lines.len().max(1) as i32 * LINE_H + 8 + PARA_GAP,
        Block::Table { header, rows, .. } => {
            (rows.len() as i32 + 1) * (LINE_H + 2)
                + 4
                + PARA_GAP
                + if header.is_empty() { 0 } else { 2 }
        }
        Block::Rule => 9,
    }
}

/// How far a list item's text is pushed in by its nesting and marker.
fn item_indent(item: &Item) -> i32 {
    item.depth as i32 * (ADVANCE * 2) + ADVANCE * 2
}

// ------------------------------------------------------------------- drawing

/// Draw a whole document into the current layout.
///
/// Each block allocates its own height, so the enclosing scroll area measures
/// the document for free and clipping falls out of the layout rather than being
/// arranged separately.
pub fn draw_document(ui: &mut Ui, blocks: &[Block], width: i32) {
    let ctx = Ctx {
        th: *ui.theme,
        width,
    };
    if blocks.is_empty() {
        ui.label_dim("  (EMPTY NOTE)");
        return;
    }
    for block in blocks {
        let h = measure(block, &ctx);
        let rect = ui.alloc(h);
        draw_block(ui, rect, block, &ctx);
    }
}

fn draw_block(ui: &mut Ui, rect: Rect, block: &Block, ctx: &Ctx) {
    let th = ctx.th;
    match block {
        Block::Heading { level, spans } => {
            let y = rect.y + HEADING_TOP;
            let color = match level {
                1 => th.accent.hi,
                2 => th.accent.face,
                _ => th.ink_light,
            };
            for (i, line) in wrap(spans, ctx.cols(0)).iter().enumerate() {
                let ly = y + i as i32 * LINE_H;
                let mut col = 0i32;
                for span in line {
                    let sx = rect.x + col * ADVANCE;
                    // A heading is one weight and one colour, whatever emphasis
                    // the source put inside it.
                    font::draw_text_styled(ui.canvas, sx, ly, &span.text, color, true);
                    col += span.text.chars().count() as i32;
                }
            }
            if *level <= 2 {
                let ry = rect.bottom() - 3;
                let tint = if *level == 1 {
                    th.accent.face
                } else {
                    th.ink_soft
                };
                ui.canvas.hline(rect.x, ry, rect.w, tint);
            }
        }

        Block::Paragraph(spans) => {
            for (i, line) in wrap(spans, ctx.cols(0)).iter().enumerate() {
                draw_line(ui, rect.x, rect.y + i as i32 * LINE_H, line, ctx);
            }
        }

        Block::List(items) => {
            let mut y = rect.y;
            for item in items {
                let indent = item_indent(item);
                let mx = rect.x + item.depth as i32 * (ADVANCE * 2);
                match item.marker {
                    Marker::Bullet => {
                        // A filled square at nesting zero, hollow deeper down,
                        // so levels are told apart without indentation alone.
                        let dot = Rect::new(mx + 2, y + 2, 3, 3);
                        if item.depth == 0 {
                            ui.canvas.fill_rect(dot, th.accent.face);
                        } else {
                            ui.canvas.stroke_rect(dot.inset(-1), th.ink_soft);
                        }
                    }
                    Marker::Number(n) => {
                        let label = format!("{n}.");
                        font::draw_text(ui.canvas, mx, y, &label, th.accent.face);
                    }
                    Marker::Task(done) => {
                        let box_rect = Rect::new(mx, y - 1, 8, 8);
                        ui.canvas.box_chamfer(box_rect, th.well, th.well_border, 1);
                        if done {
                            icon::draw_centered(ui.canvas, box_rect, icon::CHECK, th.positive.face);
                        }
                    }
                }
                for (i, line) in wrap(&item.spans, ctx.cols(indent)).iter().enumerate() {
                    draw_line(ui, rect.x + indent, y + i as i32 * LINE_H, line, ctx);
                }
                y += wrap(&item.spans, ctx.cols(indent)).len().max(1) as i32 * LINE_H;
            }
        }

        Block::Quote(lines) => {
            let indent = 10;
            let bar = Rect::new(rect.x, rect.y, 2, rect.h - PARA_GAP);
            ui.canvas.fill_rect(bar, th.accent.lo);
            let mut y = rect.y + 2;
            for spans in lines {
                for (i, line) in wrap(spans, ctx.cols(indent)).iter().enumerate() {
                    let ly = y + i as i32 * LINE_H;
                    let mut col = 0i32;
                    for span in line {
                        let sx = rect.x + indent + col * ADVANCE;
                        font::draw_text(
                            ui.canvas,
                            sx,
                            ly,
                            &span.text,
                            token_color(&th, Tok::Quote),
                        );
                        col += span.text.chars().count() as i32;
                    }
                }
                y += wrap(spans, ctx.cols(indent)).len().max(1) as i32 * LINE_H;
            }
        }

        Block::Code { lines, .. } => {
            let slab = Rect::new(rect.x, rect.y, rect.w, rect.h - PARA_GAP);
            ui.canvas.box_chamfer(slab, th.well, th.well_border, 1);
            ui.clipped(slab.inset(3), |ui| {
                for (i, line) in lines.iter().enumerate() {
                    // Code is not wrapped: a break inserted into code is a lie
                    // about what the code says.
                    font::draw_text(
                        ui.canvas,
                        slab.x + 4,
                        slab.y + 4 + i as i32 * LINE_H,
                        line,
                        th.positive.face,
                    );
                }
            });
        }

        Block::Table {
            align,
            header,
            rows,
        } => draw_table(ui, rect, align, header, rows, ctx),

        Block::Rule => {
            let y = rect.y + rect.h / 2;
            ui.canvas.hline(rect.x, y, rect.w, th.ink_soft);
            ui.canvas.hline(rect.x, y + 1, rect.w, th.panel.shade(0.4));
        }
    }
}

/// Column widths, shared by measuring and drawing so they cannot disagree.
pub fn column_widths(header: &[Vec<Span>], rows: &[Vec<Vec<Span>>], total: i32) -> Vec<i32> {
    let count = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if count == 0 {
        return Vec::new();
    }
    let mut chars = vec![0usize; count];
    let mut note = |cells: &[Vec<Span>]| {
        for (i, cell) in cells.iter().enumerate().take(count) {
            let len: usize = cell.iter().map(|s| s.text.chars().count()).sum();
            chars[i] = chars[i].max(len);
        }
    };
    note(header);
    for row in rows {
        note(row);
    }

    // Size to the content first. A three-word table stretched across the pane
    // reads as a layout accident, not as a table.
    let natural: Vec<i32> = chars.iter().map(|c| *c as i32 * ADVANCE + 10).collect();
    let wanted: i32 = natural.iter().sum();
    if wanted <= total {
        return natural;
    }
    // Only when it genuinely does not fit is the width shared out in
    // proportion. The floor is itself bounded by the share each column could
    // possibly have, or two narrow columns in a narrow table would each claim a
    // minimum and together overflow the very width they are being fitted into.
    let floor = (total / count as i32).clamp(1, ADVANCE * 2);
    natural
        .iter()
        .map(|w| (((*w as i64 * total as i64) / wanted as i64) as i32).max(floor))
        .collect()
}

fn draw_table(
    ui: &mut Ui,
    rect: Rect,
    align: &[CellAlign],
    header: &[Vec<Span>],
    rows: &[Vec<Vec<Span>>],
    ctx: &Ctx,
) {
    let th = ctx.th;
    let widths = column_widths(header, rows, rect.w);
    if widths.is_empty() {
        return;
    }
    let row_h = LINE_H + 2;
    let body = Rect::new(
        rect.x,
        rect.y,
        widths.iter().sum::<i32>(),
        rect.h - PARA_GAP,
    );
    ui.canvas.box_chamfer(body, th.well, th.well_border, 1);

    let cell = |ui: &mut Ui, x: i32, y: i32, w: i32, spans: &[Span], a: CellAlign, bold: bool| {
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        let tw = font::advance_width(&text);
        let ox = match a {
            CellAlign::Left => 4,
            CellAlign::Center => ((w - tw) / 2).max(4),
            CellAlign::Right => (w - tw - 4).max(4),
        };
        let color = if bold { th.accent.hi } else { th.ink_light };
        font::draw_text_styled(ui.canvas, x + ox, y, &text, color, bold);
    };

    // ---- header ------------------------------------------------------
    let mut y = body.y + 3;
    let mut x = body.x;
    for (i, cells) in header.iter().enumerate() {
        let w = widths[i];
        cell(
            ui,
            x,
            y,
            w,
            cells,
            align.get(i).copied().unwrap_or_default(),
            true,
        );
        x += w;
    }
    y += row_h;
    ui.canvas
        .hline(body.x + 1, y - 2, body.w - 2, th.well_border);

    // ---- body --------------------------------------------------------
    for row in rows {
        let mut x = body.x;
        for (i, cells) in row.iter().enumerate().take(widths.len()) {
            let w = widths[i];
            cell(
                ui,
                x,
                y,
                w,
                cells,
                align.get(i).copied().unwrap_or_default(),
                false,
            );
            x += w;
        }
        y += row_h;
    }

    // ---- column rules ------------------------------------------------
    let mut x = body.x;
    for w in widths.iter().take(widths.len() - 1) {
        x += w;
        ui.canvas.vline(x, body.y + 1, body.h - 2, th.well_border);
    }
}
