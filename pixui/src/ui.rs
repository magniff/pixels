//! The immediate-mode core: identity, hit testing, focus, and the per-frame
//! context that widgets are written against.
//!
//! Immediate mode is the right fit here for a specific reason. This toolkit
//! redraws continuously anyway — press springs and hover blends need a frame
//! every 16ms regardless — so the usual argument for retaining a widget tree
//! (avoiding redundant repaints) simply does not apply. What is left is the
//! part immediate mode is good at: your application state *is* the UI state,
//! with nothing to keep in sync.

use std::collections::HashMap;

use crate::anim::{smooth, WidgetAnim};
use crate::canvas::Canvas;
use crate::color::Color;
use crate::font;
use crate::geom::{Point, Rect};
use crate::input::{Cursor, Input, Key};
use crate::layout::{Align, Dir, Layout};
use crate::theme::Theme;

/// A widget's identity, hashed from its label and its position in the tree.
pub type Id = u64;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;

fn fnv(seed: Id, bytes: &[u8]) -> Id {
    let mut h = seed ^ FNV_OFFSET;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// What happened to a widget this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Response {
    pub id: Id,
    pub rect: Rect,
    /// Pointer is over it and nothing else has captured the pointer.
    pub hovered: bool,
    /// Pointer went down on it and has not come up yet.
    pub held: bool,
    /// A full press-and-release landed on it this frame, or it was activated
    /// from the keyboard.
    pub clicked: bool,
    /// A second click landed on the same widget, close by and soon after.
    ///
    /// `clicked` is also true when this is: a double click is a click that
    /// happens to be the second, and a caller that only cares about the first
    /// meaning should not have to handle both.
    pub double_clicked: bool,
    pub focused: bool,
    /// A value widget wrote a new value this frame.
    pub changed: bool,
    /// The other mouse button went down on it: the gesture that asks what can
    /// be done with a thing, rather than doing anything to it.
    pub right_clicked: bool,
}

/// Persistent state for one scrollable region.
///
/// `target` is where the content has been asked to go; `shown` is where it is
/// actually drawn, eased towards the target. Keeping them apart is what makes a
/// wheel notch glide instead of teleport — and drawing is always rounded to a
/// whole pixel, so the glide never blurs the grid.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScrollState {
    /// Requested offset from the top of the content, in pixels.
    pub target: f32,
    /// Eased offset actually used for drawing.
    pub shown: f32,
    /// Content extent measured on the previous frame.
    pub content: i32,
    /// Viewport extent on the previous frame.
    pub viewport: i32,
    /// Distance from the thumb's top edge to the pointer when a drag began.
    pub grab: i32,
    /// The part of a wheel gesture too small to be a whole row yet.
    frac: f32,
    pub(crate) touched: u64,
}

impl ScrollState {
    /// The largest meaningful offset: 0 when the content fits.
    pub fn max_offset(&self) -> f32 {
        (self.content - self.viewport).max(0) as f32
    }

    /// Whether there is anything to scroll.
    pub fn scrollable(&self) -> bool {
        self.content > self.viewport
    }

    /// Whole rows this frame's wheel comes to, keeping what is left over.
    ///
    /// A view that scrolls by whole rows — a text editor, where half a line is
    /// not a place the view can be — still has to answer a pointer that reports
    /// fractions of one. A trackpad sends a few points at a time; round each
    /// frame's fraction on its own and a gentle push rounds to nothing at all,
    /// again and again, until one frame rounds to a whole row and the view
    /// jumps. Which is exactly how it feels: it resists, then it lurches.
    ///
    /// So the remainder is kept rather than dropped, and spends itself on the
    /// next frame. `per_notch` is how many rows one notch of a wheel means.
    pub fn wheel_rows(&mut self, wheel: f32, per_notch: f32) -> i32 {
        self.frac += wheel * per_notch;
        let whole = self.frac.trunc();
        self.frac -= whole;
        whole as i32
    }
}

