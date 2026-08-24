//! A deliberately small layout model: a stack of cursors that hand out rects.
//!
//! There is no constraint solver here and no flexbox. In a fixed low-resolution
//! canvas you almost always know the pixel sizes you want, and pretending
//! otherwise costs more than it buys.

use crate::geom::Rect;

/// The axis a [`Layout`] advances along.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Vertical,
    Horizontal,
}

/// A cursor walking down (or across) a bounding rect, handing out sub-rects.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub bounds: Rect,
    pub dir: Dir,
    pub spacing: i32,
    cursor: i32,
    started: bool,
}

impl Layout {
    pub fn new(bounds: Rect, dir: Dir, spacing: i32) -> Self {
        Self {
            bounds,
            dir,
            spacing,
            cursor: 0,
            started: false,
        }
    }

    /// Total extent of the bounding rect along the layout axis.
    fn extent(&self) -> i32 {
        match self.dir {
            Dir::Vertical => self.bounds.h,
            Dir::Horizontal => self.bounds.w,
        }
    }

    /// Take `main` pixels along the axis, spanning the full cross axis.
    pub fn alloc(&mut self, main: i32) -> Rect {
        if self.started {
            self.cursor += self.spacing;
        }
        self.started = true;
        let start = self.cursor;
        self.cursor += main;
        match self.dir {
            Dir::Vertical => Rect::new(self.bounds.x, self.bounds.y + start, self.bounds.w, main),
            Dir::Horizontal => Rect::new(self.bounds.x + start, self.bounds.y, main, self.bounds.h),
        }
    }

    /// Take a fixed box, aligned to the start of the cross axis.
    pub fn alloc_sized(&mut self, w: i32, h: i32) -> Rect {
        let main = match self.dir {
            Dir::Vertical => h,
            Dir::Horizontal => w,
        };
        let cell = self.alloc(main);
        Rect::new(cell.x, cell.y, w, h)
    }

    /// Take everything that is left.
    pub fn alloc_rest(&mut self) -> Rect {
        self.alloc(
            (self.extent() - self.cursor - if self.started { self.spacing } else { 0 }).max(0),
        )
    }

    /// Advance without producing a rect.
    pub fn skip(&mut self, main: i32) {
        self.cursor += main;
    }

    /// The unallocated remainder, without consuming it.
    pub fn remaining(&self) -> Rect {
        let used = self.cursor + if self.started { self.spacing } else { 0 };
        let left = (self.extent() - used).max(0);
        match self.dir {
            Dir::Vertical => Rect::new(self.bounds.x, self.bounds.y + used, self.bounds.w, left),
            Dir::Horizontal => Rect::new(self.bounds.x + used, self.bounds.y, left, self.bounds.h),
        }
    }

    /// Pixels consumed so far, spacing included.
    pub fn used(&self) -> i32 {
        self.cursor
    }
}

/// Horizontal alignment for text inside a rect.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}
