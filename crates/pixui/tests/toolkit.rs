//! Tests for the parts that are easy to get subtly wrong: layout arithmetic,
//! widget identity, pointer capture, and the integer upscale.
//!
//! Everything here goes through the public API, which doubles as a check that
//! the toolkit is actually usable from outside the crate.

use pixui::app::blit;
use pixui::layout::Dir;
use pixui::{palette, Canvas, Color, Input, Layout, Point, Rect, ScrollState, Theme, Ui, UiState};
use pixui::{resolve_geometry, Scaling};

// ------------------------------------------------------------------ geometry

#[test]
fn rect_splits_are_exact_and_lossless() {
    let r = Rect::new(10, 20, 100, 50);
    let (a, b) = r.split_left(30);
    assert_eq!(a, Rect::new(10, 20, 30, 50));
    assert_eq!(b, Rect::new(40, 20, 70, 50));
    assert_eq!(a.w + b.w, r.w, "a split must not lose or invent pixels");

    let (top, bottom) = r.split_top(15);
    assert_eq!(top.h + bottom.h, r.h);
    assert_eq!(bottom.y, top.bottom());
}

#[test]
fn rect_split_clamps_instead_of_going_negative() {
    let r = Rect::new(0, 0, 10, 10);
    let (a, b) = r.split_left(999);
    assert_eq!(a.w, 10);
    assert_eq!(b.w, 0);
}

#[test]
fn contains_is_half_open() {
    let r = Rect::new(0, 0, 4, 4);
    assert!(r.contains(Point::new(0, 0)));
    assert!(r.contains(Point::new(3, 3)));
    assert!(!r.contains(Point::new(4, 3)), "right edge is exclusive");
    assert!(!r.contains(Point::new(-1, 0)));
}

#[test]
fn intersect_of_disjoint_rects_is_empty_not_negative() {
    let a = Rect::new(0, 0, 10, 10);
    let b = Rect::new(50, 50, 10, 10);
    let i = a.intersect(b);
    assert!(i.is_empty());
    assert!(
        i.w >= 0 && i.h >= 0,
        "negative extents would corrupt fill loops"
    );
}

// -------------------------------------------------------------------- layout

#[test]
fn vertical_layout_puts_spacing_between_items_not_before_the_first() {
    let mut l = Layout::new(Rect::new(0, 0, 100, 100), Dir::Vertical, 4);
    let a = l.alloc(10);
    let b = l.alloc(10);
    assert_eq!(a.y, 0, "the first item sits flush against the top");
    assert_eq!(
        b.y, 14,
        "the second is offset by its own height plus spacing"
    );
    assert_eq!(a.w, 100, "a vertical layout spans the full cross axis");
}

#[test]
fn horizontal_layout_advances_along_x() {
    let mut l = Layout::new(Rect::new(5, 7, 100, 20), Dir::Horizontal, 3);
    let a = l.alloc(10);
    let b = l.alloc(10);
    assert_eq!((a.x, a.h), (5, 20));
    assert_eq!(b.x, 18);
}

#[test]
fn alloc_rest_consumes_exactly_what_is_left() {
    let mut l = Layout::new(Rect::new(0, 0, 100, 100), Dir::Vertical, 5);
    l.alloc(30);
    let rest = l.alloc_rest();
    assert_eq!(rest.y, 35);
    assert_eq!(
        rest.bottom(),
        100,
        "the remainder must reach the bottom edge exactly"
    );
}

// ---------------------------------------------------------------------- font

#[test]
fn text_width_measures_the_widest_line() {
    assert_eq!(pixui::font::text_width(""), 0);
    assert_eq!(pixui::font::text_width("A"), pixui::font::GLYPH_W);
    // Five glyphs: four full advances plus one glyph, with no trailing gap.
    assert_eq!(
        pixui::font::text_width("HELLO"),
        5 * pixui::font::ADVANCE - 1
    );
    assert_eq!(
        pixui::font::text_width("HI\nTHERE"),
        pixui::font::text_width("THERE")
    );
}

