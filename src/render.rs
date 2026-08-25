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

use std::cell::RefCell;

use pixui::{font, icon, palette, Color, Cursor, Rect, Theme, Ui};

use crate::markdown::{slice_spans, wrap_ranges};
use crate::markdown::{Block, CellAlign, Item, Located, Marker, Span, Tok};

const LINE_H: i32 = font::LINE_H;
const ADVANCE: i32 = font::ADVANCE;

/// Space under each kind of block, so the document breathes without a blank
/// line having to be typed for it.
const PARA_GAP: i32 = 5;
/// How far a quote's contents sit right of its bar.
const QUOTE_INDENT: i32 = 10;
/// Height a heading's rule takes, whatever it spans.
const RULE_H: i32 = 3;

/// What a heading draws beneath itself.
#[derive(Clone, Copy, PartialEq)]
enum Rule {
    None,
    /// As wide as the words above it, which is a smaller gesture than a rule
    /// across the pane and reads as a smaller heading.
    Text,
    Full,
}

/// How one heading level is drawn.
struct Heading {
    color: Color,
    bold: bool,
    rule: Rule,
    tint: Color,
    /// Air above it. A level that opens a section deserves more of it than one
    /// that labels a paragraph, and the spacing alone tells them apart before
    /// any of the ink does.
    top: i32,
}

/// The ladder of heading levels.
///
/// There is one font, at one size, so the ladder has to be made of everything
/// else: colour, weight, the rule beneath, and the air above. Six levels need
/// six answers that differ at a glance, and dimming alone does not survive
/// being read at five pixels by seven — so the top three each underline
/// themselves differently, and the bottom one gives up its weight.
fn heading_style(th: &Theme, level: u8) -> Heading {
    let (color, bold, rule, tint, top) = match level {
        1 => (th.accent.hi, true, Rule::Full, th.accent.face, 7),
        2 => (th.accent.face, true, Rule::Full, th.ink_soft, 6),
        3 => (th.info.hi, true, Rule::Text, th.info.face, 6),
        4 => (th.ink_light, true, Rule::None, th.ink_soft, 5),
        5 => (
            th.ink_light.lerp(th.ink_soft, 0.55),
            true,
            Rule::None,
            th.ink_soft,
            4,
        ),
        _ => (th.ink_soft, false, Rule::None, th.ink_soft, 4),
    };
    Heading {
        color,
        bold,
        rule,
        tint,
        top,
    }
}

/// Everything the layout needs that does not change between blocks.
struct Ctx {
    th: Theme,
    /// Left edge of the column the source lines are numbered down. Blocks draw
    /// their own numbers into it, because only a block knows which of its rows
    /// are rows somebody typed: a list item is one, a wrapped line is not.
    gutter: std::cell::Cell<i32>,
    /// Width available for content, in pixels. A cell because a quote lends
    /// its contents a narrower one and hands it back, and everything below is
    /// reached through a shared reference.
    width: std::cell::Cell<i32>,
    /// Whether the blocks being drawn are inside a quote, so their prose keeps
    /// the quieter ink a quote has always had. Only prose: a heading inside a
    /// quote is still a heading, and code inside one is still code.
    quoted: std::cell::Cell<bool>,
    /// The pattern the search is highlighting, if there is one. The same one
    /// the source view lights up: a search is about the note, not about which
    /// view of it happens to be showing.
    search: Option<String>,
    /// The source line the caller wants brought into view, and where the block
    /// holding it turned out to be. Answered during the draw, because that is
    /// when the document's heights are known.
    reveal: Option<usize>,
    reveal_y: std::cell::Cell<Option<i32>>,
    /// A link activated this frame, on its way back out to the application.
    ///
    /// A cell rather than a return value because a link is found six calls
    /// deep, inside a loop over the runs of a wrapped line of one block, and
    /// threading it back up would put a `Option<String>` in every signature
    /// between here and there.
    clicked: RefCell<Option<String>>,
}

impl Ctx {
    /// How many characters fit in the current width, less an indent.
    fn cols(&self, indent: i32) -> usize {
        (((self.width.get() - indent) / ADVANCE).max(4)) as usize
    }

    /// Run `f` with the width reduced by `by`, then put it back.
    fn narrowed<R>(&self, by: i32, f: impl FnOnce() -> R) -> R {
        let was = self.width.get();
        self.width.set(was - by);
        let r = f();
        self.width.set(was);
        r
    }
}

