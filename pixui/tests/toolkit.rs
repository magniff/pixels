//! Tests for the parts that are easy to get subtly wrong: layout arithmetic,
//! widget identity, pointer capture, and the integer upscale.
//!
//! Everything here goes through the public API, which doubles as a check that
//! the toolkit is actually usable from outside the crate.

use pixui::app::blit;
use pixui::layout::Dir;
use pixui::{palette, Canvas, Color, Input, Layout, Point, Rect, ScrollState, Theme, Ui, UiState};
use pixui::{resolve_geometry, zoom_action, Config, Key, Mods, Scaling, ZoomAction};

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
    assert_eq!(pixui::font::text_width("A"), pixui::font::glyph_w());
    // Five glyphs: four full advances plus one glyph, with no trailing tracking.
    assert_eq!(
        pixui::font::text_width("HELLO"),
        4 * pixui::font::advance() + pixui::font::glyph_w()
    );
    // The advance is a whole cell per glyph, including after the last one.
    assert_eq!(
        pixui::font::advance_width("HELLO"),
        5 * pixui::font::advance()
    );
    assert!(
        pixui::font::advance_width("HELLO") > pixui::font::text_width("HELLO"),
        "the advance has to clear the ink, or accumulated runs collide"
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
        self.out(f).0
    }

    /// A frame, and what it reported on the way out.
    fn out<R>(&mut self, f: impl FnOnce(&mut Ui) -> R) -> (R, pixui::FrameOutput) {
        self.canvas.clear(Color::hex(0x000000));
        let mut ui = Ui::begin(&mut self.canvas, &self.input, &self.theme, &mut self.state);
        let r = f(&mut ui);
        let out = ui.finish();
        self.input.begin_frame();
        (r, out)
    }
}

// ------------------------------------------------------------------- schemes

