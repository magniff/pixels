//! Integer geometry. Everything in pixui is measured in *virtual pixels* — the
//! chunky units the user actually sees. There are no floats in layout, ever;
//! that is what keeps the look crisp.

/// A point in virtual-pixel space.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned rectangle, `x`/`y` inclusive, `w`/`h` exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const ZERO: Rect = Rect::new(0, 0, 0, 0);

    pub const fn from_min_max(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn center_x(&self) -> i32 {
        self.x + self.w / 2
    }

    pub const fn center_y(&self) -> i32 {
        self.y + self.h / 2
    }

    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Shrink by `n` on every side (negative grows).
    pub const fn inset(&self, n: i32) -> Rect {
        Rect::new(self.x + n, self.y + n, self.w - n * 2, self.h - n * 2)
    }

    pub const fn inset_xy(&self, nx: i32, ny: i32) -> Rect {
        Rect::new(self.x + nx, self.y + ny, self.w - nx * 2, self.h - ny * 2)
    }

    pub const fn translate(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn intersect(&self, other: Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }

    /// Split `n` pixels off the left edge, returning `(taken, remainder)`.
    pub fn split_left(&self, n: i32) -> (Rect, Rect) {
        let n = n.clamp(0, self.w);
        (
            Rect::new(self.x, self.y, n, self.h),
            Rect::new(self.x + n, self.y, self.w - n, self.h),
        )
    }

    /// Split `n` pixels off the top edge, returning `(taken, remainder)`.
    pub fn split_top(&self, n: i32) -> (Rect, Rect) {
        let n = n.clamp(0, self.h);
        (
            Rect::new(self.x, self.y, self.w, n),
            Rect::new(self.x, self.y + n, self.w, self.h - n),
        )
    }

    /// Split `n` pixels off the bottom edge, returning `(remainder, taken)`.
    pub fn split_bottom(&self, n: i32) -> (Rect, Rect) {
        let n = n.clamp(0, self.h);
        (
            Rect::new(self.x, self.y, self.w, self.h - n),
            Rect::new(self.x, self.bottom() - n, self.w, n),
        )
    }

    /// Centre a `w` x `h` box inside this rect.
    pub const fn centered(&self, w: i32, h: i32) -> Rect {
        Rect::new(self.x + (self.w - w) / 2, self.y + (self.h - h) / 2, w, h)
    }
}