#[test]
fn unmapped_characters_fall_back_instead_of_panicking() {
    assert_eq!(pixui::font::glyph('\u{4e2d}'), pixui::font::glyph('?'));
    assert_eq!(pixui::font::glyph('\u{1}'), pixui::font::glyph('?'));
}

// --------------------------------------------------------------------- color

#[test]
fn lerp_hits_both_endpoints_and_clamps() {
    let a = Color::hex(0x000000);
    let b = Color::hex(0xFFFFFF);
    assert_eq!(a.lerp(b, 0.0), a);
    assert_eq!(a.lerp(b, 1.0), b);
    assert_eq!(
        a.lerp(b, 5.0),
        b,
        "t must clamp rather than overflow the channels"
    );
}

#[test]
fn shade_moves_towards_white_and_black() {
    let c = palette::ACCENT;
    assert!(c.shade(0.5).luma() > c.luma());
    assert!(c.shade(-0.5).luma() < c.luma());
}

// -------------------------------------------------------------------- canvas

#[test]
fn clip_rect_blocks_writes_outside_it() {
    let mut c = Canvas::new(16, 16);
    c.clear(Color::hex(0x000000));
    c.push_clip(Rect::new(4, 4, 4, 4));
    c.fill_rect(Rect::new(0, 0, 16, 16), Color::hex(0xFFFFFF));
    c.pop_clip();

    assert_eq!(
        c.get_px(5, 5),
        Color::hex(0xFFFFFF),
        "inside the clip is painted"
    );
    assert_eq!(
        c.get_px(0, 0),
        Color::hex(0x000000),
        "outside the clip is untouched"
    );
    assert_eq!(
        c.get_px(8, 8),
        Color::hex(0x000000),
        "the clip edge is exclusive"
    );
}

#[test]
fn chamfered_fill_is_left_right_symmetric() {
    // An asymmetric chamfer is immediately visible as a lopsided button, and
    // it is exactly the kind of off-by-one that survives a casual glance.
    let mut c = Canvas::new(20, 20);
    c.clear(Color::hex(0x000000));
    let r = Rect::new(2, 2, 16, 16);
    c.fill_chamfer(r, Color::hex(0xFFFFFF), 3);

    for y in r.y..r.bottom() {
        let left = (r.x..r.right()).find(|&x| c.get_px(x, y) != Color::hex(0x000000));
        let right = (r.x..r.right())
            .rev()
            .find(|&x| c.get_px(x, y) != Color::hex(0x000000));
        if let (Some(l), Some(rr)) = (left, right) {
            assert_eq!(l - r.x, r.right() - 1 - rr, "row {y} is lopsided");
        }
    }
}

// ---------------------------------------------------------------------- blit

#[test]
fn blit_maps_each_source_pixel_to_a_scale_by_scale_block() {
    let mut c = Canvas::new(2, 2);
    c.clear(Color::hex(0x000000));
    c.set_px(1, 0, Color::hex(0xFF0000));

    let (w, h, scale) = (8usize, 8usize, 3usize);
    let mut dst = vec![0u32; w * h];
    let mut scratch = Vec::new();
    // 2x2 at 3x is 6x6 inside an 8x8 window: one pixel of letterbox all round.
    blit(
        &c,
        &mut scratch,
        &mut dst,
        w,
        h,
        scale,
        (1, 1),
        Color::hex(0x00FF00),
    );

    let at = |x: usize, y: usize| dst[y * w + x];
    assert_eq!(at(0, 0), 0x00FF00, "top-left is letterbox");
    assert_eq!(at(7, 7), 0x00FF00, "bottom-right is letterbox");
    assert_eq!(at(1, 1), 0x000000, "source (0,0) lands at the offset");
    for dy in 0..scale {
        for dx in 0..scale {
            assert_eq!(
                at(4 + dx, 1 + dy),
                0xFF0000,
                "source (1,0) fills a 3x3 block"
            );
        }
    }
}

#[test]
fn blit_survives_a_window_smaller_than_the_canvas() {
    let c = Canvas::new(64, 64);
    let mut dst = vec![0u32; 8 * 8];
    let mut scratch = Vec::new();
    // Must not panic or write out of bounds when nothing fits.
    blit(&c, &mut scratch, &mut dst, 8, 8, 1, (0, 0), palette::VOID);
}