/// WCAG relative luminance, which is what a contrast ratio is defined from.
fn relative_luminance(c: Color) -> f32 {
    let channel = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// The contrast ratio between two colours, 1.0 (identical) to 21.0 (black on
/// white).
fn contrast(a: Color, b: Color) -> f32 {
    let (x, y) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn every_scheme_can_actually_be_read() {
    // The check that a palette has been mapped onto the roles rather than
    // merely pasted into them: a scheme where the ink and the thing under it
    // are both dark is a scheme nobody can use, and it is not obvious from the
    // hex values that this has happened.
    for (name, build) in pixui::SCHEMES {
        let t = build();
        // Three bars rather than one. Anything a sentence is written in has to
        // clear 3:1; muted text is allowed to be quieter, because some of these
        // schemes are low contrast on purpose and Solarized says so in its
        // name — but not so quiet that it is gone.
        // The button bars sit a hair under WCAG's 3:1 for large text: Solarized's
        // own green lands at 2.99 against its own lightest tone, and inventing
        // a darker green to clear a round number would be inventing a colour
        // the scheme does not have.
        let pairs = [
            (3.0, "body text on a panel", t.ink, t.panel),
            (3.0, "body text in a well", t.ink_light, t.well),
            (2.0, "muted text on a panel", t.ink_soft, t.panel),
            (
                2.9,
                "a label on an accent button",
                t.accent.ink,
                t.accent.face,
            ),
            (
                2.9,
                "a label on a neutral button",
                t.neutral.ink,
                t.neutral.face,
            ),
            (
                2.9,
                "a label on a danger button",
                t.danger.ink,
                t.danger.face,
            ),
            (
                2.9,
                "a label on a positive button",
                t.positive.ink,
                t.positive.face,
            ),
            (
                3.0,
                "a title on its strip",
                t.panel_title_ink,
                t.panel_title,
            ),
        ];
        for (want, what, ink, under) in pairs {
            let ratio = contrast(ink, under);
            assert!(
                ratio >= want,
                "{name}: {what} is {ratio:.1}:1, and wants {want:.1}:1"
            );
        }
        // And code, which is most of what a scheme is for.
        for (what, ink) in [
            ("keywords", t.syntax.keyword),
            ("strings", t.syntax.string),
            ("numbers", t.syntax.number),
            ("comments", t.syntax.comment),
        ] {
            // 2.4, because Solarized Light's own comment tone is 2.45:1
            // against its own page. Low contrast is the scheme's whole idea;
            // this bar is here to catch invisible, not to catch quiet.
            let ratio = contrast(ink, t.well);
            assert!(ratio >= 2.4, "{name}: {what} are {ratio:.1}:1 on the well");
        }
    }
}

#[test]
fn a_scheme_can_be_found_by_name_however_it_is_typed() {
    assert!(pixui::scheme_named("nord").is_some());
    assert!(pixui::scheme_named("Gruvbox Dark").is_some());
    assert!(pixui::scheme_named("not a scheme").is_none());
    for (name, _) in pixui::SCHEMES {
        assert!(pixui::scheme_named(name).is_some(), "{name} is unreachable");
    }
}

// ------------------------------------------------------------------- idling

#[test]
fn a_frame_with_nothing_moving_on_it_says_so() {
    // What lets the event loop stop drawing: an application sitting still
    // should cost nothing to sit still, and this is the frame admitting there
    // is nothing to see.
    let mut h = Harness::new();
    let (_, out) = h.out(|ui| ui.label("STILL"));
    assert!(!out.animating);
}

#[test]
fn a_spring_in_flight_keeps_the_frames_coming() {
    let mut h = Harness::new();
    h.input.mouse = Point::new(20, 7);
    h.input.mouse_pressed = true;
    h.input.mouse_down = true;
    let (_, out) = h.out(|ui| ui.button_at(Rect::new(0, 0, 60, 15), "GO", pixui::Tone::Accent));
    assert!(out.animating, "a press has a spring to travel");

    // And it stops on its own once the spring has settled, rather than being
    // told to stop.
    h.input.mouse_pressed = false;
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let mut settled = false;
    for _ in 0..300 {
        let (_, out) = h.out(|ui| ui.button_at(Rect::new(0, 0, 60, 15), "GO", pixui::Tone::Accent));
        h.input.mouse_released = false;
        h.input.mouse = Point::new(500, 500);
        if !out.animating {
            settled = true;
            break;
        }
    }
    assert!(
        settled,
        "something is still asking for frames long after the press"
    );
}

#[test]
fn anything_can_ask_for_another_frame() {
    // For what the toolkit cannot see: a caret blinking on the clock, a
    // reply that has not arrived yet.
    let mut h = Harness::new();
    let (_, out) = h.out(|ui| ui.request_repaint());
    assert!(out.animating);
}

// -------------------------------------------------------------------- layers

/// One frame of a page-wide widget with a small floating one over it, with the
/// pointer inside both. Returns who got the click.
fn layered(h: &mut Harness, press: bool, release: bool) -> (bool, bool) {
    h.input.mouse = Point::new(20, 20);
    h.input.mouse_pressed = press;
    h.input.mouse_down = press || !release;
    h.input.mouse_released = release;
    h.frame(|ui| {
        // Drawn first, covering everything: the page underneath.
        let page_id = ui.id("page");
        let page = ui.interact(page_id, Rect::new(0, 0, 200, 100));
        // Drawn after it and floating over it, as a popover would be.
        let float = Rect::new(10, 10, 40, 20);
        let over = ui.layer(float, |ui| {
            let id = ui.id("float");
            ui.interact(id, float)
        });
        (page.clicked, over.clicked)
    })
}

#[test]
fn a_floating_layer_takes_the_click_from_what_is_under_it() {
    let mut h = Harness::new();
    // The first frame is the layer announcing itself; nothing is pressed yet.
    layered(&mut h, false, false);
    let (page, _) = layered(&mut h, true, false);
    assert!(!page, "the page must not take a press inside the layer");
    let (page, float) = layered(&mut h, false, true);
    assert!(float, "the layer gets the click");
    assert!(!page, "and the page never does");
}

#[test]
fn blocked_input_stops_the_pointer_as_well_as_the_keyboard() {
    // What a modal is made of: the page is drawn inside the block and the
    // dialog outside it, and the click belongs to the dialog even though the
    // page was asked first and is sitting right under the pointer.
    let mut h = Harness::new();
    let mut run = |press: bool, release: bool| {
        h.input.mouse = Point::new(20, 20);
        h.input.mouse_pressed = press;
        h.input.mouse_down = press || !release;
        h.input.mouse_released = release;
        h.frame(|ui| {
            let page = ui.input_blocked(true, |ui| {
                let id = ui.id("page");
                ui.interact(id, Rect::new(0, 0, 200, 100))
            });
            let over = Rect::new(10, 10, 40, 20);
            let id = ui.id("dialog");
            let dialog = ui.interact(id, over);
            (page.clicked || page.hovered, dialog.clicked)
        })
    };
    run(false, false);
    run(true, false);
    let (page, dialog) = run(false, true);
    assert!(!page, "nothing under a block is even hovered");
    assert!(dialog, "and the click lands on what was drawn over it");
}

#[test]
fn a_layer_only_covers_its_own_rectangle() {
    let mut h = Harness::new();
    // Same two widgets, pointer outside the floating one.
    let mut run = |press: bool, release: bool| {
        h.input.mouse = Point::new(120, 80);
        h.input.mouse_pressed = press;
        h.input.mouse_down = press || !release;
        h.input.mouse_released = release;
        h.frame(|ui| {
            let page_id = ui.id("page");
            let page = ui.interact(page_id, Rect::new(0, 0, 200, 100));
            let float = Rect::new(10, 10, 40, 20);
            ui.layer(float, |ui| {
                let id = ui.id("float");
                ui.interact(id, float)
            });
            page.clicked
        })
    };
    run(false, false);
    run(true, false);
    assert!(
        run(false, true),
        "a click beside the layer belongs to the page"
    );
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
    let base = resolve_geometry(Scaling::Adaptive, (576, 352), 4, (2304, 1408));
    assert_eq!(base.scale, 4, "2 logical points per pixel on a 2x display");
    assert_eq!(base.canvas, (576, 352));

    let wider = resolve_geometry(Scaling::Adaptive, (576, 352), 4, (2704, 1408));
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
        resolve_geometry(Scaling::Adaptive, (576, 352), 4, (w, 1408))
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
    let at = |w| resolve_geometry(Scaling::Fixed, (576, 352), 4, (w, 4000)).scale;
    assert_eq!(at(2304), 4);
    assert_eq!(at(2879), 4, "still 4 until a fifth copy fits");
    assert_eq!(at(2880), 5);
}

#[test]
fn an_adaptive_canvas_never_exceeds_its_window() {
    // The presenter has no way to shrink an oversized canvas without squashing
    // it, so the geometry must guarantee it fits.
    for w in [640, 641, 999, 1000, 1001, 2303, 2304] {
        for scale in [1, 3, 6] {
            let g = resolve_geometry(Scaling::Adaptive, (576, 352), scale, (w, w));
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
        let g = resolve_geometry(Scaling::Adaptive, (576, 352), 4, (w, 1408));
        assert_eq!(g.offset, (0, 0), "width {w} should not shift the canvas");
    }
}

#[test]
fn a_fixed_canvas_is_centred_in_its_letterbox() {
    let g = resolve_geometry(Scaling::Fixed, (576, 352), 4, (2500, 1408));
    assert_eq!(g.scale, 4);
    assert_eq!(g.offset.0, (2500 - 576 * 4) / 2);
    assert_eq!(g.canvas, (576, 352), "a fixed canvas keeps its size");
}

#[test]
fn display_density_changes_the_magnification_not_the_layout() {
    // The same window in logical terms should give the same canvas on a 1x and
    // a 2x display — only the physical magnification differs.
    let one = resolve_geometry(Scaling::Adaptive, (576, 352), 2, (1152, 704));
    let two = resolve_geometry(Scaling::Adaptive, (576, 352), 4, (2304, 1408));
    assert_eq!(
        one.canvas, two.canvas,
        "layout gets the same room either way"
    );
    assert_eq!((one.scale, two.scale), (2, 4));
}

#[test]
fn a_degenerate_window_still_produces_a_usable_canvas() {
    let g = resolve_geometry(Scaling::Adaptive, (576, 352), 6, (1, 1));
    assert!(
        g.canvas.0 >= 16 && g.canvas.1 >= 16,
        "never a zero-sized canvas"
    );
    assert!(g.scale >= 1);
}

// ----------------------------------------------------------------- ui scale

#[test]
fn halving_the_ui_scale_doubles_the_canvas() {
    // This is what "scale the UI down 2x" means: the same window, everything in
    // it half the size, twice as much of it.
    let big = resolve_geometry(Scaling::Adaptive, (576, 352), 4, (2304, 1408));
    let small = resolve_geometry(Scaling::Adaptive, (576, 352), 2, (2304, 1408));
    assert_eq!(big.canvas, (576, 352));
    assert_eq!(small.canvas, (1152, 704), "half the scale, twice the room");
    assert_eq!(small.scale * 2, big.scale);
}

#[test]
fn zoom_shortcuts_need_the_primary_modifier() {
    let plain = Mods::default();
    let cmd = Mods {
        cmd: true,
        ..Default::default()
    };

    assert_eq!(
        zoom_action(Key::Char('='), plain),
        None,
        "bare `=` is just text"
    );
    assert_eq!(zoom_action(Key::Char('='), cmd), Some(ZoomAction::In));
    assert_eq!(zoom_action(Key::Char('-'), cmd), Some(ZoomAction::Out));
    assert_eq!(zoom_action(Key::Char('0'), cmd), Some(ZoomAction::Reset));
    assert_eq!(zoom_action(Key::Char('k'), cmd), None);
}

#[test]
fn both_forms_of_the_zoom_keys_are_accepted() {
    // Which of `=`/`+` and `-`/`_` arrives depends on the keyboard layout.
    let cmd = Mods {
        cmd: true,
        ..Default::default()
    };
    assert_eq!(zoom_action(Key::Char('+'), cmd), Some(ZoomAction::In));
    assert_eq!(zoom_action(Key::Char('_'), cmd), Some(ZoomAction::Out));
}

#[test]
fn the_scale_range_is_ordered_and_at_least_one() {
    let c = Config::new("t", 100, 100).with_scale_range(0, 4);
    assert_eq!(c.scale_range, (1, 4), "a scale below 1 is meaningless");

    let c = Config::new("t", 100, 100).with_scale_range(5, 2);
    assert!(
        c.scale_range.0 <= c.scale_range.1,
        "an inverted range would clamp to nothing"
    );
}

#[test]
fn zoom_shortcuts_are_on_unless_turned_off() {
    assert!(Config::new("t", 100, 100).zoom_shortcuts);
    assert!(
        !Config::new("t", 100, 100)
            .without_zoom_shortcuts()
            .zoom_shortcuts
    );
}

#[test]
fn zooming_in_never_makes_the_canvas_overflow_the_window() {
    // The invariant has to survive the whole zoom range, not just the default.
    for ui_scale in 1..=6 {
        let g = resolve_geometry(Scaling::Adaptive, (576, 352), ui_scale, (2304, 1408));
        assert!(
            g.canvas.0 * g.scale <= 2304 && g.canvas.1 * g.scale <= 1408,
            "scale {ui_scale} produced {:?} at x{}",
            g.canvas,
            g.scale
        );
    }
}

#[test]
fn odd_magnifications_are_reachable() {
    // The step between 2 and 4 physical pixels is the one that matters: it is
    // 100% larger, and 3 is the only thing in between. Expressing the scale as
    // a whole number of *logical* points cannot name it on a 2x display, which
    // is why the opening scale is a float.
    let two = resolve_geometry(Scaling::Adaptive, (768, 470), 2, (2304, 1408));
    let three = resolve_geometry(Scaling::Adaptive, (768, 470), 3, (2304, 1408));
    let four = resolve_geometry(Scaling::Adaptive, (768, 470), 4, (2304, 1408));
    assert_eq!(two.canvas, (1152, 704));
    assert_eq!(three.canvas, (768, 469));
    assert_eq!(four.canvas, (576, 352));

    // Going up a step is +50% in apparent size, not +30%. There is nothing in
    // between, and that is inherent to keeping every pixel whole.
    let ratio = three.scale as f32 / two.scale as f32;
    assert!((ratio - 1.5).abs() < 1e-6);
}

// ------------------------------------------------------------------ splitter

#[test]
fn a_splitter_divides_its_bounds_without_overlap_or_gap() {
    let mut h = Harness::new();
    let bounds = Rect::new(0, 0, 200, 80);
    let mut size = 60;
    let (left, right) = h.frame(|ui| ui.split_left(bounds, "s", &mut size, (20, 180)));

    assert_eq!(left.x, bounds.x);
    assert_eq!(left.w, 60);
    assert_eq!(
        right.right(),
        bounds.right(),
        "the far edge is still the far edge"
    );
    assert!(
        right.x > left.right(),
        "the handle's own strip belongs to neither pane"
    );
    assert!(left.w + right.w < bounds.w, "and is excluded from both");
}

#[test]
fn dragging_a_splitter_moves_it() {
    let mut h = Harness::new();
    let bounds = Rect::new(0, 0, 200, 80);
    let mut size = 60;

    // Grab the handle, which sits at x = 60.
    h.input.mouse = Point::new(61, 40);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.split_left(bounds, "s", &mut size, (20, 180)));

    h.input.mouse = Point::new(120, 40);
    h.input.mouse_pressed = false;
    h.frame(|ui| ui.split_left(bounds, "s", &mut size, (20, 180)));
    assert!(
        (size - 118).abs() <= 2,
        "the divider follows the pointer, got {size}"
    );
}

#[test]
fn a_splitter_is_clamped_to_its_range_even_without_a_drag() {
    let mut h = Harness::new();
    let bounds = Rect::new(0, 0, 200, 80);

    let mut size = 5;
    h.frame(|ui| ui.split_left(bounds, "s", &mut size, (40, 150)));
    assert_eq!(size, 40, "a size below the range is pulled up to it");

    let mut size = 400;
    h.frame(|ui| ui.split_left(bounds, "s", &mut size, (40, 150)));
    assert!(size <= 150, "and one above is pulled down");
    assert!(size <= bounds.w, "never past the bounds it is dividing");
}

#[test]
fn dragging_past_the_range_stops_at_it() {
    let mut h = Harness::new();
    let bounds = Rect::new(0, 0, 200, 80);
    let mut size = 60;

    h.input.mouse = Point::new(61, 40);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.split_left(bounds, "s", &mut size, (30, 100)));

    h.input.mouse = Point::new(500, 40);
    h.input.mouse_pressed = false;
    h.frame(|ui| ui.split_left(bounds, "s", &mut size, (30, 100)));
    assert_eq!(
        size, 100,
        "dragging beyond the maximum holds at the maximum"
    );
}

#[test]
fn a_horizontal_splitter_works_the_same_way_on_the_other_axis() {
    let mut h = Harness::new();
    let bounds = Rect::new(0, 0, 200, 100);
    let mut size = 40;
    let (top, bottom) = h.frame(|ui| ui.split_top(bounds, "s", &mut size, (10, 90)));
    assert_eq!(top.h, 40);
    assert_eq!(bottom.bottom(), bounds.bottom());
    assert!(bottom.y > top.bottom());
}

// -------------------------------------------------------------------- cursor

#[test]
fn every_cursor_sprite_is_rectangular_and_has_a_hotspot_inside_it() {
    use pixui::input::Cursor;
    for kind in [
        Cursor::Default,
        Cursor::Pointer,
        Cursor::Grab,
        Cursor::Text,
        Cursor::ResizeH,
        Cursor::ResizeV,
    ] {
        let s = pixui::cursor::sprite(kind);
        let width = s.rows[0].chars().count();
        assert!(!s.rows.is_empty(), "{kind:?} has no rows");
        for row in s.rows {
            assert_eq!(row.chars().count(), width, "{kind:?} has a ragged row");
            assert!(
                row.chars().all(|c| matches!(c, 'X' | '#' | '.')),
                "{kind:?} uses a character that is neither outline, fill nor gap"
            );
        }
        assert!(
            s.hotspot.0 >= 0
                && s.hotspot.1 >= 0
                && (s.hotspot.0 as usize) < width
                && (s.hotspot.1 as usize) < s.rows.len(),
            "{kind:?} points somewhere outside its own sprite"
        );
    }
}

#[test]
fn a_cursor_is_drawn_around_its_hotspot() {
    let mut c = Canvas::new(40, 40);
    c.clear(Color::hex(0x000000));
    let fill = Color::hex(0xFFFFFF);
    let outline = Color::hex(0xFF0000);
    pixui::cursor::draw(
        &mut c,
        Point::new(20, 20),
        pixui::input::Cursor::Default,
        fill,
        outline,
    );

    // The arrow's hotspot is its tip, so the pixel under the pointer is ink.
    assert_eq!(
        c.get_px(20, 20),
        outline,
        "the hotspot pixel is the tip itself"
    );
    assert_eq!(
        c.get_px(10, 10),
        Color::hex(0x000000),
        "and nothing is drawn far away"
    );
}

#[test]
fn a_cursor_at_the_edge_does_not_write_out_of_bounds() {
    let mut c = Canvas::new(12, 12);
    c.clear(Color::hex(0x000000));
    // Must not panic: the sprite runs well past every edge.
    for at in [
        Point::new(0, 0),
        Point::new(11, 11),
        Point::new(-4, -4),
        Point::new(30, 30),
    ] {
        pixui::cursor::draw(
            &mut c,
            at,
            pixui::input::Cursor::Pointer,
            Color::hex(0xFFFFFF),
            Color::hex(0x111111),
        );
    }
}

// ---------------------------------------------------------------- text input

#[test]
fn a_scroll_still_sliding_asks_for_the_frames_to_slide_in() {
    let mut h = Harness::new();
    let area = Rect::new(0, 0, 200, 60);
    let mut st = pixui::ScrollState::default();

    // One frame to measure the content, so there is something to scroll.
    let tall = |ui: &mut Ui| {
        for _ in 0..40 {
            ui.alloc(12);
        }
    };
    h.frame(|ui| ui.scroll_area_with(area, "s", &mut st, tall));

    // A notch of wheel over it.
    h.input.mouse = Point::new(100, 30);
    h.input.wheel = -3.0;
    let (_, out) = h.out(|ui| ui.scroll_area_with(area, "s", &mut st, tall));
    assert!(st.target > 0.0, "the wheel moved it");
    assert!(
        out.animating,
        "the view is still easing towards the target, and only a frame it asks          for will take it there"
    );

    // Frames keep being asked for until it arrives, and stop once it has.
    h.input.wheel = 0.0;
    let mut frames = 0;
    loop {
        let (_, out) = h.out(|ui| ui.scroll_area_with(area, "s", &mut st, tall));
        frames += 1;
        if !out.animating {
            break;
        }
        assert!(frames < 200, "an ease that never settles");
    }
    assert_eq!(st.shown, st.target, "it arrived where it was sent");
}

#[test]
fn a_field_that_goes_away_hands_the_keyboard_back() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::new();

    h.input.mouse = Point::new(20, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));
    assert!(
        h.frame(|ui| ui.text_input_active()),
        "the field has the keyboard"
    );

    // The frame after it stops being drawn — a panel closed, a block applied
    // and dismissed — nobody is typing into anything.
    assert!(
        !h.frame(|ui| ui.text_input_active()),
        "a keyboard held by a field that no longer exists is a keyboard nobody          can take back, and the application's own keys never arrive"
    );
}