/// Number one row in the gutter.
fn number(ui: &mut Ui, ctx: &Ctx, y: i32, line: usize) {
    let num = format!("{:>3}", line + 1);
    font::draw_text(
        ui.canvas,
        ctx.gutter.get() + 1,
        y,
        &num,
        ctx.th.ink_soft.shade(-0.2),
    );
}

/// Light up the search hits in one row of text.
///
/// Drawn before the glyphs, and measured in characters, because every run in
/// this renderer is placed on the same character grid.
fn highlight_hits(ui: &mut Ui, ctx: &Ctx, x: i32, y: i32, text: &str) {
    let Some(pattern) = ctx.search.as_deref() else {
        return;
    };
    for (a, b) in crate::vim::matches_in(text, pattern) {
        let cell = Rect::new(
            x + a as i32 * ADVANCE - 1,
            y - 1,
            (b - a) as i32 * ADVANCE,
            LINE_H,
        );
        ui.canvas
            .fill_rect(cell, ctx.th.well.lerp(palette::YELLOW, 0.30));
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
        Tok::Strike => th.ink_soft,
        Tok::Image => th.info.hi,
        Tok::Marker => th.ink_soft,
        Tok::CodePlain => th.ink_light,
        Tok::CodeKeyword => palette::ACCENT,
        Tok::CodeType => palette::TEAL,
        Tok::CodeFunction => palette::TEAL_HI,
        Tok::CodeString => palette::GREEN,
        Tok::CodeNumber => palette::YELLOW,
        Tok::CodeComment => th.ink_soft,
        Tok::CodePunct => th.ink_light.shade(-0.30),
    }
}