/// Caret state for one text field.
#[derive(Clone, Copy, Debug, Default)]
pub struct TextState {
    /// Caret position, counted in characters rather than bytes.
    pub caret: usize,
    /// Horizontal scroll, for text wider than its field.
    pub scroll: i32,
    pub(crate) touched: u64,
}

/// State that has to outlive a single frame.
///
/// In immediate mode this is the *only* retained thing: which widget is hot,
/// which captured the pointer, which has focus, and a small animation table
/// keyed by widget id.
#[derive(Default)]
pub struct UiState {
    hot: Option<Id>,
    active: Option<Id>,
    focus: Option<Id>,
    anims: HashMap<Id, WidgetAnim>,
    scrolls: HashMap<Id, ScrollState>,
    texts: HashMap<Id, TextState>,
    /// The text field that last held focus, so an application can tell whether
    /// typing belongs to a field or to its own key handling.
    text_focus: Option<Id>,
    /// The last click: which widget, when, and where. Enough to recognise the
    /// next one as a double.
    last_click: Option<(Id, f32, Point)>,
    focus_order: Vec<Id>,
    /// Floating layers opened this frame, and the ones from the frame before.
    ///
    /// Interaction is resolved as each widget is visited, so a widget cannot
    /// know what will be drawn on top of it later in the same frame. It can
    /// know what *was* on top of it last frame, which for anything that stays
    /// on screen long enough to be clicked is the same answer.
    layers: Vec<(u32, Rect)>,
    layers_before: Vec<(u32, Rect)>,
    frame: u64,
    /// Whether to keep a note of where each named widget ended up.
    ///
    /// Off, and costing a branch, unless something asks. What asks is a test:
    /// driving the real application from a synthetic [`crate::Input`] works
    /// for anything with a key, and falls down on anything that has to be
    /// clicked, because a test that knows a button is at (1312, 84) is a test
    /// that breaks when the layout moves by a pixel. With this on it can ask
    /// where "ACCEPT" is and click the middle of it, which is what a person
    /// does.
    indexing: bool,
    named: HashMap<Id, String>,
    placed: HashMap<Id, Rect>,
}

impl UiState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focused(&self) -> Option<Id> {
        self.focus
    }

    pub fn clear_focus(&mut self) {
        self.focus = None;
    }

    /// Keep a note of where each named widget is drawn, for a harness that
    /// wants to click one by name. See [`Self::find`].
    pub fn index(&mut self, on: bool) {
        self.indexing = on;
        if !on {
            self.named.clear();
            self.placed.clear();
        }
    }

    /// Where the widget with this name was on the last frame drawn.
    ///
    /// The name is the one the application passed - a button's label, a
    /// field's name. `None` for a widget that was not drawn, which is itself
    /// worth asserting on: a panel that should have closed, a button that
    /// should not be offered.
    pub fn find(&self, name: &str) -> Option<Rect> {
        self.named
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .and_then(|(id, _)| self.placed.get(id))
            .copied()
    }

    /// Every name drawn on the last frame, for saying what *was* there when
    /// something expected is not.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .named
            .iter()
            .filter(|(id, _)| self.placed.contains_key(id))
            .map(|(_, n)| n.clone())
            .collect();
        out.sort();
        out
    }
}

/// What the frame wants the backend to do once drawing is finished.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameOutput {
    pub cursor: Cursor,
    /// Set if the frame asked to be re-skinned; applied from the next frame on.
    pub theme: Option<Theme>,
    /// Set if the frame asked the application to close.
    pub quit: bool,
    /// Set if the frame asked for a different pixel scale.
    pub pixel_scale: Option<i32>,
    /// Whether anything on this frame was still moving.
    ///
    /// When nothing is, the next frame would paint exactly the same pixels, so
    /// the event loop stops drawing and waits for something to happen. An
    /// application that sits still should cost nothing to sit still.
    pub animating: bool,
    /// Whether something else needs the GPU more than the window does.
    pub sharing_gpu: bool,
}