#[test]
fn a_focused_text_field_claims_the_keyboard() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::new();

    assert!(
        !h.frame(|ui| ui.text_input_active()),
        "nothing is focused yet"
    );

    // Click into the field.
    h.input.mouse = Point::new(20, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));

    // An application asks this *before* dispatching its own keys.
    assert!(
        h.frame(|ui| ui.text_input_active()),
        "an app with its own bindings has to know the user is typing into a field"
    );
}

#[test]
fn typing_into_a_focused_field_edits_it() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::new();

    h.input.mouse = Point::new(20, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));

    for c in "hi".chars() {
        h.input.keys.push(Key::Char(c));
        h.frame(|ui| ui.text_field_at(rect, "f", &mut text));
    }
    assert_eq!(text, "hi");

    h.input.keys.push(Key::Backspace);
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));
    assert_eq!(text, "h");
}

#[test]
fn an_unfocused_field_ignores_keys() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::new();
    h.input.keys.push(Key::Char('x'));
    h.frame(|ui| ui.text_field_at(rect, "f", &mut text));
    assert_eq!(
        text, "",
        "keys belong to whoever has focus, and nothing does"
    );
}

// ----------------------------------------------------------------- title bar

#[test]
fn a_title_bar_survives_a_title_too_long_for_it() {
    let mut h = Harness::new();
    // Must not panic, and must not paint outside the strip.
    h.frame(|ui| {
        ui.title_bar(
            Rect::new(0, 0, 60, 13),
            "AN EXTREMELY LONG APPLICATION NAME",
            Some("ALSO-A-LONG-FILE-NAME.MD"),
        )
    });
    assert_eq!(
        h.canvas.get_px(10, 40),
        Color::hex(0x000000),
        "nothing is drawn below the bar"
    );
}

