//! Where does the frame time actually go?
//!
//! Splits the per-frame cost into the two halves that matter: rasterising the
//! UI at virtual resolution, and blowing that up to the physical window. Run
//! with `cargo run --release -p pixui-demo --example bench`.

use std::time::Instant;

use demo::{frame, theme, Demo};
use pixui::app::blit;
use pixui::{palette, Canvas, Input, Point, Ui, UiState};

const ITERS: u32 = 600;

fn main() {
    let theme = theme();
    let mut canvas = Canvas::new(384, 240);
    let mut ui_state = UiState::new();
    let mut state = Demo::default();
    let mut input = Input {
        mouse_in_window: true,
        dt: 1.0 / 60.0,
        mouse: Point::new(60, 98),
        ..Default::default()
    };

    // --- half one: build and rasterise the whole UI ------------------------
    let t0 = Instant::now();
    for i in 0..ITERS {
        input.time = i as f32 / 60.0;
        canvas.clear(theme.background);
        let mut ui = Ui::begin(&mut canvas, &input, &theme, &mut ui_state);
        frame(&mut ui, &mut state);
        ui.finish();
        input.begin_frame();
    }
    let ui_ms = t0.elapsed().as_secs_f64() * 1000.0 / ITERS as f64;

    // --- half two: upscale to a HiDPI-sized window ------------------------
    for (label, win_w, win_h, scale) in [
        ("1152x720  (3x)", 1152usize, 720usize, 3usize),
        ("2304x1440 (6x)", 2304, 1440, 6),
    ] {
        let mut dst = vec![0u32; win_w * win_h];
        let mut scratch = Vec::new();
        let t = Instant::now();
        for _ in 0..ITERS {
            blit(
                &canvas,
                &mut scratch,
                &mut dst,
                win_w,
                win_h,
                scale,
                (0, 0),
                palette::VOID,
            );
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / ITERS as f64;
        let mpx = (win_w * win_h) as f64 / 1e6;
        println!(
            "blit {label}  {ms:6.3} ms/frame   {mpx:.1} Mpx   {:.1} Mpx/s",
            mpx / (ms / 1000.0)
        );
    }

    println!("ui   384x240        {ui_ms:6.3} ms/frame   0.09 Mpx");
    println!();
    println!("frame budget: 16.7 ms at 60 fps, 8.3 ms at 120 fps");
    println!("(the app follows the display's refresh rate; override with Config::with_fps)");
}