/// The per-frame UI context handed to application code.
pub struct Ui<'a> {
    /// Direct access to the rasteriser, for custom drawing alongside widgets.
    pub canvas: &'a mut Canvas,
    pub input: &'a Input,
    pub theme: &'a Theme,
    state: &'a mut UiState,
    layouts: Vec<Layout>,
    id_stack: Vec<Id>,
    label_counts: HashMap<Id, u32>,
    cursor: Cursor,
    next_theme: Option<Theme>,
    input_blocked: bool,
    /// Set by anything that will look different next frame.
    animating: bool,
    sharing_gpu: bool,
    /// How deep in floating layers the drawing currently is. Zero is the page.
    layer_depth: u32,
    keyboard_captured: bool,
    quit: bool,
    next_pixel_scale: Option<i32>,
}

impl<'a> Ui<'a> {
    /// Begin a frame. The backend calls this; application code receives the
    /// result already built.
    pub fn begin(
        canvas: &'a mut Canvas,
        input: &'a Input,
        theme: &'a Theme,
        state: &'a mut UiState,
    ) -> Self {
        state.frame = state.frame.wrapping_add(1);
        state.hot = None;
        state.focus_order.clear();

        let root = Layout::new(canvas.bounds(), Dir::Vertical, theme.metrics.gap);
        if state.indexing {
            state.placed.clear();
            state.named.clear();
        }
        Self {
            canvas,
            input,
            theme,
            state,
            layouts: vec![root],
            id_stack: vec![0],
            label_counts: HashMap::new(),
            cursor: Cursor::Default,
            next_theme: None,
            input_blocked: false,
            animating: false,
            sharing_gpu: false,
            layer_depth: 0,
            keyboard_captured: false,
            quit: false,
            next_pixel_scale: None,
        }
    }

    /// End a frame: resolve keyboard focus movement, release a stuck capture,
    /// and drop animation entries for widgets that no longer exist.
    pub fn finish(self) -> FrameOutput {
        let Ui { state, input, .. } = self;

        if input.mouse_released {
            state.active = None;
        }

        // A widget that has taken the keyboard (a text editor, say) means Tab
        // and Escape for itself, so the global focus handling stands down.
        if !self.keyboard_captured && input.key_pressed(Key::Escape) {
            state.focus = None;
        }

        if !self.keyboard_captured && input.key_pressed(Key::Tab) && !state.focus_order.is_empty() {
            let order = &state.focus_order;
            let step: i32 = if input.mods.shift { -1 } else { 1 };
            let next = match state.focus.and_then(|f| order.iter().position(|&x| x == f)) {
                Some(i) => {
                    let n = order.len() as i32;
                    order[(i as i32 + step).rem_euclid(n) as usize]
                }
                None if step > 0 => order[0],
                None => order[order.len() - 1],
            };
            state.focus = Some(next);
        }

        // This frame's layers become the ones the next frame resolves against.
        state.layers_before = std::mem::take(&mut state.layers);

        // Widgets that were not drawn this frame will never be drawn again
        // without being re-created, so their animation state is dead weight.
        let frame = state.frame;
        state.anims.retain(|_, a| a.touched == frame);
        state.scrolls.retain(|_, s| s.touched == frame);
        state.texts.retain(|_, t| t.touched == frame);

        // A field that was not drawn is gone, and the keyboard cannot be left
        // with it. Nothing else would ever hand it back: the application asks
        // whether a text field has the keys before dispatching its own, so a
        // focus pointing at a field that no longer exists swallows every key
        // until something else is clicked. Which is exactly how it feels — the
        // window stops answering and a click anywhere fixes it.
        if let Some(id) = state.text_focus.filter(|id| !state.texts.contains_key(id)) {
            state.text_focus = None;
            if state.focus == Some(id) {
                state.focus = None;
            }
        }

        let out = FrameOutput {
            cursor: self.cursor,
            theme: self.next_theme,
            animating: self.animating,
            sharing_gpu: self.sharing_gpu,
            quit: self.quit,
            pixel_scale: self.next_pixel_scale,
        };

        // ---- post-frame passes -------------------------------------------
        // These belong to the toolkit, not to whoever is driving it. Leaving
        // them to the caller meant every driver — the backend, a snapshot
        // harness — had to reimplement them and could disagree about the order.
        let theme = self.theme;
        if theme.scanline > 0.0 {
            let bounds = self.canvas.bounds();
            self.canvas.scanlines(bounds, theme.scanline);
        }
        // Last of all, so nothing is ever drawn over the pointer, and after the
        // scanlines so it is not striped.
        if input.draw_pointer && input.mouse_in_window {
            crate::cursor::draw(
                self.canvas,
                input.mouse,
                out.cursor,
                theme.cursor_fill,
                theme.cursor_outline,
            );
        }

        out
    }