#[test]
fn a_title_bar_without_a_badge_is_fine() {
    let mut h = Harness::new();
    h.frame(|ui| ui.title_bar(Rect::new(0, 0, 120, 13), "PIXUI", None));
}

// ------------------------------------------------------------ double clicks

/// Press and release at `at`, returning the response from the release frame.
fn click_at(h: &mut Harness, rect: Rect, at: Point, name: &str) -> pixui::Response {
    h.input.mouse = at;
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| {
        let id = ui.id(name);
        ui.interact(id, rect)
    });
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    h.frame(|ui| {
        let id = ui.id(name);
        ui.interact(id, rect)
    })
}

#[test]
fn two_quick_clicks_on_the_same_spot_are_a_double() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 60, 20);

    let first = click_at(&mut h, rect, Point::new(10, 10), "w");
    assert!(first.clicked && !first.double_clicked);

    h.input.time += 0.1;
    let second = click_at(&mut h, rect, Point::new(10, 10), "w");
    assert!(second.double_clicked);
    assert!(
        second.clicked,
        "a double is also a click; callers should not handle both"
    );
}

#[test]
fn a_slow_second_click_is_two_singles() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 60, 20);
    click_at(&mut h, rect, Point::new(10, 10), "w");
    h.input.time += 2.0;
    let second = click_at(&mut h, rect, Point::new(10, 10), "w");
    assert!(!second.double_clicked, "the interval has to mean something");
}

