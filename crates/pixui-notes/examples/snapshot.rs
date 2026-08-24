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

use pixui::{Canvas, Input, Key, Mods, Point, Theme, Ui, UiState};
use pixui_notes::{frame, theme, Notes};

/// One capture: a name, keys to type, and how long to let things settle.
struct Scene {
    name: &'static str,
    /// Typed one per frame, so vim's pending-key parsing runs for real.
    script: Vec<Press>,
    mouse: Point,
    settle: u32,
    /// Canvas size. Under `Scaling::Adaptive` this is what a resized window
    /// produces, so varying it here shows exactly what resizing does.
    canvas: (i32, i32),
}

/// One keystroke, with whatever modifiers were held for it.
#[derive(Clone, Copy)]
struct Press {
    key: Key,
    mods: Mods,
}

/// Turn a string into plain keystrokes, mapping space, newline and escape.
fn keys(s: &str) -> Vec<Press> {
    s.chars()
        .map(|c| Press {
            key: match c {
                ' ' => Key::Space,
                '\n' => Key::Enter,
                '\x1b' => Key::Escape,
                c => Key::Char(c),
            },
            mods: Mods::default(),
        })
        .collect()
}

/// A Ctrl chord, which a bare character cannot express.
fn ctrl(c: char) -> Vec<Press> {
    vec![Press {
        key: Key::Char(c),
        mods: Mods {
            ctrl: true,
            ..Default::default()
        },
    }]
}

fn main() -> std::io::Result<()> {
    let dir = PathBuf::from("target/notes-snapshot");
    // Start from a clean vault so captures are byte-stable between runs.
    let _ = std::fs::remove_dir_all(&dir);

    let scenes = vec![
        Scene {
            name: "01-editor",
            script: vec![],
            mouse: Point::new(430, 120),
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
            script: [keys(":e"), keys("\n"), keys("jj")].concat(),
            mouse: Point::new(-9, -9),
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "06-save-dialog",
            script: [keys(":new"), keys("\n"), keys(":w"), keys("\n")].concat(),
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
        // Search hits highlighted, with the pointer over the divider.
        Scene {
            name: "10-search",
            script: keys("/pixui\n"),
            mouse: Point::new(153, 200),
            settle: 30,
            canvas: (768, 470),
        },
        // A blockwise selection over the list items.
        Scene {
            name: "09-visual-block",
            script: [keys("11G"), ctrl('v'), keys("jjj"), keys("llllllll")].concat(),
            mouse: Point::new(-9, -9),
            settle: 30,
            canvas: (768, 470),
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
            // Ask the toolkit for its own pointer, as the backend does.
            draw_pointer: true,
            dt: 1.0 / 60.0,
            ..Default::default()
        };

        let total = 5 + scene.script.len() as u32 + scene.settle;
        for f in 0..total {
            input.time = f as f32 / 60.0;
            input.mouse = scene.mouse;
            input.keys.clear();
            input.mods = Mods::default();
            if f >= 5 {
                if let Some(press) = scene.script.get((f - 5) as usize) {
                    input.keys.push(press.key);
                    input.mods = press.mods;
                }
            }

            canvas.clear(theme.background);
            // The whole frame, exactly as the backend runs it. `Ui::finish`
            // applies the toolkit's own post-frame passes — the scanlines and
            // the drawn pointer — so there is nothing to reimplement here.
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