    // -------------------------------------------------------------- identity

    fn current_parent(&self) -> Id {
        *self.id_stack.last().unwrap_or(&0)
    }

    /// Derive a stable id for a widget from its label.
    ///
    /// Two widgets with the same label under the same parent would collide, so
    /// the number of times a label has already been used this frame is mixed
    /// in. That keeps ids stable frame-to-frame as long as widget order is.
    pub fn id(&mut self, label: &str) -> Id {
        let base = fnv(self.current_parent(), label.as_bytes());
        let count = self.label_counts.entry(base).or_insert(0);
        let id = if *count == 0 {
            base
        } else {
            fnv(base, &count.to_le_bytes())
        };
        *count += 1;
        if self.state.indexing {
            self.state.named.insert(id, label.to_string());
        }
        id
    }

    /// Run `f` under an extra identity scope, so repeated labels inside it do
    /// not collide with the same labels outside.
    pub fn scope<R>(&mut self, name: &str, f: impl FnOnce(&mut Ui) -> R) -> R {
        let id = fnv(self.current_parent(), name.as_bytes());
        self.id_stack.push(id);
        let r = f(self);
        self.id_stack.pop();
        r
    }

    // ---------------------------------------------------------------- layout

    fn layout_mut(&mut self) -> &mut Layout {
        self.layouts
            .last_mut()
            .expect("layout stack is never empty")
    }

    /// Take `main` pixels from the current layout.
    pub fn alloc(&mut self, main: i32) -> Rect {
        self.layout_mut().alloc(main)
    }

    /// Take a fixed-size box from the current layout.
    pub fn alloc_sized(&mut self, w: i32, h: i32) -> Rect {
        self.layout_mut().alloc_sized(w, h)
    }

    /// Take everything left in the current layout.
    pub fn alloc_rest(&mut self) -> Rect {
        self.layout_mut().alloc_rest()
    }

    /// Change how much room the current layout leaves between what it hands
    /// out from here on.
    ///
    /// The default is the theme's, which suits controls. A list of names is not
    /// a stack of controls: the gap that keeps two buttons from reading as one
    /// is, in a list, the gap that stops it reading as a list.
    pub fn set_spacing(&mut self, gap: i32) {
        self.layout_mut().spacing = gap;
    }

    /// Advance the cursor without drawing.
    pub fn space(&mut self, main: i32) {
        self.layout_mut().skip(main);
    }

    /// The unconsumed part of the current layout.
    pub fn remaining(&self) -> Rect {
        self.layouts
            .last()
            .map(|l| l.remaining())
            .unwrap_or(Rect::ZERO)
    }

    /// Stack widgets vertically inside `bounds`.
    pub fn column<R>(&mut self, bounds: Rect, spacing: i32, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.layouts
            .push(Layout::new(bounds, Dir::Vertical, spacing));
        let r = f(self);
        self.layouts.pop();
        r
    }

