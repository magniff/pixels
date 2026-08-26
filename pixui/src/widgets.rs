//! The widget set.
//!
//! Every control here is built from the same three moves: a shadow silhouette,
//! a chamfered face that physically sinks into that shadow when pressed, and a
//! one-pixel light/dark edge that inverts on the way down. Getting those three
//! right is most of what makes a pixel UI feel tactile rather than flat.

use crate::anim::{smooth, WidgetAnim};
use crate::clipboard;
use crate::color::Color;
use crate::font;
use crate::geom::{Point, Rect};
use crate::icon;
use crate::input::{Cursor, Key};
use crate::layout::Align;
use crate::theme::{Ramp, Tone};
use crate::ui::{Response, ScrollState, Ui};

const WHITE: Color = Color::hex(0xFFFFFF);

/// One option in a segmented control: a label, and optionally an icon.
#[derive(Clone, Copy)]
pub struct Segment<'a> {
    pub icon: Option<&'a [&'a str]>,
    pub label: &'a str,
}

impl<'a> Segment<'a> {
    pub fn new(label: &'a str) -> Self {
        Self { icon: None, label }
    }

    pub fn with_icon(icon: &'a [&'a str], label: &'a str) -> Self {
        Self {
            icon: Some(icon),
            label,
        }
    }

    /// The width of the icon and label together, including the gap.
    fn width(&self) -> i32 {
        let text = font::text_width(self.label);
        match self.icon {
            Some(rows) => icon::size(rows).0 + SEGMENT_GAP + text,
            None => text,
        }
    }
}

/// Space between a segment's icon and its label.
const SEGMENT_GAP: i32 = 4;

/// Draw a segment's icon and label as one centred group.
fn draw_segment(ui: &mut Ui, cell: Rect, seg: &Segment, ink: Color) {
    let total = seg.width();
    let mut x = cell.x + (cell.w - total) / 2;
    let y = cell.y + (cell.h - font::glyph_h()) / 2;
    if let Some(rows) = seg.icon {
        let (w, h) = icon::size(rows);
        icon::draw(ui.canvas, x, cell.y + (cell.h - h) / 2, rows, ink);
        x += w + SEGMENT_GAP;
    }
    font::draw_text(ui.canvas, x, y, seg.label, ink);
}

/// How a text field is laid out, and whether it should claim focus.
///
/// Grouped rather than passed as a run of positional arguments, where the two
/// paddings are the same type and swapping them would silently draw the text in
/// the wrong place.
#[derive(Clone, Copy, Default)]
struct FieldOpts {
    pad_left: i32,
    pad_right: i32,
    grab: bool,
}

/// A 7x7 tick, bit 6 leftmost.
#[rustfmt::skip]
const CHECK: [u8; 7] = [
    0b0000000,
    0b0000011,
    0b0000110,
    0b1001100,
    0b1101100,
    0b0111000,
    0b0010000,
];