// ------------------------------------------------------------------- widgets

struct Harness {
    canvas: Canvas,
    theme: Theme,
    state: UiState,
    input: Input,
}

impl Harness {
    fn new() -> Self {
        Self {
            canvas: Canvas::new(200, 100),
            theme: Theme::warm(),
            state: UiState::new(),
            input: Input {
                mouse_in_window: true,
                dt: 1.0 / 60.0,
                ..Default::default()
            },
        }
    }

    fn frame<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> R {
        let mut ui = Ui::begin(&mut self.canvas, &self.input, &self.theme, &mut self.state);
        let r = f(&mut ui);
        ui.finish();
        self.input.begin_frame();
        r
    }
}

#[test]
fn identical_labels_in_one_container_get_distinct_ids() {
    let mut h = Harness::new();
    let (a, b) = h.frame(|ui| (ui.id("GO"), ui.id("GO")));
    assert_ne!(a, b, "two buttons both labelled GO must not share an id");
}

#[test]
fn ids_are_stable_across_frames() {
    let mut h = Harness::new();
    let first = h.frame(|ui| (ui.id("GO"), ui.id("GO")));
    let second = h.frame(|ui| (ui.id("GO"), ui.id("GO")));
    assert_eq!(
        first, second,
        "ids must not drift, or animation state resets every frame"
    );
}

#[test]
fn scopes_separate_otherwise_colliding_labels() {
    let mut h = Harness::new();
    let (a, b) = h.frame(|ui| {
        let a = ui.scope("left", |ui| ui.id("GO"));
        let b = ui.scope("right", |ui| ui.id("GO"));
        (a, b)
    });
    assert_ne!(a, b);
}

#[test]
fn a_click_needs_press_and_release_on_the_same_widget() {
    let mut h = Harness::new();
    let rect = Rect::new(10, 10, 60, 20);
    h.input.mouse = Point::new(20, 20);

    // Press: held, but not yet a click.
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    let r = h.frame(|ui| {
        let id = ui.id("BTN");
        ui.interact(id, rect)
    });
    assert!(r.held && !r.clicked, "a press alone is not a click");

    // Release over the widget: now it fires.
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let r = h.frame(|ui| {
        let id = ui.id("BTN");
        ui.interact(id, rect)
    });
    assert!(r.clicked, "press then release inside should click");
}

#[test]
fn releasing_outside_the_widget_cancels_the_click() {
    let mut h = Harness::new();
    let rect = Rect::new(10, 10, 60, 20);

    h.input.mouse = Point::new(20, 20);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| {
        let id = ui.id("BTN");
        ui.interact(id, rect)
    });

    // Drag away, then let go. This is how a user backs out of a mis-click.
    h.input.mouse = Point::new(150, 90);
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let r = h.frame(|ui| {
        let id = ui.id("BTN");
        ui.interact(id, rect)
    });
    assert!(!r.clicked, "releasing off the widget must not fire");
}

#[test]
fn pointer_capture_stops_other_widgets_lighting_up_mid_drag() {
    let mut h = Harness::new();
    let a = Rect::new(0, 0, 50, 20);
    let b = Rect::new(60, 0, 50, 20);

    h.input.mouse = Point::new(10, 10);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| {
        let id = ui.id("A");
        ui.interact(id, a);
        let id = ui.id("B");
        ui.interact(id, b);
    });

    // Still holding, but the pointer has wandered over the other widget.
    h.input.mouse = Point::new(70, 10);
    h.input.mouse_pressed = false;
    let (ra, rb) = h.frame(|ui| {
        let ida = ui.id("A");
        let ra = ui.interact(ida, a);
        let idb = ui.id("B");
        let rb = ui.interact(idb, b);
        (ra, rb)
    });
    assert!(ra.held, "the captured widget keeps receiving the drag");
    assert!(!rb.hovered, "the widget under the cursor must stay cold");
}

