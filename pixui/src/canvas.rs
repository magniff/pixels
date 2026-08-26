//! The software rasteriser.
//!
//! At the resolutions pixui targets (a few hundred pixels on a side) a frame is
//! well under 150k pixels, so the whole UI is drawn on the CPU into a flat
//! `Vec<u32>`. No shaders, no GPU pipeline, no render graph — which is why the
//! entire renderer fits in one readable file.

use crate::color::Color;
use crate::geom::{Point, Rect};

/// A 4x4 Bayer matrix, scaled to `0..16`. Ordered dithering is how you get a
/// gradient out of a sixteen-colour palette without inventing new colours.
const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// A fixed-size RGB pixel buffer with a clip stack.
pub struct Canvas {
    width: i32,
    height: i32,
    pixels: Vec<u32>,
    clip: Rect,
    clip_stack: Vec<Rect>,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        Self {
            width,
            height,
            pixels: vec![0; (width * height) as usize],
            clip: Rect::new(0, 0, width, height),
            clip_stack: Vec::new(),
        }
    }

    /// Re-allocate for a new size. Contents are discarded; every frame starts
    /// with a clear anyway.
    pub fn resize(&mut self, width: i32, height: i32) {
        let width = width.max(1);
        let height = height.max(1);
        if (self.width, self.height) == (width, height) {
            return;
        }
        self.width = width;
        self.height = height;
        self.pixels.clear();
        self.pixels.resize((width * height) as usize, 0);
        self.clip = Rect::new(0, 0, width, height);
        self.clip_stack.clear();
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    /// The full canvas area, ignoring the current clip.
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// Raw pixels, row-major, `0x00RRGGBB`. Used by the presenter.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    // ---------------------------------------------------------------- clipping

    /// Intersect the clip rect with `rect` until the matching [`Canvas::pop_clip`].
    pub fn push_clip(&mut self, rect: Rect) {
        self.clip_stack.push(self.clip);
        self.clip = self.clip.intersect(rect);
    }

    /// Draw over everything, whatever was clipping until now.
    ///
    /// For a layer: something floating above the interface is not part of the
    /// pane it was opened over, and a menu cut off at the edge of the drawer it
    /// came from is a menu with its answers missing.
    pub fn push_no_clip(&mut self) {
        self.clip_stack.push(self.clip);
        self.clip = Rect::new(0, 0, self.width, self.height);
    }

    pub fn pop_clip(&mut self) {
        if let Some(prev) = self.clip_stack.pop() {
            self.clip = prev;
        }
    }

    pub fn clip_rect(&self) -> Rect {
        self.clip
    }

    /// Whether `p` is inside the current clip — the geometric half of hit testing.
    pub fn clip_contains(&self, p: Point) -> bool {
        self.clip.contains(p)
    }

    // ----------------------------------------------------------------- drawing

    pub fn clear(&mut self, color: Color) {
        self.pixels.fill(color.0);
    }

    #[inline]
    pub fn set_px(&mut self, x: i32, y: i32, color: Color) {
        if self.clip.contains(Point::new(x, y)) {
            self.pixels[(y * self.width + x) as usize] = color.0;
        }
    }

    /// Read a pixel back. Returns black outside the canvas.
    #[inline]
    pub fn get_px(&self, x: i32, y: i32) -> Color {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            Color(self.pixels[(y * self.width + x) as usize])
        } else {
            Color(0)
        }
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let r = rect.intersect(self.clip);
        if r.is_empty() {
            return;
        }
        for y in r.y..r.bottom() {
            let start = (y * self.width + r.x) as usize;
            self.pixels[start..start + r.w as usize].fill(color.0);
        }
    }

    /// Fill blended towards `color` by `alpha` (`0.0..=1.0`). Used sparingly —
    /// translucency is not really a pixel-art idiom, but it is the cheapest way
    /// to do a click flash or a modal scrim.
    pub fn fill_rect_blend(&mut self, rect: Rect, color: Color, alpha: f32) {
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let r = rect.intersect(self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let i = (y * self.width + x) as usize;
                self.pixels[i] = Color(self.pixels[i]).lerp(color, alpha).0;
            }
        }
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, w, 1), color);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, color: Color) {
        self.fill_rect(Rect::new(x, y, 1, h), color);
    }

    /// A one-pixel outline drawn *inside* `rect`.
    pub fn stroke_rect(&mut self, rect: Rect, color: Color) {
        if rect.is_empty() {
            return;
        }
        self.hline(rect.x, rect.y, rect.w, color);
        self.hline(rect.x, rect.bottom() - 1, rect.w, color);
        self.vline(rect.x, rect.y, rect.h, color);
        self.vline(rect.right() - 1, rect.y, rect.h, color);
    }

    /// A dashed outline, `on`/`off` in pixels. Reads as a focus ring.
    pub fn stroke_rect_dashed(&mut self, rect: Rect, color: Color, on: i32, off: i32, phase: i32) {
        if rect.is_empty() || on <= 0 {
            return;
        }
        let period = on + off.max(0);
        let mut step = phase.rem_euclid(period);
        let dash = |c: &mut Canvas, x: i32, y: i32, step: &mut i32| {
            if *step % period < on {
                c.set_px(x, y, color);
            }
            *step += 1;
        };
        for x in rect.x..rect.right() {
            dash(self, x, rect.y, &mut step);
        }
        for y in rect.y + 1..rect.bottom() {
            dash(self, rect.right() - 1, y, &mut step);
        }
        for x in (rect.x..rect.right() - 1).rev() {
            dash(self, x, rect.bottom() - 1, &mut step);
        }
        for y in (rect.y + 1..rect.bottom() - 1).rev() {
            dash(self, rect.x, y, &mut step);
        }
    }

    // ---------------------------------------------------------------- chamfers

    /// How far row `y` is pulled in from the edges, for a rect with corners cut
    /// by `chamfer` pixels.
    #[inline]
    fn chamfer_inset(rect: Rect, y: i32, chamfer: i32) -> i32 {
        let from_top = y - rect.y;
        let from_bottom = rect.bottom() - 1 - y;
        let d = from_top.min(from_bottom);
        if d < chamfer {
            chamfer - d
        } else {
            0
        }
    }

    /// A filled rect with its corners cut on the diagonal.
    ///
    /// This is pixui's rounded rectangle. Cutting one or two pixels off each
    /// corner reads as "soft" at these sizes, costs nothing, and — unlike a real
    /// radius — never produces a half-lit edge pixel.
    pub fn fill_chamfer(&mut self, rect: Rect, color: Color, chamfer: i32) {
        if rect.is_empty() {
            return;
        }
        let chamfer = chamfer.clamp(0, rect.w.min(rect.h) / 2);
        for y in rect.y..rect.bottom() {
            let inset = Self::chamfer_inset(rect, y, chamfer);
            self.hline(rect.x + inset, y, rect.w - inset * 2, color);
        }
    }

    /// A chamfered box: one-pixel `border`, `fill` inside.
    pub fn box_chamfer(&mut self, rect: Rect, fill: Color, border: Color, chamfer: i32) {
        self.fill_chamfer(rect, border, chamfer);
        self.fill_chamfer(rect.inset(1), fill, (chamfer - 1).max(0));
    }

    /// A chamfered outline with nothing drawn inside it.
    pub fn stroke_chamfer(&mut self, rect: Rect, color: Color, chamfer: i32) {
        if rect.is_empty() {
            return;
        }
        let chamfer = chamfer.clamp(0, rect.w.min(rect.h) / 2);
        for y in rect.y..rect.bottom() {
            let inset = Self::chamfer_inset(rect, y, chamfer);
            let inner = Self::chamfer_inset(rect.inset(1), y, (chamfer - 1).max(0));
            let on_cap = y < rect.y + 1 || y >= rect.bottom() - 1;
            if on_cap {
                self.hline(rect.x + inset, y, rect.w - inset * 2, color);
            } else {
                let left = rect.x + inset;
                let right = rect.right() - inset;
                let inner_left = rect.x + 1 + inner;
                let inner_right = rect.right() - 1 - inner;
                self.hline(left, y, (inner_left - left).max(1), color);
                self.hline(inner_right, y, (right - inner_right).max(1), color);
            }
        }
    }

    // ----------------------------------------------------------------- texture

    /// Ordered-dither between two colours. `t` of 0 is all `a`, 1 is all `b`.
    /// The threshold comes from screen position, so adjacent fills tile
    /// seamlessly instead of showing a seam.
    pub fn dither_rect(&mut self, rect: Rect, a: Color, b: Color, t: f32) {
        let r = rect.intersect(self.clip);
        let level = (t.clamp(0.0, 1.0) * 16.0) as u8;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let threshold = BAYER4[(y & 3) as usize][(x & 3) as usize];
                let c = if threshold < level { b } else { a };
                self.pixels[(y * self.width + x) as usize] = c.0;
            }
        }
    }

    /// A horizontal dithered ramp across `rect`.
    pub fn gradient_rect(&mut self, rect: Rect, a: Color, b: Color, vertical: bool) {
        let r = rect.intersect(self.clip);
        if r.is_empty() {
            return;
        }
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let t = if vertical {
                    (y - rect.y) as f32 / (rect.h.max(1) - 1).max(1) as f32
                } else {
                    (x - rect.x) as f32 / (rect.w.max(1) - 1).max(1) as f32
                };
                let level = (t.clamp(0.0, 1.0) * 16.0) as u8;
                let threshold = BAYER4[(y & 3) as usize][(x & 3) as usize];
                let c = if threshold < level { b } else { a };
                self.pixels[(y * self.width + x) as usize] = c.0;
            }
        }
    }

    /// A two-tone checkerboard with `size`-pixel cells.
    pub fn checker(&mut self, rect: Rect, a: Color, b: Color, size: i32) {
        let r = rect.intersect(self.clip);
        let size = size.max(1);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let c = if ((x.div_euclid(size)) + (y.div_euclid(size))) & 1 == 0 {
                    a
                } else {
                    b
                };
                self.pixels[(y * self.width + x) as usize] = c.0;
            }
        }
    }

    /// Copy a rectangle out of `src` into this canvas, top-left at `at`.
    pub fn blit_from(&mut self, src: &Canvas, from: Rect, at: Point) {
        let from = from.intersect(src.bounds());
        for row in 0..from.h {
            for col in 0..from.w {
                let px = src.get_px(from.x + col, from.y + row);
                self.set_px(at.x + col, at.y + row, px);
            }
        }
    }

    /// Paint `color` over `rect` at a fractional coverage, leaving the pixels
    /// it does not take untouched.
    ///
    /// Half a colour, for a palette that has no half colours: `amount` is the
    /// share of pixels the ordered dither hands to `color`, so anything can be
    /// drawn at partial strength over whatever is already there.
    pub fn dither_fill(&mut self, rect: Rect, color: Color, amount: f32) {
        let level = (amount.clamp(0.0, 1.0) * 16.0) as u8;
        if level == 0 {
            return;
        }
        let r = rect.intersect(self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                if BAYER4[(y & 3) as usize][(x & 3) as usize] < level {
                    self.pixels[(y * self.width + x) as usize] = color.0;
                }
            }
        }
    }

    /// Composite `src` over this canvas within `rect`, keeping the fraction
    /// `amount` of it, chosen per pixel by an ordered dither.
    ///
    /// A cross-fade with no intermediate colours. Blending two images properly
    /// needs colours between them, which a sixteen-tone palette does not have;
    /// choosing one source or the other per pixel gets the same read out of the
    /// pixels that exist. It is the same bargain the dithered gradients make,
    /// and it is what these transitions looked like when palettes were this
    /// small for real.
    pub fn dither_over(&mut self, rect: Rect, src: &Canvas, src_origin: Point, amount: f32) {
        let level = (amount.clamp(0.0, 1.0) * 16.0) as u8;
        if level == 0 {
            return;
        }
        let r = rect.intersect(self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                if BAYER4[(y & 3) as usize][(x & 3) as usize] >= level {
                    continue;
                }
                let px = src.get_px(src_origin.x + (x - rect.x), src_origin.y + (y - rect.y));
                self.pixels[(y * self.width + x) as usize] = px.0;
            }
        }
    }

    /// Darken every other row. A light touch of this sells the CRT without
    /// making anything harder to read.
    pub fn scanlines(&mut self, rect: Rect, strength: f32) {
        let r = rect.intersect(self.clip);
        for y in (r.y..r.bottom()).step_by(2) {
            for x in r.x..r.right() {
                let i = (y * self.width + x) as usize;
                self.pixels[i] = Color(self.pixels[i]).shade(-strength).0;
            }
        }
    }
}
