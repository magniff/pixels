//! The widget set.
//!
//! Every control here is built from the same three moves: a shadow silhouette,
//! a chamfered face that physically sinks into that shadow when pressed, and a
//! one-pixel light/dark edge that inverts on the way down. Getting those three
//! right is most of what makes a pixel UI feel tactile rather than flat.

use crate::anim::{smooth, WidgetAnim};
use crate::color::Color;
use crate::font;
use crate::geom::Rect;
use crate::input::{Cursor, Key};
use crate::layout::Align;
use crate::theme::{Ramp, Tone};
use crate::ui::{Response, ScrollState, Ui};

const WHITE: Color = Color::hex(0xFFFFFF);

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
        let phase = (self.input.time * 14.0) as i32;
        self.canvas
            .stroke_rect_dashed(rect.inset(-2), color, 2, 2, phase);
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
            // A dithered fade across the strip keeps a flat fill from looking dead.
            self.canvas.gradient_rect(
                strip.inset_xy(1, 1),
                th.panel_title,
                th.panel_title.shade(0.18),
                false,
            );
            self.canvas
                .hline(strip.x, strip.bottom(), strip.w, th.panel_border);
            let text = strip.translate(m.text_pad, 0);
            self.draw_text_in_shadow(
                text,
                title,
                th.panel_title_ink,
                th.panel_title.shade(0.30),
                Align::Left,
            );
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
        let th = *self.theme;
        let m = th.metrics;
        let id = self.id(name);

        let mut resp = self.interact(id, rect);
        self.focusable(id);
        if resp.hovered {
            self.request_cursor(Cursor::Text);
        }

        let mut st = self.text_state(id);
        let chars: Vec<char> = text.chars().collect();
        st.caret = st.caret.min(chars.len());

        let inner = rect.inset_xy(m.text_pad, 0);

        // Clicking places the caret at the nearest character boundary.
        if resp.held {
            let local = self.input.mouse.x - inner.x + st.scroll;
            let col = ((local + font::ADVANCE / 2) / font::ADVANCE).max(0) as usize;
            st.caret = col.min(chars.len());
        }

        if resp.focused && !self.is_input_blocked() {
            let mut edited = chars.clone();
            for key in &self.input.keys {
                match key {
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
        let caret_x = st.caret as i32 * font::ADVANCE;
        if caret_x - st.scroll > inner.w - font::ADVANCE {
            st.scroll = caret_x - inner.w + font::ADVANCE;
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
            self.canvas
                .stroke_chamfer(rect, ring, (m.chamfer - 1).max(0));
        }

        self.clipped(inner, |ui| {
            let y = rect.y + (rect.h - font::GLYPH_H) / 2;
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
                        .fill_rect(Rect::new(cx, y - 1, 1, font::GLYPH_H + 2), th.accent.face);
                }
            }
        });

        self.set_text_state(id, st);
        resp.rect = rect;
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
        let th = *self.theme;
        let m = th.metrics;
        let dt = self.input.dt;

        let id = self.id(name);
        let thumb_id = self.scope(name, |ui| ui.id("thumb"));
        let track_id = self.scope(name, |ui| ui.id("track"));
        let mut st = self.scroll_state(id);

        const BAR_W: i32 = 7;
        let gutter = BAR_W + 2;
        let view = Rect::new(rect.x, rect.y, (rect.w - gutter).max(1), rect.h);
        let track = Rect::new(rect.right() - BAR_W, rect.y, BAR_W, rect.h);
        let max = st.max_offset();

        // ---- wheel ------------------------------------------------------
        let pointer = self.input.mouse;
        let over = !self.is_input_blocked()
            && rect.contains(pointer)
            && self.canvas.clip_contains(pointer);
        if over && self.input.wheel != 0.0 {
            // Three text lines per notch, the usual convention.
            st.target -= self.input.wheel * 3.0 * font::LINE_H as f32;
        }

        // ---- thumb geometry ---------------------------------------------
        let thumb_h = if st.scrollable() {
            let ratio = view.h as f32 / st.content.max(1) as f32;
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
            BAR_W,
            thumb_h,
        );

        // ---- drag the thumb ---------------------------------------------
        // The thumb is interacted with before the content so that it wins the
        // pointer where they overlap: it is drawn on top, so it must be hit
        // first.
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
            let page = view.h as f32;
            st.target += if pointer.y < thumb.y { -page } else { page };
        }

        // ---- settle ------------------------------------------------------
        st.target = st.target.clamp(0.0, max);
        st.shown = smooth(st.shown, st.target, 26.0, dt);
        if (st.shown - st.target).abs() < 0.5 {
            st.shown = st.target;
        }
        let offset = st.shown.round() as i32;

        // ---- content -----------------------------------------------------
        // Height comes from last frame so `alloc_rest` inside stays sensible.
        let content_h = st.content.max(view.h);
        let bounds = Rect::new(view.x, view.y - offset, view.w, content_h);
        let (result, used) = self.clipped(view, |ui| ui.column_measured(bounds, m.gap, f));
        st.content = used;
        st.viewport = view.h;

        // ---- chrome, drawn last so it sits above the content -------------
        let anim = self.animate(thumb_id, &thumb_resp);
        if st.scrollable() {
            self.draw_well(track, th.well);
            self.draw_control_face(thumb.inset_xy(0, 1), th.neutral, &anim);
        }
        // Edge shadows: the cheapest possible hint that content continues past
        // the clip, and far less intrusive than a second scrollbar.
        if offset > 0 {
            self.canvas.hline(view.x, view.y, view.w, th.shadow);
        }
        if (offset as f32) < max {
            self.canvas
                .hline(view.x, view.bottom() - 1, view.w, th.shadow);
        }

        self.set_scroll_state(id, st);
        (result, st)
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
        let rect = self.alloc(font::GLYPH_H + 5);
        self.draw_text_in(
            Rect::new(rect.x, rect.y, rect.w, font::GLYPH_H),
            text,
            th.ink,
            Align::Left,
        );
        let y = rect.y + font::GLYPH_H + 2;
        self.canvas
            .hline(rect.x, y, rect.w, th.ink.lerp(th.panel, 0.55));
        rect
    }

    /// Label on the left, value on the right, on one row.
    pub fn value_row(&mut self, label: &str, value: &str) -> Rect {
        let th = *self.theme;
        let rect = self.alloc(font::GLYPH_H + 3);
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

        let inner = rect.inset(2);
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
                let cell = Rect::new(x, inner.y, w, inner.h);
                let id = ui.id(opt);
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

                let is_sel = *selected == i;
                let anim = ui.animate(id, &resp);
                if is_sel {
                    let ramp = th.accent;
                    let face = ramp
                        .face
                        .lerp(ramp.hi, anim.hover * 0.3)
                        .lerp(WHITE, anim.flash * 0.4);
                    ui.canvas.box_chamfer(cell, face, th.panel_border, 1);
                    ui.canvas.hline(cell.x + 1, cell.y + 1, cell.w - 2, ramp.hi);
                    ui.draw_text_in(cell, opt, ramp.ink, Align::Center);
                } else {
                    let ink = th.ink_light.lerp(WHITE, anim.hover * 0.5);
                    if anim.hover > 0.01 {
                        ui.canvas
                            .fill_chamfer(cell, th.well.shade(0.12 * anim.hover), 1);
                    }
                    ui.draw_text_in(cell, opt, ink, Align::Center);
                }
            }
        });

        changed
    }
}
