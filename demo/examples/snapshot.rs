//! Render the demo UI to image files with no window and no event loop.
//!
//! This exists to make the layering claim testable rather than rhetorical.
//! `pixui::app` is the only module that knows about winit and softbuffer, so
//! driving the UI without it should be possible using nothing but the public
//! API — a canvas, an input struct, and the frame lifecycle. That is exactly
//! what this file does, and it is also how you would snapshot-test a pixel UI:
//! the output is deterministic down to the pixel.
//!
//! Run with: `cargo run -p pixui-demo --example snapshot`

use std::fs::File;
use std::io::{BufWriter, Write};

use demo::{frame, theme, Demo};
use pixui::{Canvas, Input, Point, Theme, Ui, UiState};

/// One thing to render: a name, some state tweaks, and a synthetic pointer.
struct Scene {
    name: &'static str,
    tab: usize,
    dark: bool,
    mouse: Point,
    down: bool,
    /// Wheel notches to feed in, to park a scroll area mid-list.
    wheel: f32,
    /// Frames to run before capturing, so springs settle where we want them.
    settle: u32,
}

fn main() -> std::io::Result<()> {
    #[rustfmt::skip]
    let scenes = [
        Scene { name: "demo-widgets",    tab: 0, dark: false, mouse: Point::new(-9, -9),   down: false, wheel:  0.0, settle: 90 },
        Scene { name: "demo-hover",      tab: 0, dark: false, mouse: Point::new(60, 98),   down: false, wheel:  0.0, settle: 90 },
        Scene { name: "demo-press",      tab: 0, dark: false, mouse: Point::new(60, 98),   down: true,  wheel:  0.0, settle: 90 },
        Scene { name: "demo-scroll", tab: 1, dark: false, mouse: Point::new(-9, -9),   down: false, wheel:  0.0, settle: 60 },
        Scene { name: "demo-scroll-mid", tab: 1, dark: false, mouse: Point::new(180, 120), down: false, wheel: -9.0, settle: 90 },
        Scene { name: "demo-palette",    tab: 2, dark: false, mouse: Point::new(-9, -9),   down: false, wheel:  0.0, settle: 40 },
        Scene { name: "demo-about",      tab: 3, dark: false, mouse: Point::new(-9, -9),   down: false, wheel:  0.0, settle: 40 },
        Scene { name: "demo-midnight",   tab: 1, dark: true,  mouse: Point::new(180, 120), down: false, wheel: -5.0, settle: 90 },
    ];

    std::fs::create_dir_all("screenshots")?;

    for scene in &scenes {
        let mut canvas = Canvas::new(384, 240);
        let mut ui_state = UiState::new();
        let mut theme = theme();
        let mut state = Demo {
            tab: scene.tab,
            dark: scene.dark,
            ..Default::default()
        };
        if scene.dark {
            theme = Theme::midnight();
            theme.scanline = 0.0;
        }
        // A fixed progress value keeps the output byte-stable between runs.
        state.autorun = false;
        state.progress = 0.62;

        let mut input = Input {
            mouse_in_window: true,
            // Ask the toolkit for its own pointer, as the backend does.
            draw_pointer: true,
            dt: 1.0 / 60.0,
            ..Default::default()
        };

        for f in 0..scene.settle {
            input.time = f as f32 / 60.0;
            input.mouse = scene.mouse;
            // Only the first frame carries the press edge; after that the
            // button is simply held, which is what a real drag looks like.
            input.mouse_pressed = scene.down && f == scene.settle / 2;
            input.mouse_down = scene.down && f >= scene.settle / 2;
            input.mouse_released = false;
            // Deliver the whole wheel gesture on one frame, then let the
            // scroll spring settle over the frames that follow.
            input.wheel = if f == 10 { scene.wheel } else { 0.0 };

            canvas.clear(theme.background);
            let out = {
                let mut ui = Ui::begin(&mut canvas, &input, &theme, &mut ui_state);
                frame(&mut ui, &mut state);
                ui.finish()
            };
            if let Some(t) = out.theme {
                theme = t;
            }
            input.begin_frame();
        }

        let path = format!("screenshots/{}.ppm", scene.name);
        write_ppm(&path, &canvas, 3)?;
        println!("wrote {path}");
    }

    Ok(())
}

/// Dump the canvas as a binary PPM, nearest-neighbour scaled by `scale`.
///
/// PPM because it is nine lines of code and needs no image crate; the point is
/// to look at the pixels, not to ship a codec.
fn write_ppm(path: &str, canvas: &Canvas, scale: usize) -> std::io::Result<()> {
    let w = canvas.width() as usize;
    let h = canvas.height() as usize;
    let mut out = BufWriter::new(File::create(path)?);
    write!(out, "P6\n{} {}\n255\n", w * scale, h * scale)?;

    let px = canvas.pixels();
    let mut row = vec![0u8; w * scale * 3];
    for y in 0..h {
        for x in 0..w {
            let c = px[y * w + x];
            let rgb = [(c >> 16) as u8, (c >> 8) as u8, c as u8];
            for s in 0..scale {
                let i = (x * scale + s) * 3;
                row[i..i + 3].copy_from_slice(&rgb);
            }
        }
        for _ in 0..scale {
            out.write_all(&row)?;
        }
    }
    out.flush()
}