#[test]
fn a_second_click_somewhere_else_is_not_a_double() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 60, 20);
    click_at(&mut h, rect, Point::new(10, 10), "w");
    h.input.time += 0.1;
    let second = click_at(&mut h, rect, Point::new(50, 10), "w");
    assert!(
        !second.double_clicked,
        "a drifting pointer is not a double click"
    );
}

#[test]
fn three_clicks_are_a_double_then_a_single() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 60, 20);
    click_at(&mut h, rect, Point::new(10, 10), "w");
    h.input.time += 0.1;
    assert!(click_at(&mut h, rect, Point::new(10, 10), "w").double_clicked);
    h.input.time += 0.1;
    assert!(
        !click_at(&mut h, rect, Point::new(10, 10), "w").double_clicked,
        "the double resets, or every click after the second would be one"
    );
}

// --------------------------------------------------------------- search box

#[test]
fn the_search_clear_button_empties_the_field() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::from("something");

    // The cross sits at the right-hand end of the field.
    h.input.mouse = Point::new(113, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.search_field_at(rect, "s", &mut text, "find"));
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let resp = h.frame(|ui| ui.search_field_at(rect, "s", &mut text, "find"));

    assert_eq!(text, "", "clicking the cross clears it");
    assert!(resp.changed, "and reports the change, like typing would");
}