    /// As [`Ui::column`], but also reports how many pixels the content
    /// consumed along the layout axis.
    ///
    /// This is how a scroll area learns its content height in immediate mode:
    /// there is no tree to measure, so you lay the content out and see how far
    /// the cursor got. The answer is one frame stale, which is invisible.
    pub fn column_measured<R>(
        &mut self,
        bounds: Rect,
        spacing: i32,
        f: impl FnOnce(&mut Ui) -> R,
    ) -> (R, i32) {
        self.layouts
            .push(Layout::new(bounds, Dir::Vertical, spacing));
        let r = f(self);
        let used = self.layouts.last().map(|l| l.used()).unwrap_or(0);
        self.layouts.pop();
        (r, used)
    }

    /// Lay widgets out left-to-right inside `bounds`.
    pub fn row<R>(&mut self, bounds: Rect, spacing: i32, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.layouts
            .push(Layout::new(bounds, Dir::Horizontal, spacing));
        let r = f(self);
        self.layouts.pop();
        r
    }

    /// Take `h` pixels from the current (vertical) layout and fill them with a row.
    pub fn row_h<R>(&mut self, h: i32, spacing: i32, f: impl FnOnce(&mut Ui) -> R) -> R {
        let bounds = self.alloc(h);
        self.row(bounds, spacing, f)
    }

    /// Take `w` pixels from the current (horizontal) layout and fill them with a column.
    pub fn column_w<R>(&mut self, w: i32, spacing: i32, f: impl FnOnce(&mut Ui) -> R) -> R {
        let bounds = self.alloc(w);
        self.column(bounds, spacing, f)
    }

    /// Restrict drawing to `rect` for the duration of `f`.
    pub fn clipped<R>(&mut self, rect: Rect, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.canvas.push_clip(rect);
        let r = f(self);
        self.canvas.pop_clip();
        r
    }

    // ------------------------------------------------------- input gating

    /// Run `f` with pointer and keyboard interaction suppressed if `blocked`.
    ///
    /// This is how a modal works in immediate mode. The dialog is drawn *after*
    /// the content beneath it, but interaction is resolved as each widget is
    /// visited — so by the time the dialog exists, the widgets underneath have
    /// already had their say. Wrapping the background in this call is what
    /// stops a click landing on a button hidden behind a dialog.
    pub fn input_blocked<R>(&mut self, blocked: bool, f: impl FnOnce(&mut Ui) -> R) -> R {
        let prev = self.input_blocked;
        self.input_blocked = prev || blocked;
        let r = f(self);
        self.input_blocked = prev;
        r
    }

    /// Ask for another frame after this one.
    ///
    /// Anything that changes with time rather than with input has to say so: a
    /// caret that blinks, a spinner, a progress bar reading a file that is
    /// still arriving. Widgets that animate through [`Self::animate`] and the
    /// springs are covered already.
    pub fn request_repaint(&mut self) {
        self.animating = true;
    }

    /// Say that something else on this machine is using the GPU heavily, and
    /// that the window should keep out of its way until it stops.
    ///
    /// For an application that computes on the GPU as well as drawing on it -
    /// a model answering a question, a video encoding, a simulation stepping.
    /// The two queue behind each other, and since a command buffer runs to
    /// completion, a frame that arrives behind a long computation waits for
    /// all of it. Measured with a language model reading a question on a
    /// worker thread: 50-87 frames a second with 200ms gaps, against 111-119
    /// with a worst frame of 9.7ms once the window stopped using the GPU.
    ///
    /// Presenting on the CPU costs more of it - 11ms a frame against 1.5ms at
    /// 3024x1724 - so this is for the stretch where it is true and not for the
    /// whole run. Called every frame it applies to, like
    /// [`Self::request_repaint`]. Where the toolkit has only one presenter
    /// compiled in, it does nothing.
    pub fn share_gpu(&mut self) {
        self.sharing_gpu = true;
    }