/// Draw one wrapped line of runs, positioning by character offset.
fn draw_line(ui: &mut Ui, x: i32, y: i32, spans: &[Span], ctx: &Ctx) {
    if ctx.search.is_some() {
        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
        highlight_hits(ui, ctx, x, y, &text);
    }
    let mut col = 0i32;
    for span in spans {
        let sx = x + col * ADVANCE;
        let len = span.text.chars().count() as i32;
        let plain = span.tok == Tok::Text && ctx.quoted.get();
        let mut color = token_color(&ctx.th, if plain { Tok::Quote } else { span.tok });
        let cell = Rect::new(sx - 1, y - 1, len * ADVANCE, LINE_H);

        match span.tok {
            // A code span gets a slab behind it, which is what makes it read as
            // code rather than as differently coloured prose.
            Tok::Code => {
                ui.canvas.fill_rect(cell, ctx.th.well.shade(0.10));
            }
            // A line through the middle, which is the whole point of the
            // notation and the only way it survives being rendered.
            Tok::Strike => {
                ui.canvas
                    .hline(sx, y + font::GLYPH_H / 2, len * ADVANCE - 2, color);
            }
            // There is no way to draw the image, so say so plainly and let the
            // alt text stand where the picture would.
            Tok::Image => {
                ui.canvas.box_chamfer(
                    Rect::new(sx - 2, y - 2, len * ADVANCE + 2, LINE_H + 2),
                    ctx.th.well.shade(0.08),
                    ctx.th.info.face,
                    1,
                );
            }
            Tok::Link => {
                // Only a link once it knows where it points: a bare `[x]` with
                // no target is prose in brackets, and gets no pointer.
                if let Some(href) = span.href.as_deref() {
                    // Keyed on the target and the row, so two links to
                    // different places never share a hover, and one link keeps
                    // its own across a redraw.
                    let id = ui.id(&format!("link:{href}:{y}"));
                    let resp = ui.interact(id, cell);
                    if resp.hovered {
                        ui.request_cursor(Cursor::Pointer);
                        color = ctx.th.info.hi;
                    }
                    if resp.clicked {
                        *ctx.clicked.borrow_mut() = Some(href.to_string());
                    }
                }
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
            let h = heading_style(&ctx.th, *level);
            let lines = wrap(spans, ctx.cols(0)).len().max(1) as i32;
            let rule = if h.rule == Rule::None { 0 } else { RULE_H };
            h.top + lines * LINE_H + rule + 2
        }
        Block::Paragraph(spans) => wrap(spans, ctx.cols(0)).len().max(1) as i32 * LINE_H + PARA_GAP,
        Block::List(items) => {
            items
                .iter()
                .map(|it| {
                    let indent = list_indent(items, it);
                    wrap(&it.spans, ctx.cols(indent)).len().max(1) as i32 * LINE_H
                })
                .sum::<i32>()
                + PARA_GAP
        }
        Block::Quote(inner) => {
            let indent = QUOTE_INDENT;
            ctx.narrowed(indent, || {
                inner.iter().map(|(_, b)| measure(b, ctx)).sum::<i32>()
            }) + 4
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

/// How far a list's text is pushed in, past its nesting and its markers.
///
/// One indent for the whole list, measured from its widest marker, so `1.` and
/// `10.` do not leave their text ragged against each other.
fn list_indent(items: &[Item], item: &Item) -> i32 {
    let widest = items
        .iter()
        .map(|it| match it.marker {
            Marker::Number(n) => (n.to_string().chars().count() as i32 + 1) * ADVANCE,
            // A checkbox is wider than a bullet and needs the room to say so.
            Marker::Task(_) => 11,
            Marker::Bullet => ADVANCE + 2,
        })
        .max()
        .unwrap_or(ADVANCE);
    item.depth as i32 * (ADVANCE * 2) + widest + 4
}

// ------------------------------------------------------------------- drawing

/// Draw a whole document into the current layout.
///
/// Each block allocates its own height, so the enclosing scroll area measures
/// the document for free and clipping falls out of the layout rather than being
/// arranged separately.
/// Draw the document, and report a link the pointer activated in it.
///
/// Each block arrives with the source line it was parsed from, and is numbered
/// with it in a gutter the same width as the source view's. A rendering has no
/// lines of its own to count — a paragraph is one block however many rows it
/// wraps into — so the number says where the block *came from*, which is the
/// only number about a rendered document that means anything.
pub fn draw_document(ui: &mut Ui, blocks: &[Located], req: Request) -> Drawn {
    let ctx = Ctx {
        th: *ui.theme,
        gutter: std::cell::Cell::new(0),
        width: std::cell::Cell::new(req.width - crate::GUTTER),
        quoted: std::cell::Cell::new(false),
        search: req.search,
        reveal: req.reveal,
        reveal_y: std::cell::Cell::new(None),
        clicked: RefCell::new(None),
    };
    if blocks.is_empty() {
        ui.label_dim("  (EMPTY NOTE)");
        return Drawn::default();
    }
    // Distance down the document, taken from the rows themselves rather than
    // from the heights: the layout puts its own gap between them, and adding
    // that up a second time here is how the two would drift apart.
    let mut top = None;
    for (line, block) in blocks {
        let h = measure(block, &ctx);
        let row = ui.alloc(h);
        let top = *top.get_or_insert(row.y);
        ctx.gutter.set(row.x);
        let body = Rect::new(row.x + crate::GUTTER, row.y, row.w - crate::GUTTER, h);
        // The block holding a line is the last one that starts at or above it.
        if ctx.reveal.is_some_and(|want| *line <= want) {
            ctx.reveal_y.set(Some(row.y - top));
        }
        draw_block(ui, body, block, &ctx, *line);
    }
    Drawn {
        clicked: ctx.clicked.into_inner(),
        reveal: ctx.reveal_y.get(),
    }
}

/// What the caller wants drawn, and what it wants to know.
pub struct Request {
    /// Width available to the document, gutter included.
    pub width: i32,
    /// A pattern to light up wherever it appears in the rendered text.
    pub search: Option<String>,
    /// A source line to report the position of, for scrolling to it.
    pub reveal: Option<usize>,
}

/// What came back out of a draw.
#[derive(Default)]
pub struct Drawn {
    /// A link the pointer activated.
    pub clicked: Option<String>,
    /// How far down the document the requested line's block begins.
    pub reveal: Option<i32>,
}

fn draw_block(ui: &mut Ui, rect: Rect, block: &Block, ctx: &Ctx, line: usize) {
    let th = ctx.th;
    match block {
        Block::Heading { level, spans } => {
            let style = heading_style(&th, *level);
            let y = rect.y + style.top;
            number(ui, ctx, y, line);
            let mut widest = 0i32;
            for (i, row) in wrap(spans, ctx.cols(0)).iter().enumerate() {
                let ly = y + i as i32 * LINE_H;
                if ctx.search.is_some() {
                    let text: String = row.iter().map(|s| s.text.as_str()).collect();
                    highlight_hits(ui, ctx, rect.x, ly, &text);
                }
                let mut col = 0i32;
                for span in row {
                    let sx = rect.x + col * ADVANCE;
                    // A heading is one weight and one colour, whatever emphasis
                    // the source put inside it.
                    font::draw_text_styled(ui.canvas, sx, ly, &span.text, style.color, style.bold);
                    col += span.text.chars().count() as i32;
                }
                widest = widest.max(col * ADVANCE);
            }
            let rule_w = match style.rule {
                Rule::None => 0,
                // Less the tracking that trails the last glyph, so the rule
                // ends under the final letter rather than past it.
                Rule::Text => (widest - 2).clamp(1, rect.w),
                Rule::Full => rect.w,
            };
            if rule_w > 0 {
                ui.canvas
                    .hline(rect.x, rect.bottom() - RULE_H, rule_w, style.tint);
            }
        }

        Block::Paragraph(spans) => {
            // One number, at the top. A paragraph is not its source lines —
            // they reflow into however many rows the pane is wide enough for,
            // and no row below the first is a line anybody typed.
            number(ui, ctx, rect.y, line);
            for (i, row) in wrap(spans, ctx.cols(0)).iter().enumerate() {
                draw_line(ui, rect.x, rect.y + i as i32 * LINE_H, row, ctx);
            }
        }

        Block::List(items) => {
            let mut y = rect.y;
            for item in items {
                number(ui, ctx, y, item.line);
                let indent = list_indent(items, item);
                let mx = rect.x + item.depth as i32 * (ADVANCE * 2);
                match item.marker {
                    Marker::Bullet => {
                        if item.depth == 0 {
                            ui.canvas
                                .fill_rect(Rect::new(mx + 2, y + 2, 3, 3), th.accent.face);
                        } else {
                            // A dash, not a hollow square: an outlined box at
                            // this size is indistinguishable from an unchecked
                            // task, which is a different thing entirely.
                            ui.canvas
                                .fill_rect(Rect::new(mx + 1, y + 3, 4, 1), th.ink_soft);
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
                for (i, row) in wrap(&item.spans, ctx.cols(indent)).iter().enumerate() {
                    draw_line(ui, rect.x + indent, y + i as i32 * LINE_H, row, ctx);
                }
                y += wrap(&item.spans, ctx.cols(indent)).len().max(1) as i32 * LINE_H;
            }
        }

        Block::Quote(inner) => {
            // The contents are ordinary blocks, drawn ordinarily, in a column
            // pushed right of the bar. A quote is a place, not a style — a
            // heading inside one is still a heading.
            let bar = Rect::new(rect.x, rect.y, 2, rect.h - PARA_GAP);
            ui.canvas.fill_rect(bar, th.accent.lo);
            let indent = QUOTE_INDENT;
            let was = ctx.quoted.replace(true);
            ctx.narrowed(indent, || {
                let mut y = rect.y + 2;
                for (at_line, block) in inner {
                    let h = measure(block, ctx);
                    let at = Rect::new(rect.x + indent, y, rect.w - indent, h);
                    draw_block(ui, at, block, ctx, *at_line);
                    y += h;
                }
            });
            ctx.quoted.set(was);
        }

        Block::Code { lang, lines, first } => {
            let slab = Rect::new(rect.x, rect.y, rect.w, rect.h - PARA_GAP);
            ui.canvas.box_chamfer(slab, th.well, th.well_border, 1);
            let highlighted = crate::syntax::highlight(lang, lines);
            // Every row of a code block is a line of the source, in order and
            // never re-wrapped, so every one of them can be numbered.
            for i in 0..highlighted.len() {
                number(ui, ctx, slab.y + 4 + i as i32 * LINE_H, first + i);
            }
            ui.clipped(slab.inset(3), |ui| {
                for (i, spans) in highlighted.iter().enumerate() {
                    // Code is not wrapped: a break inserted into code is a lie
                    // about what the code says. Runs are placed by character
                    // offset for the same reason prose is.
                    let y = slab.y + 4 + i as i32 * LINE_H;
                    if ctx.search.is_some() {
                        let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                        highlight_hits(ui, ctx, slab.x + 4, y, &text);
                    }
                    let mut col = 0i32;
                    for span in spans {
                        let sx = slab.x + 4 + col * ADVANCE;
                        font::draw_text(ui.canvas, sx, y, &span.text, token_color(&th, span.tok));
                        col += span.text.chars().count() as i32;
                    }
                }
            });
        }

        Block::Table {
            align,
            header,
            rows,
        } => draw_table(ui, rect, align, header, rows, ctx, line),

        Block::Rule => {
            let y = rect.y + rect.h / 2;
            number(ui, ctx, y - 3, line);
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
    line: usize,
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
        highlight_hits(ui, ctx, x + ox, y, &text);
        font::draw_text_styled(ui.canvas, x + ox, y, &text, color, bold);
    };

    // ---- header ------------------------------------------------------
    let mut y = body.y + 3;
    number(ui, ctx, y, line);
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
    // The header is the block's line, the alignment row the one after it, and
    // the body follows from there — a table's rows are always consecutive.
    for (r, row) in rows.iter().enumerate() {
        number(ui, ctx, y, line + 2 + r);
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