#[test]
fn toggle_flips_exactly_once_per_click() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut value = false;

    h.input.mouse = Point::new(10, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.toggle_at(rect, "SW", &mut value));
    assert!(!value, "the value flips on release, not on press");

    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let r = h.frame(|ui| ui.toggle_at(rect, "SW", &mut value));
    assert!(value && r.changed);

    // Holding still afterwards must not flip it again.
    h.input.mouse_released = false;
    h.frame(|ui| ui.toggle_at(rect, "SW", &mut value));
    assert!(value, "a settled toggle stays put");
}

#[test]
fn dragging_a_slider_writes_the_value() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 100, 15);
    let mut value = 0.0f32;

    h.input.mouse = Point::new(95, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    let r = h.frame(|ui| ui.slider_at(rect, "VOL", &mut value, 0.0, 1.0));
    assert!(r.changed);
    assert!(
        value > 0.8,
        "dragging to the right end should approach the maximum"
    );

    h.input.mouse = Point::new(-40, 7);
    h.input.mouse_pressed = false;
    h.frame(|ui| ui.slider_at(rect, "VOL", &mut value, 0.0, 1.0));
    assert_eq!(
        value, 0.0,
        "dragging past the left end clamps to the minimum"
    );
}

/// Register two focusable widgets and hand back their ids.
fn two_focusables(ui: &mut Ui) -> (pixui::Id, pixui::Id) {
    let a = ui.id("A");
    ui.focusable(a);
    let b = ui.id("B");
    ui.focusable(b);
    (a, b)
}

#[test]
fn tab_moves_focus_through_the_widgets_in_order() {
    let mut h = Harness::new();

    // The first frame only establishes the tab order.
    let (a, b) = h.frame(two_focusables);
    assert_eq!(h.state.focused(), None);

    // Tab is resolved in finish(), so it takes effect from this frame onward.
    h.input.keys.push(pixui::Key::Tab);
    h.frame(two_focusables);
    assert_eq!(
        h.state.focused(),
        Some(a),
        "the first Tab lands on the first widget"
    );

    h.input.keys.push(pixui::Key::Tab);
    h.frame(two_focusables);
    assert_eq!(h.state.focused(), Some(b), "the next Tab advances");

    h.input.keys.push(pixui::Key::Tab);
    h.input.mods.shift = true;
    h.frame(two_focusables);
    assert_eq!(h.state.focused(), Some(a), "Shift+Tab goes back");
}

#[test]
fn escape_drops_focus() {
    let mut h = Harness::new();
    let (a, _) = h.frame(two_focusables);
    h.input.keys.push(pixui::Key::Tab);
    h.frame(two_focusables);
    assert_eq!(h.state.focused(), Some(a));

    h.input.keys.push(pixui::Key::Escape);
    h.frame(two_focusables);
    assert_eq!(h.state.focused(), None);
}

// -------------------------------------------------------------------- scroll

/// A list taller than any viewport we give it.
fn long_list(ui: &mut Ui) -> ScrollState {
    let (_, st) = ui.scroll_area(Rect::new(0, 0, 100, 40), "list", |ui| {
        for i in 0..10 {
            ui.label(&format!("row {i}"));
        }
    });
    st
}

/// A short list that fits its viewport with room to spare.
fn short_list(ui: &mut Ui) -> ScrollState {
    let (_, st) = ui.scroll_area(Rect::new(0, 0, 100, 80), "list", |ui| {
        ui.label("only");
        ui.label("two");
    });
    st
}

/// Ten stacked buttons; reports which one registered a click.
fn button_list(ui: &mut Ui) -> Option<usize> {
    let mut hit = None;
    ui.scroll_area(Rect::new(0, 0, 100, 30), "list", |ui| {
        for i in 0..10 {
            if ui.button(&format!("B{i}")).clicked {
                hit = Some(i);
            }
        }
    });
    hit
}

#[test]
fn a_scroll_area_measures_its_own_content() {
    let mut h = Harness::new();
    let st = h.frame(long_list);
    assert!(
        st.content > st.viewport,
        "ten rows must not fit in a 40px viewport"
    );
    assert!(st.scrollable());
    assert!(st.max_offset() > 0.0);
}