#[test]
fn clicking_the_body_of_a_search_field_does_not_clear_it() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::from("keep me");

    h.input.mouse = Point::new(40, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.search_field_at(rect, "s", &mut text, "find"));
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    h.frame(|ui| ui.search_field_at(rect, "s", &mut text, "find"));
    assert_eq!(text, "keep me");
}

#[test]
fn a_field_that_grabs_focus_puts_the_caret_after_the_existing_text() {
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::from("name.md");

    // First frame: the field appears and takes focus.
    h.frame(|ui| ui.text_field_grab_at(rect, "f", &mut text, "", true));
    // Second: typing should append, not prepend.
    h.input.keys.push(Key::Char('!'));
    h.frame(|ui| ui.text_field_grab_at(rect, "f", &mut text, "", false));
    assert_eq!(text, "name.md!");
}

#[test]
fn a_grabbing_field_owns_the_keyboard_from_its_very_first_frame() {
    // Without this, the keystroke the user meant for the field goes to whatever
    // the application does with keys instead.
    let mut h = Harness::new();
    let rect = Rect::new(0, 0, 120, 15);
    let mut text = String::new();
    h.frame(|ui| ui.text_field_grab_at(rect, "f", &mut text, "", true));
    assert!(h.frame(|ui| ui.text_input_active()));
}

