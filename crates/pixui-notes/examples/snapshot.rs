//! Render the notes app to image files with no window and no event loop.
//!
//! Same trick as the other demo: the toolkit's frame lifecycle is public, so
//! the whole application can be driven from a synthetic [`Input`]. That makes
//! a modal editor genuinely testable — you can type `dw`, capture the frame,
//! and diff the pixels.
//!
//! Run with: `cargo run -p pixui-notes --example snapshot`

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use pixui::{Canvas, Input, Key, Point, Theme, Ui, UiState};
use pixui_notes::{frame, theme, Notes};

/// One capture: a name, keys to type, and how long to let things settle.
struct Scene {
    name: &'static str,
    /// Typed one per frame, so vim's pending-key parsing runs for real.
    script: Vec<Key>,
    mouse: Point,
    settle: u32,
    /// Canvas size. Under `Scaling::Adaptive` this is what a resized window
    /// produces, so varying it here shows exactly what resizing does.
    canvas: (i32, i32),
}

/// Turn a string into keystrokes, mapping space and newline to their keys.
fn keys(s: &str) -> Vec<Key> {
    s.chars()
        .map(|c| match c {
            ' ' => Key::Space,
            '\n' => Key::Enter,
            '\x1b' => Key::Escape,
            c => Key::Char(c),
        })
        .collect()
}

fn main() -> std::io::Result<()> {
    let dir = PathBuf::from("target/notes-snapshot");
    // Start from a clean vault so captures are byte-stable between runs.
    let _ = std::fs::remove_dir_all(&dir);

    let scenes = vec![
        Scene {
            name: "01-editor",
            script: vec![],
            mouse: Point::new(-9, -9),
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "02-insert",
            script: [keys("Go"), keys("A new thought, typed in insert mode.")].concat(),
            mouse: Point::new(-9, -9),
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "03-visual",
            script: keys("jjjvwwwl"),
            mouse: Point::new(-9, -9),
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "04-command",
            script: keys(":w notes-are-files"),
            mouse: Point::new(-9, -9),
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "05-open-dialog",
            script: [keys(":e"), vec![Key::Enter], keys("jj")].concat(),
            mouse: Point::new(-9, -9),
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "06-save-dialog",
            script: [keys(":new"), vec![Key::Enter], keys(":w"), vec![Key::Enter]].concat(),
            mouse: Point::new(-9, -9),
            settle: 40,
            canvas: (768, 470),
        },
        // The same app on a larger window. Under `Scaling::Adaptive` a resize
        // produces exactly this: the same pixel size, with more room in it.
        Scene {
            name: "07-resized",
            script: vec![],
            mouse: Point::new(-9, -9),
            settle: 40,
            canvas: (1050, 620),
        },
        // A text object mid-flight: `ci"` inside a quoted span.
        Scene {
            name: "08-text-object",
            script: [keys("GA \"quoted words\" here\x1b"), keys("hhhhhhci\"")].concat(),
            mouse: Point::new(-9, -9),
            settle: 30,
            canvas: (768, 470),
        },
    ];

    std::fs::create_dir_all("snapshots")?;

    for scene in &scenes {
        // A fresh vault and a fresh UI per scene keeps them independent.
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = Notes::open(dir.clone());
        let mut canvas = Canvas::new(scene.canvas.0, scene.canvas.1);
        let mut ui_state = UiState::new();
        let theme: Theme = theme();
        let mut input = Input {
            mouse_in_window: true,
            dt: 1.0 / 60.0,
            ..Default::default()
        };

        let total = 5 + scene.script.len() as u32 + scene.settle;
        for f in 0..total {
            input.time = f as f32 / 60.0;
            input.mouse = scene.mouse;
            input.keys.clear();
            if f >= 5 {
                if let Some(key) = scene.script.get((f - 5) as usize) {
                    input.keys.push(*key);
                }
            }

            canvas.clear(theme.background);
            {
                let mut ui = Ui::begin(&mut canvas, &input, &theme, &mut ui_state);
                frame(&mut ui, &mut app);
                ui.finish();
            }
            input.begin_frame();
        }

        let path = format!("snapshots/notes-{}.ppm", scene.name);
        write_ppm(&path, &canvas, 2)?;
        println!("wrote {path}");
    }

    Ok(())
}

/// Dump the canvas as a binary PPM, nearest-neighbour scaled by `scale`.
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