#[test]
fn content_that_fits_does_not_scroll() {
    let mut h = Harness::new();
    let st = h.frame(short_list);
    assert!(!st.scrollable());
    assert_eq!(
        st.max_offset(),
        0.0,
        "a short list must not invent scroll range"
    );
}

#[test]
fn the_wheel_scrolls_and_clamps_at_both_ends() {
    let mut h = Harness::new();
    h.input.mouse = Point::new(50, 20);
    let first = h.frame(long_list);
    assert_eq!(first.target, 0.0, "a fresh list starts at the top");

    // Negative wheel is "scroll down", matching the platform convention.
    h.input.wheel = -1.0;
    let st = h.frame(long_list);
    assert!(st.target > 0.0, "one notch down should move the content");

    // Far past the end: must stop at the last pixel of content, not sail past.
    h.input.wheel = -500.0;
    let st = h.frame(long_list);
    assert_eq!(
        st.target,
        st.max_offset(),
        "scrolling down clamps to the end"
    );

    h.input.wheel = 500.0;
    let st = h.frame(long_list);
    assert_eq!(st.target, 0.0, "scrolling up clamps to the top");
}

#[test]
fn the_wheel_is_ignored_when_the_pointer_is_elsewhere() {
    let mut h = Harness::new();
    h.frame(long_list);

    h.input.mouse = Point::new(180, 90); // outside the scroll area
    h.input.wheel = -5.0;
    let st = h.frame(long_list);
    assert_eq!(
        st.target, 0.0,
        "a wheel event outside the area must not scroll it"
    );
}

#[test]
fn clicks_inside_the_viewport_reach_the_widget_under_them() {
    let mut h = Harness::new();
    h.frame(button_list); // measure

    // B1 spans y 19..34 (15px tall, 4px gap); y=25 is inside both it and the
    // 30px viewport.
    h.input.mouse = Point::new(20, 25);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(button_list);

    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let hit = h.frame(button_list);
    assert_eq!(hit, Some(1));
}

#[test]
fn a_widget_clipped_by_the_viewport_stops_responding_where_it_stops_being_drawn() {
    let mut h = Harness::new();
    h.frame(button_list);

    // y=32 is still inside B1's rect, but past the bottom of the 30px
    // viewport. Hit testing has to honour the clip, or you get a button that
    // reacts to clicks on whatever is drawn over it.
    h.input.mouse = Point::new(20, 32);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(button_list);

    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let hit = h.frame(button_list);
    assert_eq!(
        hit, None,
        "a click below the viewport must not reach the clipped widget"
    );
}

/// Press and release at the current pointer position, returning what was hit.
fn click_here(h: &mut Harness) -> Option<usize> {
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(button_list);
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    h.frame(button_list)
}

#[test]
fn scrolling_changes_which_widget_is_under_a_fixed_point() {
    let mut h = Harness::new();
    h.input.mouse = Point::new(20, 7);
    h.frame(button_list);

    assert_eq!(
        click_here(&mut h),
        Some(0),
        "at rest, y=7 is inside the first button"
    );

    // Buttons are 15 tall with a 4px gap, so they repeat every 19px. Two wheel
    // notches is 2 * 3 lines * 9px = 54px, putting content y = 7 + 54 = 61
    // inside button 3, which spans 57..72.
    h.input.wheel = -2.0;
    h.frame(button_list);
    for _ in 0..40 {
        h.frame(button_list); // let the scroll spring settle
    }

    assert_eq!(
        click_here(&mut h),
        Some(3),
        "the pointer has not moved, but the content under it has"
    );
}

#[test]
fn dragging_the_thumb_scrolls_the_content() {
    let mut h = Harness::new();
    h.frame(long_list);

    // The bar sits in the rightmost 7px of the area, so x=96 is on the thumb.
    h.input.mouse = Point::new(96, 2);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(long_list);

    h.input.mouse_pressed = false;
    h.input.mouse = Point::new(96, 30);
    let st = h.frame(long_list);
    assert!(
        st.target > 0.0,
        "dragging the thumb down should scroll the content down"
    );
    assert_eq!(
        st.shown, st.target,
        "a drag tracks the pointer exactly, with no easing lag"
    );
}

// ------------------------------------------------------------------ geometry