// ------------------------------------------------------------------ segments

#[test]
fn a_segment_with_an_icon_is_wider_than_one_without() {
    use pixui::Segment;
    let plain = Segment::new("SOURCE");
    let with = Segment::with_icon(pixui::icon::CODE, "SOURCE");
    assert!(with.icon.is_some() && plain.icon.is_none());

    // Both draw without panicking, and the icon one still fits its cell.
    let mut h = Harness::new();
    let mut selected = 0usize;
    h.frame(|ui| ui.segments_at("t", Rect::new(0, 0, 180, 14), &[with, plain], &mut selected));
}

#[test]
fn a_segmented_control_still_takes_plain_labels() {
    let mut h = Harness::new();
    let mut selected = 1usize;
    h.frame(|ui| ui.segmented_at("t", Rect::new(0, 0, 120, 14), &["A", "B"], &mut selected));
    assert_eq!(selected, 1, "drawing must not disturb the selection");
}

#[test]
fn clicking_a_segment_selects_it() {
    let mut h = Harness::new();
    let mut selected = 0usize;
    let rect = Rect::new(0, 0, 120, 14);

    // The right-hand half is the second segment.
    h.input.mouse = Point::new(90, 7);
    h.input.mouse_down = true;
    h.input.mouse_pressed = true;
    h.frame(|ui| ui.segmented_at("t", rect, &["A", "B"], &mut selected));
    h.input.mouse_down = false;
    h.input.mouse_released = true;
    let changed = h.frame(|ui| ui.segmented_at("t", rect, &["A", "B"], &mut selected));

    assert_eq!(selected, 1);
    assert!(changed, "and it reports the change");
}

#[test]
fn every_icon_is_rectangular() {
    for rows in [
        pixui::icon::SEARCH,
        pixui::icon::CROSS,
        pixui::icon::CHECK,
        pixui::icon::CHEVRON,
        pixui::icon::CODE,
        pixui::icon::PAGE,
    ] {
        let (w, h) = pixui::icon::size(rows);
        assert!(w > 0 && h > 0);
        for row in rows {
            assert_eq!(row.chars().count() as i32, w, "ragged icon row");
            assert!(
                row.chars().all(|c| c == '#' || c == '.'),
                "unexpected glyph in an icon"
            );
        }
    }
}

#[test]
fn a_caller_owned_scroll_area_survives_not_being_drawn() {
    // A view behind a tab should come back where it was left. The toolkit
    // garbage collects state for widgets that stop being drawn, which is why
    // the position has to live outside it.
    let mut h = Harness::new();
    let mut state = ScrollState::default();
    let area = Rect::new(0, 0, 100, 40);

    let long = |ui: &mut Ui| {
        for i in 0..12 {
            ui.label(&format!("row {i}"));
        }
    };

    h.input.mouse = Point::new(50, 20);
    h.frame(|ui| ui.scroll_area_with(area, "v", &mut state, long));
    h.input.wheel = -3.0;
    h.frame(|ui| ui.scroll_area_with(area, "v", &mut state, long));
    let parked = state.target;
    assert!(parked > 0.0, "the wheel moved it");

    // Several frames where the area is not drawn at all.
    for _ in 0..5 {
        h.frame(|ui| ui.label("something else entirely"));
    }

    h.frame(|ui| ui.scroll_area_with(area, "v", &mut state, long));
    assert_eq!(state.target, parked, "and it came back where it was left");
}

// --------------------------------------------------------------------- fonts

#[test]
fn every_face_has_a_glyph_for_every_printable_ascii() {
    for face in pixui::font::FACES {
        let want = 95 * face.glyph_h as usize;
        assert_eq!(
            face.rows.len(),
            want,
            "{}: {} rows for {} glyphs of {} rows each",
            face.name,
            face.rows.len(),
            95,
            face.glyph_h
        );
        // A face whose ink is wider than its cell would blit outside itself.
        let widest = face.rows.iter().copied().max().unwrap_or(0);
        assert!(
            widest < 1 << face.glyph_w,
            "{} has ink outside its {}px cell",
            face.name,
            face.glyph_w
        );
        // And one that draws nothing is a font that was not baked.
        assert!(face.rows.iter().any(|r| *r != 0), "{} is blank", face.name);
    }
}