    /// Whether interaction is currently suppressed.
    pub fn is_input_blocked(&self) -> bool {
        self.input_blocked
    }

    /// Draw `f` as a floating layer covering `rect`.
    ///
    /// Everything inside takes the pointer ahead of everything at a shallower
    /// depth, whatever order it is drawn in. That is the piece immediate mode
    /// does not give you for free: a panel drawn last is *painted* on top, but
    /// by the time it is drawn the widgets underneath have already been asked
    /// whether they were clicked, and one of them will have said yes.
    ///
    /// The layer's extent is declared rather than discovered, so a click on a
    /// bare patch of the panel is swallowed by the panel instead of falling
    /// through to whatever is behind it.
    ///
    /// This does not reorder painting. A layer still has to be drawn after what
    /// it covers, which is the easy half and the one the caller controls.
    pub fn layer<R>(&mut self, rect: Rect, f: impl FnOnce(&mut Ui) -> R) -> R {
        self.layer_depth += 1;
        self.state.layers.push((self.layer_depth, rect));
        // Out from under whatever was clipping. A layer floats above the
        // interface rather than inside a part of it, and one opened over a
        // narrow pane was being cut off at that pane's edge - which for a menu
        // meant its answers were missing the ends of their words.
        self.canvas.push_no_clip();
        let r = f(self);
        self.canvas.pop_clip();
        self.layer_depth -= 1;
        r
    }

    /// Whether something floating above this point in the stack has the pointer.
    ///
    /// Answered from the previous frame's layers: see [`UiState::layers`].
    pub fn pointer_covered(&self) -> bool {
        let p = self.input.mouse;
        self.state
            .layers_before
            .iter()
            .any(|(depth, rect)| *depth > self.layer_depth && rect.contains(p))
    }

    /// Declare that a widget is handling the keyboard itself this frame, so the
    /// built-in Tab and Escape focus handling should stay out of the way.
    pub fn capture_keyboard(&mut self) {
        self.keyboard_captured = true;
    }

    pub fn is_keyboard_captured(&self) -> bool {
        self.keyboard_captured
    }

    // ------------------------------------------------------------ interaction

    /// Hit-test `rect` against the pointer and update capture state.
    ///
    /// Capture matters: once a widget owns the pointer, dragging outside its
    /// bounds keeps feeding it events, and no other widget lights up. That is
    /// what makes a slider survive a sloppy drag.
    pub fn interact(&mut self, id: Id, rect: Rect) -> Response {
        if self.state.indexing {
            self.state.placed.insert(id, rect);
        }
        let p: Point = self.input.mouse;
        // Blocked input means blocked: a widget drawn under a modal must not
        // take the pointer, and until this said so, `input_blocked` only ever
        // stopped a field from typing. A dialog's own buttons then went dead
        // wherever they overlapped something underneath, because the thing
        // underneath was asked first and said yes.
        let inside = !self.input_blocked
            && self.input.mouse_in_window
            && rect.contains(p)
            && self.canvas.clip_contains(p)
            && !self.pointer_covered();

        let mut resp = Response {
            id,
            rect,
            ..Default::default()
        };

        if self.state.active == Some(id) {
            resp.held = true;
            resp.hovered = inside;
            self.state.hot = Some(id);
            if self.input.mouse_released {
                resp.clicked = inside;
                self.state.active = None;
                if resp.clicked {
                    resp.double_clicked = self.register_click(id, p);
                }
            }
        } else if self.state.active.is_none() && inside {
            resp.hovered = true;
            self.state.hot = Some(id);
            if self.input.mouse_pressed {
                self.state.active = Some(id);
                self.state.focus = Some(id);
                resp.held = true;
            }
        }

        // Not through the capture dance the left button goes through: nothing
        // is dragged with this, so there is nothing to hold, and taking capture
        // would only mean a menu could not be opened over something already
        // being pressed.
        resp.right_clicked = inside && self.input.right_pressed;
        resp.focused = self.state.focus == Some(id);
        resp
    }