#[test]
fn adaptive_grows_the_canvas_instead_of_the_pixels() {
    let base = resolve_geometry(Scaling::Adaptive, (576, 352), 2, 2.0, (2304, 1408));
    assert_eq!(base.scale, 4, "2 logical points per pixel on a 2x display");
    assert_eq!(base.canvas, (576, 352));

    let wider = resolve_geometry(Scaling::Adaptive, (576, 352), 2, 2.0, (2704, 1408));
    assert_eq!(wider.scale, 4, "the magnification does not change");
    assert_eq!(
        wider.canvas.0, 676,
        "the extra 400 pixels became 100 columns of canvas"
    );
}

#[test]
fn adaptive_moves_one_canvas_pixel_per_scale_pixels_of_window() {
    // This is the whole point: the step is the scale, not the canvas width.
    let at = |w| {
        resolve_geometry(Scaling::Adaptive, (576, 352), 2, 2.0, (w, 1408))
            .canvas
            .0
    };
    let start = at(2304);
    assert_eq!(at(2305), start, "sub-step movement holds still");
    assert_eq!(at(2308), start + 1, "one step is 4 physical pixels");
    assert_eq!(at(2312), start + 2);
}

#[test]
fn fixed_magnifies_in_whole_canvas_widths() {
    // The contrast: a fixed canvas only changes size when a whole extra
    // multiple fits, which is a very large step indeed.
    let at = |w| resolve_geometry(Scaling::Fixed, (576, 352), 2, 2.0, (w, 4000)).scale;
    assert_eq!(at(2304), 4);
    assert_eq!(at(2879), 4, "still 4 until a fifth copy fits");
    assert_eq!(at(2880), 5);
}

#[test]
fn an_adaptive_canvas_never_exceeds_its_window() {
    // The presenter has no way to shrink an oversized canvas without squashing
    // it, so the geometry must guarantee it fits.
    for w in [640, 641, 999, 1000, 1001, 2303, 2304] {
        for dpr in [1.0, 1.5, 2.0] {
            let g = resolve_geometry(Scaling::Adaptive, (576, 352), 3, dpr, (w, w));
            assert!(
                g.canvas.0 * g.scale <= w,
                "canvas {:?} at x{} overflows a {w}px window",
                g.canvas,
                g.scale
            );
        }
    }
}

#[test]
fn an_adaptive_canvas_is_pinned_to_the_corner() {
    // Centring would split the sub-scale remainder across both edges, shunting
    // the whole UI sideways by a pixel every few pixels of a drag.
    for w in 2300..2320 {
        let g = resolve_geometry(Scaling::Adaptive, (576, 352), 2, 2.0, (w, 1408));
        assert_eq!(g.offset, (0, 0), "width {w} should not shift the canvas");
    }
}

#[test]
fn a_fixed_canvas_is_centred_in_its_letterbox() {
    let g = resolve_geometry(Scaling::Fixed, (576, 352), 2, 2.0, (2500, 1408));
    assert_eq!(g.scale, 4);
    assert_eq!(g.offset.0, (2500 - 576 * 4) / 2);
    assert_eq!(g.canvas, (576, 352), "a fixed canvas keeps its size");
}

#[test]
fn display_density_changes_the_magnification_not_the_layout() {
    // The same window in logical terms should give the same canvas on a 1x and
    // a 2x display — only the physical magnification differs.
    let one = resolve_geometry(Scaling::Adaptive, (576, 352), 2, 1.0, (1152, 704));
    let two = resolve_geometry(Scaling::Adaptive, (576, 352), 2, 2.0, (2304, 1408));
    assert_eq!(
        one.canvas, two.canvas,
        "layout gets the same room either way"
    );
    assert_eq!((one.scale, two.scale), (2, 4));
}

#[test]
fn a_degenerate_window_still_produces_a_usable_canvas() {
    let g = resolve_geometry(Scaling::Adaptive, (576, 352), 3, 2.0, (1, 1));
    assert!(
        g.canvas.0 >= 16 && g.canvas.1 >= 16,
        "never a zero-sized canvas"
    );
    assert!(g.scale >= 1);
}
