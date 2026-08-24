//! Platform-neutral input.
//!
//! Nothing in this module mentions winit. That is deliberate: the backend
//! translates into these types, so an application built on pixui never depends
//! on the windowing crate, and the backend can be swapped without touching a
//! single widget.

use crate::geom::Point;

/// A key pixui cares about. Deliberately small — this is a UI toolkit, not a
/// keyboard driver.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Key {
    Tab,
    Enter,
    Space,
    Escape,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// A printable character produced by the platform's text input.
    Char(char),
}

/// Modifier state at the time of the event.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Mods {
    pub shift: bool,
    /// Ctrl on Windows/Linux, Command on macOS — whichever is the local
    /// "primary" modifier, so shortcuts do the expected thing per platform.
    pub cmd: bool,
    /// The literal Control key, on every platform.
    ///
    /// Separate from `cmd` because some conventions mean Control specifically
    /// and not "the primary modifier" — vim's Ctrl-r is Ctrl-r on a Mac too.
    pub ctrl: bool,
    pub alt: bool,
}

/// Everything a frame needs to know about the pointer and keyboard.
///
/// Mouse coordinates are already in *virtual* pixels: the backend has undone
/// the integer upscale and the letterbox offset, so widgets never see a
/// physical pixel or a DPI factor.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub mouse: Point,
    pub mouse_down: bool,
    /// True only on the frame the button went down.
    pub mouse_pressed: bool,
    /// True only on the frame the button came up.
    pub mouse_released: bool,
    /// Whether the pointer is inside the window at all.
    pub mouse_in_window: bool,
    /// Accumulated scroll for this frame, in notches.
    pub wheel: f32,
    /// Keys that went down this frame, in order.
    pub keys: Vec<Key>,
    pub mods: Mods,
    /// Seconds since the previous frame, already clamped to something sane.
    pub dt: f32,
    /// Seconds since the app started.
    pub time: f32,
}

impl Input {
    pub fn key_pressed(&self, key: Key) -> bool {
        self.keys.contains(&key)
    }

    /// Clear the per-frame edges, keeping the level state (`mouse_down`, `mods`).
    pub fn begin_frame(&mut self) {
        self.mouse_pressed = false;
        self.mouse_released = false;
        self.wheel = 0.0;
        self.keys.clear();
    }
}

/// A cursor shape a widget can ask for. The backend applies whatever the last
/// widget requested this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cursor {
    #[default]
    Default,
    /// Over something clickable.
    Pointer,
    /// Dragging along an axis.
    Grab,
    Text,
}