    /// Record a click and report whether it completes a double.
    ///
    /// A double is reset once recognised, so three clicks are a double followed
    /// by a single rather than two overlapping doubles.
    fn register_click(&mut self, id: Id, at: Point) -> bool {
        const INTERVAL: f32 = 0.40;
        const SLOP: i32 = 3;
        let now = self.input.time;
        let double = self.state.last_click.is_some_and(|(prev, when, where_)| {
            prev == id
                && now - when < INTERVAL
                && (where_.x - at.x).abs() <= SLOP
                && (where_.y - at.y).abs() <= SLOP
        });
        self.state.last_click = if double { None } else { Some((id, now, at)) };
        double
    }

    /// Register `id` in this frame's tab order and report keyboard activation.
    pub fn focusable(&mut self, id: Id) -> bool {
        if self.input_blocked {
            return false;
        }
        self.state.focus_order.push(id);
        self.state.focus == Some(id)
            && (self.input.key_pressed(Key::Enter) || self.input.key_pressed(Key::Space))
    }

    /// The stored animation for a widget, for tests and diagnostics.
    pub fn anim_of(&self, id: Id) -> WidgetAnim {
        self.state.anims.get(&id).copied().unwrap_or_default()
    }

    pub fn is_active(&self, id: Id) -> bool {
        self.state.active == Some(id)
    }

    pub fn is_hot(&self, id: Id) -> bool {
        self.state.hot == Some(id)
    }

    pub fn set_focus(&mut self, id: Id) {
        self.state.focus = Some(id);
    }

    /// Drop keyboard focus, so an application with its own key handling gets
    /// the keys back from whatever field was holding them.
    pub fn clear_focus(&mut self) {
        self.state.focus = None;
    }

    /// Swap the theme from the next frame onwards.
    ///
    /// It cannot take effect mid-frame — half the widgets are already drawn —
    /// so the backend applies it once this frame is presented.
    pub fn request_theme(&mut self, theme: Theme) {
        self.next_theme = Some(theme);
    }

    /// How many physical pixels one virtual pixel currently occupies.
    pub fn pixel_scale(&self) -> i32 {
        self.input.pixel_scale
    }

    /// Ask for a different pixel scale, applied from the next frame.
    ///
    /// Under [`crate::Scaling::Adaptive`] this is the zoom control: a smaller
    /// scale means smaller chrome and more room, a larger one the reverse. Only
    /// whole numbers are possible, so the steps are coarse by construction —
    /// from 2 the next stop up is 3, which is 50% larger, not 30%. The backend
    /// clamps to the configured range.
    pub fn request_pixel_scale(&mut self, scale: i32) {
        self.next_pixel_scale = Some(scale);
    }

    /// Ask the application to close after this frame.
    pub fn request_quit(&mut self) {
        self.quit = true;
    }

    /// Ask the backend for a cursor shape. The strongest request wins, so a
    /// pointer over a button is not undone by a later default request.
    pub fn request_cursor(&mut self, c: Cursor) {
        if c != Cursor::Default {
            self.cursor = c;
        }
    }

    // ----------------------------------------------------------------- scroll

    /// This region's scroll state, or a fresh one if it is new this frame.
    pub fn scroll_state(&self, id: Id) -> ScrollState {
        self.state.scrolls.get(&id).copied().unwrap_or_default()
    }

    /// Store a region's scroll state and mark it live for this frame.
    pub fn set_scroll_state(&mut self, id: Id, mut scroll: ScrollState) {
        scroll.touched = self.state.frame;
        self.state.scrolls.insert(id, scroll);
    }