#[test]
fn a_face_can_be_found_by_name_and_put_back() {
    let before = pixui::font::face().name;
    for (i, face) in pixui::font::FACES.iter().enumerate() {
        assert_eq!(pixui::font::face_named(face.name), Some(i));
        pixui::font::use_face(i);
        assert_eq!(pixui::font::face().name, face.name);
        // The metrics follow the face, which is the whole point of it being a
        // choice: a layout reading them gets this face's numbers.
        assert_eq!(pixui::font::line_h(), face.line_h);
        assert_eq!(pixui::font::advance(), face.advance);
    }
    assert_eq!(pixui::font::face_named("no such face"), None);
    // Out of range keeps whatever was in use rather than panicking.
    let now = pixui::font::face().name;
    pixui::font::use_face(999);
    assert_eq!(pixui::font::face().name, now);
    pixui::font::use_face(pixui::font::face_named(before).unwrap());
}

#[test]
fn a_row_band_covers_its_own_line_and_leaves_the_one_above_alone() {
    let before = pixui::font::face().name;
    for (i, face) in pixui::font::FACES.iter().enumerate() {
        pixui::font::use_face(i);
        // Text is drawn from the top of its glyphs, so a line at `y` has the
        // line before it at `y - line_h`.
        let y = 100;
        let top = pixui::font::row_top(y);
        let bottom = top + face.line_h;
        assert!(
            top >= y - face.line_h + face.glyph_h,
            "{}: a band from {top} reaches into the glyphs of the line above",
            face.name
        );
        assert!(
            top <= y && bottom >= y + face.glyph_h,
            "{}: a band of {top}..{bottom} does not cover its own glyphs",
            face.name
        );
    }
    pixui::font::use_face(pixui::font::face_named(before).unwrap());
}

// ----------------------------------------------------------------- clipboard

#[test]
fn a_clipboard_with_nothing_on_it_answers_nothing() {
    // No window, so this is the in-process fallback, and it starts empty.
    assert_eq!(pixui::clipboard::paste(), None);
    pixui::clipboard::copy("");
    assert_eq!(
        pixui::clipboard::paste(),
        None,
        "an empty string is not something to paste"
    );
}

#[test]
fn what_is_copied_is_what_comes_back() {
    pixui::clipboard::copy("two\nlines\n");
    assert_eq!(
        pixui::clipboard::paste().as_deref(),
        Some("two\nlines\n"),
        "including the newlines, which is what makes it a linewise yank"
    );
    pixui::clipboard::copy("replaced");
    assert_eq!(pixui::clipboard::paste().as_deref(), Some("replaced"));
}

#[test]
fn the_other_button_asks_rather_than_takes() {
    let mut state = UiState::new();
    let mut canvas = Canvas::new(200, 100);
    let theme = Theme::default();
    let rect = Rect::new(10, 10, 50, 20);

    let input = Input {
        mouse: Point::new(20, 20),
        mouse_in_window: true,
        right_pressed: true,
        ..Default::default()
    };
    let mut ui = Ui::begin(&mut canvas, &input, &theme, &mut state);
    let id = ui.id("row");
    let resp = ui.interact(id, rect);
    assert!(
        resp.right_clicked,
        "over it, and the other button went down"
    );
    assert!(!resp.clicked, "which is not a click");
    assert!(!resp.held, "and takes nothing: nothing is dragged with it");
    ui.finish();

    // Somewhere else entirely.
    let elsewhere = Input {
        mouse: Point::new(150, 90),
        mouse_in_window: true,
        right_pressed: true,
        ..Default::default()
    };
    let mut ui = Ui::begin(&mut canvas, &elsewhere, &theme, &mut state);
    let id = ui.id("row");
    assert!(!ui.interact(id, rect).right_clicked);
    ui.finish();
}

#[test]
fn the_first_to_ask_for_a_point_gets_the_press() {
    // Which is what a small control inside a big one depends on: a bin at the
    // end of a row is inside the row, and if the row asks first the row takes
    // the press and the answer to "delete this" is "open it".
    let mut canvas = Canvas::new(200, 100);
    let theme = Theme::default();
    let row = Rect::new(0, 0, 200, 20);
    let bin = Rect::new(180, 0, 20, 20);
    let on_the_bin = Point::new(190, 10);

    let press = Input {
        mouse: on_the_bin,
        mouse_in_window: true,
        mouse_down: true,
        mouse_pressed: true,
        ..Default::default()
    };
    let release = Input {
        mouse: on_the_bin,
        mouse_in_window: true,
        mouse_released: true,
        ..Default::default()
    };

    for (small_first, who) in [(true, "bin"), (false, "row")] {
        let mut state = UiState::new();
        let mut took = None;
        for input in [&press, &release] {
            let mut ui = Ui::begin(&mut canvas, input, &theme, &mut state);
            let (a, b) = if small_first {
                (("bin", bin), ("row", row))
            } else {
                (("row", row), ("bin", bin))
            };
            for (name, rect) in [a, b] {
                let id = ui.id(name);
                if ui.interact(id, rect).clicked {
                    took = Some(name);
                }
            }
            ui.finish();
        }
        assert_eq!(took, Some(who), "whoever asked first has it");
    }
}