impl Ui<'_> {
    // ------------------------------------------------------------------ chrome

    /// Draw the raised face of a control and return the rect it actually
    /// occupies, which is `rect` shifted down by the current press depth.
    ///
    /// The shadow is drawn at full depth and then covered by the face, so a
    /// fully pressed control lands flush against its own shadow. That is the
    /// entire illusion — nothing is faded, the button really does move.
    fn draw_control_face(&mut self, rect: Rect, ramp: Ramp, anim: &WidgetAnim) -> Rect {
        let th = *self.theme;
        let m = th.metrics;

        // The spring overshoots at both ends; let it lift a pixel off the page
        // on release, which is what sells the bounce.
        let sink = (anim.press.pos * m.press_depth as f32).round() as i32;
        let sink = sink.clamp(-1, m.press_depth);
        let body = rect.translate(0, sink);

        self.canvas
            .fill_chamfer(rect.translate(0, m.press_depth), th.shadow, m.chamfer);

        let press = anim.press.pos.clamp(0.0, 1.0);
        let face = ramp
            .face
            .lerp(ramp.hi, anim.hover * 0.28)
            .lerp(WHITE, anim.flash * 0.40)
            .lerp(ramp.lo, press * 0.15);

        self.canvas
            .box_chamfer(body, face, th.panel_border, m.chamfer);

        let edge_w = body.w - m.chamfer * 2;
        if edge_w > 0 {
            let top = ramp.hi.lerp(ramp.lo, press);
            let bottom = ramp.lo.lerp(face, press);
            self.canvas
                .hline(body.x + m.chamfer, body.y + 1, edge_w, top);
            self.canvas
                .hline(body.x + m.chamfer, body.bottom() - 2, edge_w, bottom);
        }

        body
    }

    /// Marching-ants focus ring, drawn outside `rect`.
    fn draw_focus_ring(&mut self, rect: Rect, anim: &WidgetAnim) {
        if anim.focus <= 0.02 {
            return;
        }
        let th = *self.theme;
        let color = th.background.lerp(th.focus_ring, anim.focus);
        // Still dashes, and no travel either — unlike [`Ui::focus_flare`], which
        // is over in half a second. This ring stays for as long as the widget
        // holds the keyboard, and dashes that march forever are a full redraw
        // of the window sixty times a second for as long as anything is
        // focused, which is most of the time. The fade in is the animation.
        self.canvas
            .stroke_rect_dashed(rect.inset(-2), color, 2, 2, 0);
    }

    /// Say that the keyboard has just arrived somewhere.
    ///
    /// Draws nothing at rest, so a region that holds the focus most of the
    /// time does not end up wearing a permanent border. Pass `arrived` on the
    /// one frame it took the keyboard; the ring fades up and back down on its
    /// own, marching like the focus ring it is a bigger version of.
    ///
    /// The geometry never moves. An earlier version contracted onto its
    /// target, which meant sweeping the line across whatever chrome sat
    /// between the two positions — around a short field that is the panel edge
    /// and the field's own border, and it read as flicker rather than as
    /// arrival. Only the colour changes here.
    ///
    /// For a widget that already shows its own focus — a text field lights its
    /// border — this is the wrong tool: two rings one outside the other read
    /// as a doubled outline rather than as arrival. It is for a *region* the
    /// keyboard can be aimed at, which has no focus of its own to show.
    ///
    /// `over` is what the ring sits on and fades back into. A parameter rather
    /// than a guess from the theme, because the caller is the only one that
    /// knows what its own region is surrounded by, and fading to the wrong
    /// colour leaves an outline behind after the animation is over.
    pub fn focus_flare(&mut self, id_label: &str, rect: Rect, over: Color, arrived: bool) {
        let dt = self.input.dt;
        let id = self.id(id_label);
        let flare = self.with_anim(id, |a| {
            if arrived {
                a.flash = 1.0;
            }
            a.flash = smooth(a.flash, 0.0, 5.5, dt);
            a.flash
        });
        if flare <= 0.05 {
            return;
        }
        let color = over.lerp(self.theme.focus_ring, flare);
        let phase = (self.input.time * 14.0) as i32;
        self.canvas
            .stroke_rect_dashed(rect.inset(-2), color, 2, 2, phase);
        // The dashes march on the clock rather than on any state, and the
        // flare is fading besides.
        self.request_repaint();
    }

    /// A recessed well: the inverse of [`Ui::draw_control_face`], used for
    /// tracks and anything the eye should read as a hole rather than a lump.
    fn draw_well(&mut self, rect: Rect, fill: Color) {
        let th = *self.theme;
        let m = th.metrics;
        self.canvas
            .box_chamfer(rect, fill, th.well_border, (m.chamfer - 1).max(0));
        let inner_w = rect.w - 2;
        if inner_w > 0 {
            self.canvas
                .hline(rect.x + 1, rect.y + 1, inner_w, fill.shade(-0.25));
        }
    }

    // ----------------------------------------------------------------- panels

    /// A titled panel. Returns the content rect inside it.
    ///
    /// Pass an empty title for a plain frame with no strip.
    pub fn panel(&mut self, rect: Rect, title: &str) -> Rect {
        let th = *self.theme;
        let m = th.metrics;

        self.canvas
            .fill_chamfer(rect.translate(0, m.press_depth), th.shadow, m.chamfer);
        self.canvas
            .box_chamfer(rect, th.panel, th.panel_border, m.chamfer);

        let mut inner = rect.inset(1);
        if !title.is_empty() {
            let (strip, rest) = inner.split_top(m.title_h);
            inner = rest;
            self.canvas
                .fill_chamfer(strip, th.panel_title, (m.chamfer - 1).max(0));
            // A lit top edge rather than a dithered fade. The fade put an
            // ordered-dither checkerboard directly behind 5x7 letterforms, and
            // the bright halo under each glyph doubled every stroke; together
            // they read as mush at this size.
            let lit = strip.w - m.chamfer * 2;
            if lit > 0 {
                self.canvas.hline(
                    strip.x + m.chamfer,
                    strip.y,
                    lit,
                    th.panel_title.shade(0.28),
                );
            }
            self.canvas
                .hline(strip.x, strip.bottom(), strip.w, th.panel_border);

            let text = strip.translate(m.text_pad, 0);
            let y = text.y + (text.h - font::glyph_h()) / 2;
            font::draw_text_styled(self.canvas, text.x, y, title, th.panel_title_ink, true);
        }
        inner.inset(m.pad)
    }

    /// A flat inset region, for grouping without the weight of a full panel.
    pub fn well(&mut self, rect: Rect) -> Rect {
        let th = *self.theme;
        self.draw_well(rect, th.well);
        rect.inset(2)
    }

    // ------------------------------------------------------------ text entry

    /// A single-line text field.
    ///
    /// Caret positions are counted in *characters*, not bytes. The built-in
    /// font is ASCII-only so today those are the same number, but indexing a
    /// `String` by bytes is the kind of shortcut that turns into a panic the
    /// first time someone types outside it.
    pub fn text_field(&mut self, name: &str, text: &mut String) -> Response {
        let h = self.theme.metrics.control_h;
        let rect = self.alloc(h);
        self.text_field_at(rect, name, text)
    }

    /// A text field at an explicit rect.
    pub fn text_field_at(&mut self, rect: Rect, name: &str, text: &mut String) -> Response {
        self.text_field_hint_at(rect, name, text, "")
    }

    /// A text field showing `hint` in place of empty text.
    pub fn text_field_hint_at(
        &mut self,
        rect: Rect,
        name: &str,
        text: &mut String,
        hint: &str,
    ) -> Response {
        let pad = self.theme.metrics.text_pad;
        let opts = FieldOpts {
            pad_left: pad,
            pad_right: pad,
            grab: false,
        };
        self.text_field_core(rect, name, text, hint, opts)
    }

    /// A text field that takes focus on the frame `grab` is true.
    ///
    /// For a field that appears in response to something else — a rename
    /// started by a double click — where the user expects to type immediately.
    /// It has to claim focus *before* its own hit testing, or it spends its
    /// first frame unfocused and the keystroke meant for it goes wherever the
    /// application sends keys instead.
    pub fn text_field_grab_at(
        &mut self,
        rect: Rect,
        name: &str,
        text: &mut String,
        hint: &str,
        grab: bool,
    ) -> Response {
        let pad = self.theme.metrics.text_pad;
        let opts = FieldOpts {
            pad_left: pad,
            pad_right: pad,
            grab,
        };
        self.text_field_core(rect, name, text, hint, opts)
    }

    /// The text field proper, with explicit room reserved at each end.
    ///
    /// The padding is a parameter because a field with furniture in it — a
    /// search glass, a clear button — needs its text to start and stop clear of
    /// them, while the well behind still spans the whole control.
    fn text_field_core(
        &mut self,
        rect: Rect,
        name: &str,
        text: &mut String,
        hint: &str,
        opts: FieldOpts,
    ) -> Response {
        let th = *self.theme;
        let id = self.id(name);

        if opts.grab {
            self.set_focus(id);
        }
        let mut resp = self.interact(id, rect);
        self.focusable(id);
        if resp.focused {
            // Let the application know typing belongs here, not to its own
            // key handling.
            self.set_text_focus(id);
        }
        if resp.hovered {
            self.request_cursor(Cursor::Text);
        }

        let mut st = self.text_state(id);
        let chars: Vec<char> = text.chars().collect();
        st.caret = st.caret.min(chars.len());
        if opts.grab {
            // A field that has just appeared with text already in it should put
            // the caret after that text, not before it. Otherwise the first
            // thing typed lands in front of what is being edited.
            st.caret = chars.len();
        }

        let inner = Rect::from_min_max(
            rect.x + opts.pad_left,
            rect.y,
            rect.right() - opts.pad_right,
            rect.bottom(),
        );

        // Clicking places the caret at the nearest character boundary.
        if resp.held {
            let local = self.input.mouse.x - inner.x + st.scroll;
            let col = ((local + font::advance() / 2) / font::advance()).max(0) as usize;
            st.caret = col.min(chars.len());
        }

        if resp.focused && !self.is_input_blocked() {
            let mut edited = chars.clone();
            let cmd = self.input.mods.cmd;
            for key in &self.input.keys {
                match key {
                    // The three the whole desktop shares. A field has no
                    // selection of its own, so copy and cut mean the field:
                    // there is nothing else they could mean, and reaching for
                    // Cmd-C in a one-line box is a request for what is in it.
                    Key::Char('c') if cmd => clipboard::copy(&edited.iter().collect::<String>()),
                    Key::Char('x') if cmd => {
                        clipboard::copy(&edited.iter().collect::<String>());
                        edited.clear();
                        st.caret = 0;
                    }
                    Key::Char('v') if cmd => {
                        if let Some(text) = clipboard::paste() {
                            // Flattened, because this is one line. Pasting a
                            // paragraph into a search box should give the
                            // paragraph run together, not its first line with
                            // the rest silently dropped.
                            for c in text.chars().map(|c| if c == '\n' { ' ' } else { c }) {
                                edited.insert(st.caret.min(edited.len()), c);
                                st.caret += 1;
                            }
                        }
                    }
                    Key::Char(c) if !self.input.mods.cmd && !self.input.mods.ctrl => {
                        edited.insert(st.caret.min(edited.len()), *c);
                        st.caret += 1;
                    }
                    Key::Space => {
                        edited.insert(st.caret.min(edited.len()), ' ');
                        st.caret += 1;
                    }
                    Key::Backspace if st.caret > 0 => {
                        edited.remove(st.caret - 1);
                        st.caret -= 1;
                    }
                    Key::Delete if st.caret < edited.len() => {
                        edited.remove(st.caret);
                    }
                    Key::Left => st.caret = st.caret.saturating_sub(1),
                    Key::Right => st.caret = (st.caret + 1).min(edited.len()),
                    Key::Home => st.caret = 0,
                    Key::End => st.caret = edited.len(),
                    _ => {}
                }
            }
            let next: String = edited.iter().collect();
            if next != *text {
                *text = next;
                resp.changed = true;
            }
        }

        // Keep the caret in view when the text is longer than the field.
        let caret_x = st.caret as i32 * font::advance();
        if caret_x - st.scroll > inner.w - font::advance() {
            st.scroll = caret_x - inner.w + font::advance();
        }
        if caret_x - st.scroll < 0 {
            st.scroll = caret_x;
        }
        st.scroll = st.scroll.max(0);

        // ---- draw --------------------------------------------------------
        let anim = self.animate(id, &resp);
        self.draw_well(rect, th.well);
        if anim.focus > 0.02 {
            let ring = th.well.lerp(th.focus_ring, anim.focus);
            let chamfer = (th.metrics.chamfer - 1).max(0);
            self.canvas.stroke_chamfer(rect, ring, chamfer);
        }

        self.clipped(inner, |ui| {
            let y = rect.y + (rect.h - font::glyph_h()) / 2;
            if text.is_empty() && !resp.focused && !hint.is_empty() {
                font::draw_text(ui.canvas, inner.x, y, hint, th.ink_light.shade(-0.35));
            }
            font::draw_text(ui.canvas, inner.x - st.scroll, y, text, th.ink_light);

            // A caret that blinks only while focused, and holds solid for a
            // moment after each keystroke so it never vanishes mid-type.
            if resp.focused {
                let since_key = if ui.input.keys.is_empty() {
                    ui.input.time
                } else {
                    0.0
                };
                if since_key % 1.0 < 0.6 || !ui.input.keys.is_empty() {
                    let cx = inner.x + caret_x - st.scroll;
                    ui.canvas
                        .fill_rect(Rect::new(cx, y - 1, 1, font::glyph_h() + 2), th.accent.face);
                }
            }
        });

        self.set_text_state(id, st);
        resp.rect = rect;
        resp
    }

    // ------------------------------------------------------------ title bar

    /// An application title strip: a small mark, a bold title, and an optional
    /// recessed badge on the right for a document name or a readout.
    ///
    /// Deliberately flat. The obvious treatment — a dithered gradient behind
    /// the text, with a bright one-pixel halo under each glyph — puts an
    /// ordered-dither checkerboard directly behind 5x7 letterforms and then
    /// doubles every stroke. At this size that reads as mush. A solid face with
    /// a lit top edge gives the same sense of a raised bar and leaves the text
    /// alone.
    pub fn title_bar(&mut self, rect: Rect, title: &str, badge: Option<&str>) {
        let th = *self.theme;
        self.canvas.fill_rect(rect, th.accent.face);
        self.canvas.hline(rect.x, rect.y, rect.w, th.accent.hi);
        self.canvas
            .hline(rect.x, rect.bottom() - 1, rect.w, th.panel_border);

        let y = rect.y + (rect.h - 1 - font::glyph_h()) / 2;

        // A small mark, so the strip reads as an application's rather than as
        // one more panel heading.
        let mark = Rect::new(rect.x + 5, y + 1, 5, 5);
        self.canvas.fill_rect(mark, th.ink);
        self.canvas.fill_rect(mark.inset(1), th.accent.hi);

        // Reserve the badge first, so a long title is clipped rather than
        // running underneath it.
        let mut title_right = rect.right() - 6;
        if let Some(text) = badge {
            let w = font::advance_width(text) + 10;
            let chip = Rect::new(rect.right() - w - 3, rect.y + 2, w, rect.h - 5);
            self.canvas.fill_chamfer(chip, th.accent.lo, 1);
            let inner = Rect::new(chip.x, rect.y, chip.w - 5, rect.h - 1);
            // Whichever ink the chip can actually be read in: a fixed one is a
            // guess about how dark this theme's accent happens to be.
            self.draw_text_in(inner, text, th.ink_on(th.accent.lo), Align::Right);
            title_right = chip.x - 4;
        }

        let title_x = mark.right() + 5;
        let area = Rect::from_min_max(title_x, rect.y, title_right.max(title_x), rect.bottom());
        self.clipped(area, |ui| {
            font::draw_text_styled(ui.canvas, title_x, y, title, th.ink, true);
        });
    }

    // -------------------------------------------------------------- splitters

    /// Split `bounds` into a left pane of width `size` and a right pane, with a
    /// divider the pointer can drag.
    ///
    /// `size` is updated in place and clamped to `range`, so the caller owns the
    /// value and decides whether to persist it. Returns the two content rects,
    /// with the divider's own strip excluded from both — a pane that had to
    /// dodge the handle itself would be a trap.
    pub fn split_left(
        &mut self,
        bounds: Rect,
        name: &str,
        size: &mut i32,
        range: (i32, i32),
    ) -> (Rect, Rect) {
        const HANDLE: i32 = 4;
        let (lo, hi) = (
            range.0.max(0),
            range.1.min(bounds.w - HANDLE).max(range.0.max(0)),
        );
        *size = (*size).clamp(lo, hi);

        let id = self.id(name);
        let handle = Rect::new(bounds.x + *size, bounds.y, HANDLE, bounds.h);
        // A four pixel target is a nuisance to hit, so the *grabbable* area is
        // wider than the drawn one. Nothing is drawn in the extra margin.
        let grab = handle.inset_xy(-3, 0);

        let resp = self.interact(id, grab);
        if resp.hovered || resp.held {
            self.request_cursor(Cursor::ResizeH);
        }
        if resp.held {
            *size = (self.input.mouse.x - bounds.x - HANDLE / 2).clamp(lo, hi);
        }

        let handle = Rect::new(bounds.x + *size, bounds.y, HANDLE, bounds.h);
        let anim = self.animate(id, &resp);
        self.draw_split_handle(handle, &anim, true);

        (
            Rect::new(bounds.x, bounds.y, *size, bounds.h),
            Rect::from_min_max(handle.right(), bounds.y, bounds.right(), bounds.bottom()),
        )
    }

    /// As [`Ui::split_left`], but a horizontal divider with the pane above it.
    pub fn split_top(
        &mut self,
        bounds: Rect,
        name: &str,
        size: &mut i32,
        range: (i32, i32),
    ) -> (Rect, Rect) {
        const HANDLE: i32 = 4;
        let (lo, hi) = (
            range.0.max(0),
            range.1.min(bounds.h - HANDLE).max(range.0.max(0)),
        );
        *size = (*size).clamp(lo, hi);

        let id = self.id(name);
        let handle = Rect::new(bounds.x, bounds.y + *size, bounds.w, HANDLE);
        let grab = handle.inset_xy(0, -3);

        let resp = self.interact(id, grab);
        if resp.hovered || resp.held {
            self.request_cursor(Cursor::ResizeV);
        }
        if resp.held {
            *size = (self.input.mouse.y - bounds.y - HANDLE / 2).clamp(lo, hi);
        }

        let handle = Rect::new(bounds.x, bounds.y + *size, bounds.w, HANDLE);
        let anim = self.animate(id, &resp);
        self.draw_split_handle(handle, &anim, false);

        (
            Rect::new(bounds.x, bounds.y, bounds.w, *size),
            Rect::from_min_max(bounds.x, handle.bottom(), bounds.right(), bounds.bottom()),
        )
    }

    /// The divider itself: a quiet line that lights up under the pointer, with
    /// a few grip pips so it reads as draggable before you try.
    fn draw_split_handle(&mut self, handle: Rect, anim: &WidgetAnim, vertical: bool) {
        let th = *self.theme;
        let lit = anim.hover.max(anim.press.pos.clamp(0.0, 1.0));
        let line = th.panel_border.lerp(th.accent.face, lit);

        if vertical {
            self.canvas
                .vline(handle.center_x(), handle.y, handle.h, line);
            let cy = handle.center_y();
            for i in -1..=1 {
                let y = cy + i * 4;
                self.canvas
                    .fill_rect(Rect::new(handle.center_x() - 1, y, 3, 1), line);
            }
        } else {
            self.canvas
                .hline(handle.x, handle.center_y(), handle.w, line);
            let cx = handle.center_x();
            for i in -1..=1 {
                let x = cx + i * 4;
                self.canvas
                    .fill_rect(Rect::new(x, handle.center_y() - 1, 1, 3), line);
            }
        }
    }

    /// A search field: a magnifying glass, the text, and a clear button that
    /// appears once there is something to clear.
    ///
    /// `changed` is true when the text changed, whether by typing or by
    /// clearing, so a caller can treat both the same way.
    pub fn search_field_at(
        &mut self,
        rect: Rect,
        name: &str,
        text: &mut String,
        hint: &str,
    ) -> Response {
        self.search_field_grab_at(rect, name, text, hint, false)
    }

    /// A search field that can be handed the keyboard from outside.
    ///
    /// Pass `grab` on the one frame a shortcut aims at this field. It has to
    /// be a pulse rather than a flag: holding focus every frame would take it
    /// straight back the moment the user clicked somewhere else.
    pub fn search_field_grab_at(
        &mut self,
        rect: Rect,
        name: &str,
        text: &mut String,
        hint: &str,
        grab: bool,
    ) -> Response {
        let th = *self.theme;
        let glass_w = icon::size(icon::SEARCH).0;
        let pad = th.metrics.text_pad;

        let show_clear = !text.is_empty();
        let clear_rect = Rect::new(rect.right() - 13, rect.y + 2, 11, rect.h - 4);

        // The clear button claims the pointer *before* the field does. Both
        // cover the same pixels, and whoever interacts first wins the press —
        // otherwise clicking the cross would only move the caret.
        let clear_id = self.scope(name, |ui| ui.id("clear"));
        let mut cleared = false;
        if show_clear {
            let resp = self.interact(clear_id, clear_rect);
            if resp.hovered {
                self.request_cursor(Cursor::Pointer);
            }
            if resp.clicked {
                // Clear before the field runs, so the box empties this frame
                // rather than a frame late.
                text.clear();
                cleared = true;
            }
        }

        let opts = FieldOpts {
            pad_left: pad + glass_w + 2,
            pad_right: if text.is_empty() { pad } else { 15 },
            grab,
        };
        let mut resp = self.text_field_core(rect, name, text, hint, opts);
        resp.changed |= cleared;

        // ---- the glass ---------------------------------------------------
        let (_, gh) = icon::size(icon::SEARCH);
        let tint = if text.is_empty() && !resp.focused {
            th.ink_light.shade(-0.45)
        } else {
            th.accent.face
        };
        icon::draw(
            self.canvas,
            rect.x + pad,
            rect.y + (rect.h - gh) / 2,
            icon::SEARCH,
            tint,
        );

        // ---- the clear button --------------------------------------------
        if !text.is_empty() {
            let hot = self.is_hot(clear_id);
            if hot {
                self.canvas.fill_chamfer(clear_rect, th.well.shade(0.22), 1);
            }
            let ink = if hot {
                th.danger.face
            } else {
                th.ink_light.shade(-0.25)
            };
            icon::draw_centered(self.canvas, clear_rect, icon::CROSS, ink);
        }

        resp
    }

    // ---------------------------------------------------------------- scroll

    /// A vertically scrollable region.
    ///
    /// Content is laid out in a column inside `rect`, clipped to it, and offset
    /// by the current scroll position. Widgets inside behave normally —
    /// [`Ui::interact`] already honours the clip, so a button scrolled halfway
    /// out of view stops responding exactly where it stops being drawn.
    ///
    /// Content height is measured from the previous frame, which is why the
    /// scrollbar gutter is reserved whether or not a bar is showing: if the
    /// gutter appeared and disappeared, content that reflows on width would
    /// oscillate between two layouts forever.
    ///
    /// Vertical only. A horizontal axis would double the chrome for a case that
    /// pixel-art UIs almost never want.
    pub fn scroll_area<R>(
        &mut self,
        rect: Rect,
        name: &str,
        f: impl FnOnce(&mut Ui) -> R,
    ) -> (R, ScrollState) {
        let id = self.id(name);
        let mut state = self.scroll_state(id);
        let out = self.scroll_area_with(rect, name, &mut state, f);
        self.set_scroll_state(id, state);
        (out, state)
    }

    /// A scroll area whose position the caller owns.
    ///
    /// Widget state is garbage collected when a widget stops being drawn, which
    /// is right for one that has genuinely gone and wrong for one that is
    /// merely not showing — a view behind a tab, say, which should come back
    /// where it was left. Holding the state outside the toolkit is the honest
    /// way to say "this outlives not being drawn".
    pub fn scroll_area_with<R>(
        &mut self,
        rect: Rect,
        name: &str,
        state: &mut ScrollState,
        f: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let th = *self.theme;
        let m = th.metrics;

        let mut st = *state;

        let view = Rect::new(
            rect.x,
            rect.y,
            (rect.w - Self::SCROLL_GUTTER).max(1),
            rect.h,
        );
        let track = Rect::new(rect.right() - Self::BAR_W, rect.y, Self::BAR_W, rect.h);

        // ---- wheel ------------------------------------------------------
        let over = !self.is_input_blocked()
            && !self.pointer_covered()
            && rect.contains(self.input.mouse)
            && self.canvas.clip_contains(self.input.mouse);
        if over && self.input.wheel != 0.0 {
            // Three text lines per notch, the usual convention.
            st.target -= self.input.wheel * 3.0 * font::line_h() as f32;
        }

        // The bar sits in a gutter beside the view rather than over it, so
        // taking the pointer and painting it here — before the content —
        // cannot come between the content and the mouse.
        self.scroll_bar(track, name, &mut st);
        let offset = st.shown.round() as i32;

        // ---- content -----------------------------------------------------
        // Height comes from last frame so `alloc_rest` inside stays sensible.
        let content_h = st.content.max(view.h);
        let bounds = Rect::new(view.x, view.y - offset, view.w, content_h);
        let (result, used) = self.clipped(view, |ui| ui.column_measured(bounds, m.gap, f));
        if used != st.content {
            // The content was laid out against the height measured last frame,
            // so a frame that changes it has drawn the bar — and anything that
            // asked how much room was left — against the old one. Ask for
            // another frame, or the stale one is the one left on screen.
            self.request_repaint();
        }
        st.content = used;
        st.viewport = view.h;

        // Edge shadows: the cheapest possible hint that content continues past
        // the clip, and far less intrusive than a second scrollbar.
        if offset > 0 {
            self.canvas.hline(view.x, view.y, view.w, th.shadow);
        }
        if (offset as f32) < st.max_offset() {
            self.canvas
                .hline(view.x, view.bottom() - 1, view.w, th.shadow);
        }

        *state = st;
        result
    }

    /// Width of a scrollbar, and of the gutter one wants reserved for it.
    pub const BAR_W: i32 = 7;
    pub const SCROLL_GUTTER: i32 = Self::BAR_W + 2;

    /// The scrollbar on its own: drag the thumb, click the track to page, and
    /// ease the offset toward wherever that leaves it.
    ///
    /// A scroll area draws one of these. So does anything that scrolls its own
    /// content by other means and only wants the same bar to report it — an
    /// editor that scrolls by whole lines, say, because that is what a caret
    /// moves in. Two bars that look different for no reason are two bars the
    /// eye has to learn separately.
    ///
    /// The caller owns the state, and is expected to have filled in `content`
    /// and `viewport`; this reads them, and writes `target` and `shown`.
    pub fn scroll_bar(&mut self, track: Rect, name: &str, st: &mut ScrollState) {
        let th = *self.theme;
        let dt = self.input.dt;
        let pointer = self.input.mouse;
        let max = st.max_offset();

        let thumb_id = self.scope(name, |ui| ui.id("thumb"));
        let track_id = self.scope(name, |ui| ui.id("track"));

        // ---- thumb geometry ---------------------------------------------
        let thumb_h = if st.scrollable() {
            let ratio = st.viewport as f32 / st.content.max(1) as f32;
            ((track.h as f32 * ratio).round() as i32).clamp(10, track.h)
        } else {
            track.h
        };
        let travel = (track.h - thumb_h).max(0);
        let t = if max > 0.0 {
            (st.shown / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let thumb = Rect::new(
            track.x,
            track.y + (travel as f32 * t).round() as i32,
            track.w,
            thumb_h,
        );

        // ---- drag the thumb ---------------------------------------------
        let thumb_resp = self.interact(thumb_id, thumb);
        if thumb_resp.held {
            if self.input.mouse_pressed {
                st.grab = pointer.y - thumb.y;
            }
            let want = pointer.y - st.grab;
            let rel = if travel > 0 {
                (want - track.y) as f32 / travel as f32
            } else {
                0.0
            };
            st.target = rel.clamp(0.0, 1.0) * max;
            // A drag should track the pointer exactly, not lag behind it.
            st.shown = st.target;
        }
        if thumb_resp.hovered || thumb_resp.held {
            self.request_cursor(Cursor::Grab);
        }

        // ---- click the track to page ------------------------------------
        let track_resp = self.interact(track_id, track);
        if track_resp.clicked && st.scrollable() {
            let page = st.viewport as f32;
            st.target += if pointer.y < thumb.y { -page } else { page };
        }

        // ---- settle ------------------------------------------------------
        st.target = st.target.clamp(0.0, max);
        st.shown = smooth(st.shown, st.target, 26.0, dt);
        if (st.shown - st.target).abs() < 0.5 {
            st.shown = st.target;
        } else {
            // A view still sliding towards where it was sent needs the frames
            // to slide in. Nothing else will ask for them: a wheel notch is one
            // event and buys one frame, and an ease that takes ten frames would
            // otherwise stop a third of the way there and sit until the pointer
            // moved. Which reads as a scroll that will not go where it is put.
            self.request_repaint();
        }

        let anim = self.animate(thumb_id, &thumb_resp);
        if st.scrollable() {
            self.draw_well(track, th.well);
            self.draw_control_face(thumb.inset_xy(0, 1), th.neutral, &anim);
        }
    }

    // ------------------------------------------------------------------ text

    /// A single line of body text on its own row.
    pub fn label(&mut self, text: &str) -> Rect {
        let color = self.theme.ink;
        self.label_colored(text, color)
    }

    /// Body text in a specific colour.
    pub fn label_colored(&mut self, text: &str, color: Color) -> Rect {
        let h = font::text_height(text) + 2;
        let rect = self.alloc(h);
        self.draw_text_in(rect, text, color, Align::Left);
        rect
    }

    /// De-emphasised text.
    pub fn label_dim(&mut self, text: &str) -> Rect {
        let color = self.theme.ink_soft;
        self.label_colored(text, color)
    }

    /// A section heading with a rule under it.
    pub fn heading(&mut self, text: &str) -> Rect {
        let th = *self.theme;
        let rect = self.alloc(font::glyph_h() + 5);
        self.draw_text_in(
            Rect::new(rect.x, rect.y, rect.w, font::glyph_h()),
            text,
            th.ink,
            Align::Left,
        );
        let y = rect.y + font::glyph_h() + 2;
        self.canvas
            .hline(rect.x, y, rect.w, th.ink.lerp(th.panel, 0.55));
        rect
    }

    /// Label on the left, value on the right, on one row.
    pub fn value_row(&mut self, label: &str, value: &str) -> Rect {
        let th = *self.theme;
        let rect = self.alloc(font::glyph_h() + 3);
        self.draw_text_in(rect, label, th.ink_soft, Align::Left);
        self.draw_text_in(rect, value, th.ink, Align::Right);
        rect
    }

    /// An engraved horizontal rule.
    pub fn separator(&mut self) {
        let th = *self.theme;
        let rect = self.alloc(2);
        self.canvas
            .hline(rect.x, rect.y, rect.w, th.ink.lerp(th.panel, 0.6));
        self.canvas
            .hline(rect.x, rect.y + 1, rect.w, th.panel.shade(0.5));
    }

    // ---------------------------------------------------------------- buttons

    /// A push button on its own row, in the default tone.
    pub fn button(&mut self, label: &str) -> Response {
        let h = self.theme.metrics.control_h;
        let rect = self.alloc(h);
        self.button_at(rect, label, Tone::Neutral)
    }

    /// A push button on its own row, in a specific tone.
    pub fn button_tone(&mut self, label: &str, tone: Tone) -> Response {
        let h = self.theme.metrics.control_h;
        let rect = self.alloc(h);
        self.button_at(rect, label, tone)
    }

    /// A push button at an explicit rect.
    pub fn button_at(&mut self, rect: Rect, label: &str, tone: Tone) -> Response {
        let th = *self.theme;
        let ramp = th.ramp(tone);
        let id = self.id(label);

        // A button's own shadow lives below it, so reserve that space for hit
        // testing too — clicking the shadow of a raised button should work.
        let mut resp = self.interact(id, rect);
        if self.focusable(id) {
            resp.clicked = true;
            self.with_anim(id, |a| a.press.snap(1.0));
        }
        if resp.hovered {
            self.request_cursor(Cursor::Pointer);
        }

        let anim = self.animate(id, &resp);
        let body = self.draw_control_face(rect, ramp, &anim);
        self.draw_focus_ring(rect, &anim);
        self.draw_text_in(body, label, ramp.ink, Align::Center);
        resp.rect = body;
        resp
    }

    /// A push button whose whole face is one icon.
    ///
    /// Named separately from its label because it has none: two icon buttons
    /// with the same drawing are still two buttons, and the name is what keeps
    /// their hover and their press apart.
    pub fn icon_button_at(
        &mut self,
        rect: Rect,
        name: &str,
        rows: &[&str],
        tone: Tone,
    ) -> Response {
        let ramp = self.theme.ramp(tone);
        let id = self.id(name);
        let mut resp = self.interact(id, rect);
        if self.focusable(id) {
            resp.clicked = true;
            self.with_anim(id, |a| a.press.snap(1.0));
        }
        if resp.hovered {
            self.request_cursor(Cursor::Pointer);
        }
        let anim = self.animate(id, &resp);
        let body = self.draw_control_face(rect, ramp, &anim);
        icon::draw_centered(self.canvas, body, rows, ramp.ink);
        resp.rect = body;
        resp
    }

    // ------------------------------------------------------------------ menus

    /// A menu's name in a bar.
    ///
    /// Drawn as a control at rest rather than as a word that happens to react
    /// to the pointer: a chamfered face and a caret under the name, because a
    /// menu nobody can see is a menu nobody opens. It lights up on hover and
    /// sinks in while its menu is down.
    pub fn menu_title(&mut self, rect: Rect, label: &str, open: bool) -> Response {
        let th = *self.theme;
        let id = self.id(label);
        let resp = self.interact(id, rect);
        if resp.hovered {
            self.request_cursor(Cursor::Pointer);
        }

        let face = if open {
            th.accent.lo
        } else if resp.hovered {
            th.accent.hi
        } else {
            th.accent.face.shade(0.10)
        };
        self.canvas.fill_chamfer(rect, face, 1);
        if !open {
            // A lit top edge, the same trick the panel titles use: enough to
            // read as raised without a full border in a thirteen-pixel strip.
            self.canvas
                .hline(rect.x + 1, rect.y, rect.w - 2, th.accent.hi);
        }
        self.canvas
            .stroke_chamfer(rect, th.accent.lo.shade(-0.15), 1);

        let ink = if open { th.neutral.hi } else { th.ink };
        let y = rect.y + (rect.h - font::glyph_h()) / 2;
        font::draw_text_styled(self.canvas, rect.x + 5, y, label, ink, true);

        let caret = Rect::new(rect.right() - 10, rect.y + (rect.h - 3) / 2, 5, 3);
        icon::draw(self.canvas, caret.x, caret.y, icon::CARET_DOWN, ink);
        resp
    }

    /// The list of entries hanging under an open menu.
    ///
    /// Drawn in a layer, so it takes the pointer ahead of whatever it is
    /// covering, and sized from its widest entry. The caller decides what an
    /// entry means and when the menu closes; this only reports what was
    /// pointed at.
    ///
    /// Entries are [`Segment`]s: a menu entry and a segment of a segmented
    /// control are the same thing — a label with an optional picture beside it
    /// — and two structs for that would be two structs to keep in step.
    pub fn menu_items(&mut self, at: Point, items: &[Segment]) -> MenuPick {
        let th = *self.theme;
        let m = th.metrics;
        let row = font::line_h() + 4;
        let w = items.iter().map(Segment::width).max().unwrap_or(40) + 30;
        let h = items.len() as i32 * row + 4;
        let screen = self.canvas.bounds();
        let rect = Rect::new(at.x.min(screen.right() - w - 2), at.y, w, h);

        self.layer(rect, |ui| {
            let th_ = th;
            ui.canvas
                .fill_chamfer(rect.translate(0, m.press_depth), th_.shadow, m.chamfer);
            ui.canvas
                .box_chamfer(rect, th_.panel, th_.panel_border, m.chamfer);

            let mut pick = MenuPick::default();
            for (i, item) in items.iter().enumerate() {
                let cell = Rect::new(rect.x + 2, rect.y + 2 + i as i32 * row, rect.w - 4, row);
                let id = ui.id(item.label);
                let resp = ui.interact(id, cell);
                if resp.hovered {
                    ui.request_cursor(Cursor::Pointer);
                    ui.canvas.fill_chamfer(cell, th_.accent.face, 1);
                }
                let ink = if resp.hovered {
                    th_.accent.ink
                } else {
                    th_.ink
                };
                let y = cell.y + (cell.h - font::glyph_h()) / 2;
                // The icons share a column, so the labels line up whether or
                // not every entry has one.
                let mut x = cell.x + 6;
                if let Some(rows) = item.icon {
                    let (_, ih) = icon::size(rows);
                    icon::draw(ui.canvas, x, cell.y + (cell.h - ih) / 2, rows, ink);
                }
                // A fixed column, wide enough for the widest icon and a gap.
                x += 12;
                font::draw_text(ui.canvas, x, y, item.label, ink);
                if resp.clicked {
                    pick.chosen = Some(i);
                }
            }
            // A press anywhere else puts the menu away, which is what every
            // menu everywhere does and what the hand expects.
            if ui.input.mouse_pressed && !rect.contains(ui.input.mouse) {
                pick.dismissed = true;
            }
            pick
        })
    }

    // ---------------------------------------------------------------- toggles

    /// A checkbox with a label to its right. Returns a response whose
    /// `changed` is true on the frame the value flipped.
    pub fn checkbox(&mut self, label: &str, value: &mut bool) -> Response {
        let h = self.theme.metrics.control_h;
        let rect = self.alloc(h);
        self.checkbox_at(rect, label, value)
    }

    /// A checkbox at an explicit rect.
    pub fn checkbox_at(&mut self, rect: Rect, label: &str, value: &mut bool) -> Response {
        let th = *self.theme;
        let m = th.metrics;
        let id = self.id(label);

        let mut resp = self.interact(id, rect);
        if self.focusable(id) {
            resp.clicked = true;
            self.with_anim(id, |a| a.press.snap(1.0));
        }
        if resp.hovered {
            self.request_cursor(Cursor::Pointer);
        }
        if resp.clicked {
            *value = !*value;
            resp.changed = true;
        }

        let anim = self.animate(id, &resp);
        let side = (rect.h - 2).min(13);
        let box_rect = Rect::new(rect.x, rect.y + (rect.h - side) / 2, side, side);

        // The tick pops in with its own spring rather than appearing instantly.
        let fresh = !self.anim_exists(id);
        let target = if *value { 1.0 } else { 0.0 };
        let dt = self.input.dt;
        let tick = self.with_anim(id, |a| {
            if fresh {
                a.value.snap(target);
            }
            a.value.stiffness = 1100.0;
            a.value.damping = 30.0;
            a.value.step(target, dt);
            a.value.pos
        });

        let body = self.draw_control_face(box_rect, th.neutral, &anim);
        self.draw_focus_ring(box_rect, &anim);

        if tick > 0.05 {
            // Scale the tick by growing it from the centre in whole pixels.
            let t = tick.clamp(0.0, 1.15);
            let size = (7.0 * t).round() as i32;
            if size > 0 {
                let area = body.centered(7, 7);
                let color = th.accent.face.lerp(th.ink, 0.15);
                let skip = (7 - size.min(7)) / 2;
                for (row, bits) in CHECK.iter().enumerate() {
                    let ry = row as i32;
                    if ry < skip || ry >= 7 - skip {
                        continue;
                    }
                    for col in 0..7 {
                        if bits & (1 << (6 - col)) != 0 {
                            self.canvas.set_px(area.x + col, area.y + ry, color);
                        }
                    }
                }
            }
        }

        let text = Rect::from_min_max(
            box_rect.right() + m.text_pad,
            rect.y,
            rect.right(),
            rect.bottom(),
        );
        self.draw_text_in(text, label, th.ink, Align::Left);
        resp.rect = rect;
        resp
    }

    /// A sliding switch with a label to its right.
    pub fn toggle(&mut self, label: &str, value: &mut bool) -> Response {
        let h = self.theme.metrics.control_h;
        let rect = self.alloc(h);
        self.toggle_at(rect, label, value)
    }

    /// A sliding switch at an explicit rect.
    pub fn toggle_at(&mut self, rect: Rect, label: &str, value: &mut bool) -> Response {
        let th = *self.theme;
        let m = th.metrics;
        let id = self.id(label);

        let mut resp = self.interact(id, rect);
        if self.focusable(id) {
            resp.clicked = true;
            self.with_anim(id, |a| a.press.snap(1.0));
        }
        if resp.hovered {
            self.request_cursor(Cursor::Pointer);
        }
        if resp.clicked {
            *value = !*value;
            resp.changed = true;
        }
        let anim = self.animate(id, &resp);

        let fresh = !self.anim_exists(id);
        let target = if *value { 1.0 } else { 0.0 };
        let dt = self.input.dt;
        let slide = self.with_anim(id, |a| {
            if fresh {
                a.value.snap(target);
            }
            a.value.stiffness = 800.0;
            a.value.damping = 32.0;
            a.value.step(target, dt);
            a.value.pos.clamp(-0.15, 1.15)
        });

        let track_h = (rect.h - 3).max(7);
        let track = Rect::new(rect.x, rect.y + (rect.h - track_h) / 2, 24, track_h);
        let fill = th.well.lerp(th.positive.face, slide.clamp(0.0, 1.0));
        self.draw_well(track, fill);

        let knob_w = 10;
        let travel = track.w - knob_w - 2;
        let knob_x = track.x + 1 + (travel as f32 * slide).round() as i32;
        let knob = Rect::new(knob_x, track.y - 1, knob_w, track.h + 2);
        let body = self.draw_control_face(knob, th.neutral, &anim);

        // Two grip lines, the universal shorthand for "this thing slides".
        let gx = body.center_x();
        let gy = body.y + 3;
        let gh = (body.h - 6).max(1);
        self.canvas.vline(gx - 1, gy, gh, th.neutral.lo);
        self.canvas.vline(gx + 1, gy, gh, th.neutral.lo);

        self.draw_focus_ring(track, &anim);

        let text = Rect::from_min_max(
            track.right() + m.text_pad,
            rect.y,
            rect.right(),
            rect.bottom(),
        );
        self.draw_text_in(text, label, th.ink, Align::Left);
        resp.rect = rect;
        resp
    }

    // ---------------------------------------------------------------- sliders

    /// A horizontal slider over `min..=max`. `changed` is true while dragging.
    pub fn slider(&mut self, label: &str, value: &mut f32, min: f32, max: f32) -> Response {
        let h = self.theme.metrics.control_h;
        let rect = self.alloc(h);
        self.slider_at(rect, label, value, min, max)
    }

    /// A slider at an explicit rect.
    pub fn slider_at(
        &mut self,
        rect: Rect,
        label: &str,
        value: &mut f32,
        min: f32,
        max: f32,
    ) -> Response {
        let th = *self.theme;
        let id = self.id(label);
        let span = (max - min).abs().max(f32::EPSILON);

        let mut resp = self.interact(id, rect);
        let focused_key = self.focusable(id);
        if resp.hovered || resp.held {
            self.request_cursor(Cursor::Grab);
        }

        let knob_w = 9;
        let track = Rect::new(rect.x, rect.center_y() - 3, rect.w, 7);
        let travel = (track.w - knob_w).max(1);

        if resp.held {
            let rel = (self.input.mouse.x - track.x - knob_w / 2) as f32 / travel as f32;
            let next = min + rel.clamp(0.0, 1.0) * (max - min);
            if (next - *value).abs() > f32::EPSILON {
                *value = next;
                resp.changed = true;
            }
        }
        // Space and Enter mean nothing on a slider; the arrow keys do.
        let _ = focused_key;
        if resp.focused {
            let step = span / 20.0;
            if self.input.key_pressed(crate::input::Key::Left) {
                *value = (*value - step).clamp(min.min(max), max.max(min));
                resp.changed = true;
            }
            if self.input.key_pressed(crate::input::Key::Right) {
                *value = (*value + step).clamp(min.min(max), max.max(min));
                resp.changed = true;
            }
        }

        let t = ((*value - min) / (max - min)).clamp(0.0, 1.0);
        let anim = self.animate(id, &resp);

        self.draw_well(track, th.well);
        let filled = (travel as f32 * t).round() as i32 + knob_w / 2;
        if filled > 2 {
            let fill_rect = Rect::new(track.x + 1, track.y + 1, filled - 1, track.h - 2);
            self.canvas.fill_rect(fill_rect, th.accent.face);
            self.canvas
                .gradient_rect(fill_rect, th.accent.face, th.accent.hi, true);
        }

        let knob_x = track.x + (travel as f32 * t).round() as i32;
        let knob = Rect::new(knob_x, rect.y + 1, knob_w, rect.h - 2);
        let body = self.draw_control_face(knob, th.neutral, &anim);
        let gy = body.y + 3;
        let gh = (body.h - 6).max(1);
        self.canvas.vline(body.center_x(), gy, gh, th.neutral.lo);

        self.draw_focus_ring(track, &anim);
        resp.rect = rect;
        resp
    }

    /// A labelled slider row: caption and readout above, track below.
    pub fn slider_labeled(
        &mut self,
        label: &str,
        value: &mut f32,
        min: f32,
        max: f32,
        readout: &str,
    ) -> Response {
        self.value_row(label, readout);
        self.slider(label, value, min, max)
    }

    // --------------------------------------------------------------- readouts

    /// A progress bar. `t` is clamped to `0..=1`.
    pub fn progress(&mut self, t: f32, tone: Tone) -> Rect {
        let h = self.theme.metrics.control_h - 4;
        let rect = self.alloc(h);
        self.progress_at(rect, t, tone)
    }

    /// A progress bar at an explicit rect.
    pub fn progress_at(&mut self, rect: Rect, t: f32, tone: Tone) -> Rect {
        let th = *self.theme;
        let ramp = th.ramp(tone);
        let t = t.clamp(0.0, 1.0);
        self.draw_well(rect, th.well);

        let inner = rect.inset(2);
        let w = (inner.w as f32 * t).round() as i32;
        if w > 0 {
            let filled = Rect::new(inner.x, inner.y, w, inner.h);
            self.canvas.gradient_rect(filled, ramp.hi, ramp.face, true);
            // A dithered leading edge stops the bar looking like a hard cut.
            let edge = Rect::new(inner.x + w - 2, inner.y, 2.min(w), inner.h);
            self.canvas.dither_rect(edge, ramp.face, ramp.lo, 0.5);
        }
        rect
    }

    /// A row of mutually exclusive options sharing one frame.
    ///
    /// Returns true if the selection changed this frame.
    pub fn segmented(&mut self, id_label: &str, options: &[&str], selected: &mut usize) -> bool {
        let h = self.theme.metrics.control_h;
        let rect = self.alloc(h);
        self.segmented_at(id_label, rect, options, selected)
    }

    /// A segmented control at an explicit rect.
    pub fn segmented_at(
        &mut self,
        id_label: &str,
        rect: Rect,
        options: &[&str],
        selected: &mut usize,
    ) -> bool {
        let segments: Vec<Segment> = options.iter().map(|l| Segment::new(l)).collect();
        self.segments_at(id_label, rect, &segments, selected)
    }

    /// A segmented control whose options may carry an icon.
    ///
    /// The icon and its label are laid out as one group and centred together,
    /// rather than the icon being pinned to the edge — a tab with a picture
    /// stuck to its left and the word floating in the middle reads as two
    /// things that happen to share a box.
    pub fn segments_at(
        &mut self,
        id_label: &str,
        rect: Rect,
        options: &[Segment],
        selected: &mut usize,
    ) -> bool {
        if options.is_empty() {
            return false;
        }
        let th = *self.theme;
        let m = th.metrics;
        let mut changed = false;

        self.canvas
            .fill_chamfer(rect.translate(0, m.press_depth), th.shadow, m.chamfer);
        self.canvas
            .box_chamfer(rect, th.well, th.panel_border, m.chamfer);

        // One pixel of well shows around the cells, not two: each cell is a
        // control face now, and it needs the height for its own lit edges —
        // squeeze it and those edges cut through the letters.
        let inner = rect.inset(1);
        let n = options.len() as i32;
        let cell_w = inner.w / n;

        self.scope(id_label, |ui| {
            for (i, opt) in options.iter().enumerate() {
                let x = inner.x + cell_w * i as i32;
                let w = if i as i32 == n - 1 {
                    inner.right() - x
                } else {
                    cell_w
                };
                let cell = Rect::new(x, inner.y, w, inner.h - m.press_depth);
                let id = ui.id(opt.label);
                let mut resp = ui.interact(id, cell);
                if ui.focusable(id) {
                    resp.clicked = true;
                }
                if resp.hovered {
                    ui.request_cursor(Cursor::Pointer);
                }
                if resp.clicked && *selected != i {
                    *selected = i;
                    changed = true;
                }

                // Selection drives the same spring a button's press does, so
                // choosing a segment lifts it out of the strip with the bounce
                // a released button has, and the one it replaces sinks. The
                // colour crosses over on the same spring, which is what keeps
                // the two halves of the swap reading as one movement.
                let is_sel = *selected == i;
                let dt = ui.input.dt;
                let anim = ui.with_anim(id, |a| {
                    a.press
                        .step(if is_sel && !resp.held { 0.0 } else { 1.0 }, dt);
                    a.hover = smooth(a.hover, if resp.hovered { 1.0 } else { 0.0 }, 24.0, dt);
                    a.focus = smooth(a.focus, if resp.focused { 1.0 } else { 0.0 }, 20.0, dt);
                    a.flash = smooth(a.flash, 0.0, 11.0, dt);
                    if resp.clicked {
                        a.flash = 1.0;
                    }
                    *a
                });

                let lift = (1.0 - anim.press.pos).clamp(0.0, 1.0);
                let ramp = Ramp {
                    face: th.well.lerp(th.accent.face, lift),
                    hi: th.well.shade(0.16).lerp(th.accent.hi, lift),
                    lo: th.well.shade(-0.16).lerp(th.accent.lo, lift),
                    ink: th.ink_light.lerp(th.accent.ink, lift),
                };
                let body = ui.draw_control_face(cell, ramp, &anim);
                draw_segment(ui, body, opt, ramp.ink);
            }
        });

        changed
    }
}

/// What an open menu reported this frame.
#[derive(Clone, Copy, Default)]
pub struct MenuPick {
    /// The entry that was clicked.
    pub chosen: Option<usize>,
    /// A press landed outside the menu, so it should be put away.
    pub dismissed: bool,
}