    /// Whether a text field currently owns the keyboard.
    ///
    /// Read this *before* dispatching keys to your own handling: an application
    /// with its own key bindings — a modal editor, a game — has to know when
    /// the user is typing into a field instead. The answer is carried over from
    /// the previous frame, which is exact, because focus only ever changes on a
    /// click or a Tab, and neither of those is a keystroke you would lose.
    pub fn text_input_active(&self) -> bool {
        self.state
            .text_focus
            .is_some_and(|id| self.state.focus == Some(id))
    }

    /// Record that a text field holds the keyboard this frame.
    pub(crate) fn set_text_focus(&mut self, id: Id) {
        self.state.text_focus = Some(id);
    }

    /// Caret state for a text field, or a fresh one if it is new this frame.
    pub fn text_state(&self, id: Id) -> TextState {
        self.state.texts.get(&id).copied().unwrap_or_default()
    }

    /// Store a text field's caret state and mark it live for this frame.
    pub fn set_text_state(&mut self, id: Id, mut text: TextState) {
        text.touched = self.state.frame;
        self.state.texts.insert(id, text);
    }

    // -------------------------------------------------------------- animation

    /// Whether this widget already has animation state, i.e. it existed last frame.
    pub fn anim_exists(&self, id: Id) -> bool {
        self.state.anims.contains_key(&id)
    }

    /// Advance and return this widget's animation state.
    pub fn animate(&mut self, id: Id, resp: &Response) -> WidgetAnim {
        let dt = self.input.dt;
        let mut a = self.state.anims.get(&id).copied().unwrap_or_default();
        a.touched = self.state.frame;
        a.press.step(if resp.held { 1.0 } else { 0.0 }, dt);
        a.hover = smooth(a.hover, if resp.hovered { 1.0 } else { 0.0 }, 24.0, dt);
        a.focus = smooth(a.focus, if resp.focused { 1.0 } else { 0.0 }, 20.0, dt);
        a.flash = smooth(a.flash, 0.0, 11.0, dt);
        if resp.clicked {
            a.flash = 1.0;
        }
        // Hover and focus blends settle exponentially and never quite arrive,
        // so they are compared against their target rather than their velocity.
        let blending = (a.hover - f32::from(u8::from(resp.hovered))).abs() > 0.002
            || (a.focus - f32::from(u8::from(resp.focused))).abs() > 0.002;
        if a.moving() || blending {
            self.animating = true;
        }
        self.state.anims.insert(id, a);
        a
    }

    /// Read-modify-write access to a widget's stored animation, for widgets
    /// that drive `value` themselves (toggles, sliders).
    pub fn with_anim<R>(&mut self, id: Id, f: impl FnOnce(&mut WidgetAnim) -> R) -> R {
        let mut a = self.state.anims.get(&id).copied().unwrap_or_default();
        a.touched = self.state.frame;
        let r = f(&mut a);
        if a.moving() {
            self.animating = true;
        }
        self.state.anims.insert(id, a);
        r
    }

    // ------------------------------------------------------------------ text

    /// Draw `text` inside `rect`, vertically centred and horizontally aligned.
    pub fn draw_text_in(&mut self, rect: Rect, text: &str, color: Color, align: Align) {
        let w = font::text_width(text);
        let x = match align {
            Align::Left => rect.x,
            Align::Center => rect.x + (rect.w - w) / 2,
            Align::Right => rect.right() - w,
        };
        let y = rect.y + (rect.h - font::text_height(text)) / 2;
        font::draw_text(self.canvas, x, y, text, color);
    }

    /// As [`Ui::draw_text_in`], but with a one-pixel drop shadow.
    pub fn draw_text_in_shadow(
        &mut self,
        rect: Rect,
        text: &str,
        color: Color,
        shadow: Color,
        align: Align,
    ) {
        let w = font::text_width(text);
        let x = match align {
            Align::Left => rect.x,
            Align::Center => rect.x + (rect.w - w) / 2,
            Align::Right => rect.right() - w,
        };
        let y = rect.y + (rect.h - font::text_height(text)) / 2;
        font::draw_text_shadow(self.canvas, x, y, text, color, shadow);
    }
}
