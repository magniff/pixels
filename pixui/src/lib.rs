//! # pixui
//!
//! A chunky, warm, pixel-art immediate-mode UI toolkit that renders entirely on
//! the CPU.
//!
//! ## Why software rendering is the right call here
//!
//! A conventional UI toolkit spends most of its complexity budget on
//! resolution independence: antialiasing, glyph hinting, subpixel positioning,
//! vector rasterisation. pixui opts out of all of it. Everything is drawn into
//! a small fixed-size buffer — a few hundred pixels on a side, well under 150k
//! pixels — and then blown up by a whole-number factor at present time.
//!
//! At that size the entire frame is a rounding error for a modern CPU, which is
//! why the renderer is one readable file with no shaders in it. The pixel-art
//! look is not a costume bolted onto a normal toolkit; it is what makes the
//! toolkit small.
//!
//! ## Layering
//!
//! ```text
//!   your application          ← only ever sees the modules below
//!   ─────────────────────────────────────────────────────────────
//!   widgets                   button, toggle, slider, panel, ...
//!   ui + layout + theme       identity, hit testing, focus, look
//!   canvas + font + color     the software rasteriser
//!   ─────────────────────────────────────────────────────────────
//!   app                       ← the only module that names a platform crate
//! ```
//!
//! [`app`] is the single place winit and softbuffer appear. Everything above it
//! speaks [`input::Input`] and [`canvas::Canvas`], so the backend could be
//! replaced — GPU, framebuffer, embedded, a test harness that renders to a PNG —
//! without touching a widget.
//!
//! ## Getting started
//!
//! ```no_run
//! use pixui::{Config, Theme, Tone, Ui};
//!
//! struct State { clicks: u32, loud: bool }
//!
//! pixui::run(
//!     Config::new("hello", 240, 140).with_scale(3.0),
//!     State { clicks: 0, loud: false },
//!     |ui: &mut Ui, state: &mut State| {
//!         let screen = ui.canvas.bounds().inset(8);
//!         let inner = ui.panel(screen, "HELLO");
//!         ui.column(inner, 4, |ui| {
//!             ui.label(&format!("clicks: {}", state.clicks));
//!             if ui.button_tone("PRESS ME", Tone::Accent).clicked {
//!                 state.clicks += 1;
//!             }
//!             ui.toggle("loud", &mut state.loud);
//!         });
//!     },
//! ).unwrap();
//! ```
//!
//! ## What this deliberately does not do
//!
//! Worth knowing before you build on it:
//!
//! - **ASCII only.** The built-in bitmap font covers printable ASCII. Scripts
//!   that need shaping, and IME composition, are out of scope.
//! - **No accessibility tree.** To a screen reader this is an opaque rectangle.
//!   A real product built on this would need to publish an `accesskit` tree
//!   alongside the pixels.
//! - **One window, no OS chrome.** No menus, no dialogs, no multi-window.

#![forbid(unsafe_code)]

pub mod anim;
pub mod app;
pub mod canvas;
pub mod clipboard;
pub mod color;
pub mod cursor;
pub mod font;
pub mod geom;
pub mod icon;
pub mod input;
pub mod layout;
pub mod theme;
pub mod ui;
pub mod widgets;

pub use anim::{smooth, Spring, WidgetAnim};
pub use app::{resolve_geometry, run, zoom_action, Config, Geometry, Scaling, ZoomAction};
pub use canvas::Canvas;
pub use color::{palette, Color};
pub use geom::{Point, Rect};
pub use input::{Cursor, Input, Key, Mods};
pub use layout::{Align, Dir, Layout};
pub use theme::{scheme_named, Metrics, Named, Ramp, Scheme, Syntax, Theme, Tone, SCHEMES};
pub use ui::{FrameOutput, Id, Response, ScrollState, TextState, Ui, UiState};
pub use widgets::{Floating, Segment};
