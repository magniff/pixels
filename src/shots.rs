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

use crate::{frame, theme, Notes};
use pixui::{Canvas, Input, Key, Mods, Point, Theme, Ui, UiState};

/// One capture: a name, keys to type, and how long to let things settle.
struct Scene {
    name: &'static str,
    /// Typed one per frame, so vim's pending-key parsing runs for real.
    script: Vec<Press>,
    mouse: Point,
    /// Click at `mouse` before the script runs, for scenes that need focus
    /// somewhere first.
    click_first: bool,
    /// Make that opening click a double.
    double_click: bool,
    /// Press at the first point and drag to the second.
    drag: Option<(Point, Point)>,
    /// Click at `mouse` once the script has finished typing, for a control
    /// that only exists as a result of what was typed.
    click_after: bool,
    /// Notches of wheel, rolled at `mouse` for a few frames after the script.
    wheel: f32,
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

/// A Command chord (Control off macOS), for the pane and view shortcuts.
fn cmd(c: char) -> Vec<Press> {
    vec![Press {
        key: Key::Char(c),
        mods: Mods {
            cmd: true,
            ..Default::default()
        },
    }]
}

/// Tab, or Shift-Tab — neither of which a character can express.
fn tab(shift: bool) -> Vec<Press> {
    vec![Press {
        key: Key::Tab,
        mods: Mods {
            shift,
            ..Default::default()
        },
    }]
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

/// Render every scene into `screenshots/`.
pub fn run() -> std::io::Result<()> {
    let dir = PathBuf::from("target/notes-snapshot");
    // Start from a clean vault so captures are byte-stable between runs.
    let _ = std::fs::remove_dir_all(&dir);

    let scenes = vec![
        Scene {
            name: "editor",
            script: vec![],
            mouse: Point::new(430, 120),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "insert",
            script: [keys("Go"), keys("A new thought, typed in insert mode.")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "visual",
            script: keys("jjjvwwwl"),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "command",
            script: keys(":w notes-are-files"),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "dialog-open",
            script: [keys(":e"), keys("\n"), keys("jj")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "dialog-save",
            script: [keys(":new"), keys("\n"), keys(":w"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 40,
            canvas: (768, 470),
        },
        // The same app on a larger window. Under `Scaling::Adaptive` a resize
        // produces exactly this: the same pixel size, with more room in it.
        Scene {
            name: "resized",
            script: vec![],
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 40,
            canvas: (1050, 620),
        },
        // Search hits highlighted, with the pointer over the divider.
        Scene {
            name: "search",
            script: keys("/pixui\n"),
            mouse: Point::new(153, 200),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // Typing in the sidebar filter narrows the list live.
        Scene {
            name: "filter",
            script: keys("export"),
            mouse: Point::new(80, 44),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // The rendered view of the welcome note.
        Scene {
            name: "preview",
            script: [keys(":preview"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // Hovering a link in the rendering: it lights up and the pointer
        // turns, because clicking it goes somewhere.
        Scene {
            name: "link-hover",
            script: [keys(":preview"), keys("\n")].concat(),
            mouse: Point::new(220, 197),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // The moment cmd-N lands: a ring around the list that has just taken
        // the keyboard, on its way out again.
        Scene {
            name: "pane-flare",
            script: cmd('n'),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 2,
            canvas: (768, 470),
        },
        // Cmd-N hands the keyboard to the note list, which says so with a
        // marching ring on the row j and k will move.
        Scene {
            name: "pane-notes",
            script: [cmd('n'), keys("jj")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 20,
            canvas: (768, 470),
        },
        // `o` on a list item opens the next one already marked up, and Tab
        // takes it a level in.
        Scene {
            name: "auto-indent",
            script: [
                keys(":e ideas.md"),
                keys("\n"),
                keys("jjjo"),
                keys("and a nested one"),
                tab(false),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 20,
            canvas: (768, 470),
        },
        // The mark that appears beside a selection: the assistant, offering.
        Scene {
            name: "assist-mark",
            script: [keys(":e ideas.md"), keys("\n"), keys("jjVj")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 20,
            canvas: (768, 470),
        },
        // Clicking the mark, which is how the assistant is usually reached.
        Scene {
            name: "assist-open",
            script: [keys(":e ideas.md"), keys("\n"), keys("jjVj")].concat(),
            mouse: Point::new(745, 72),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: true,
            settle: 20,
            canvas: (768, 470),
        },
        // Asked, answered, and waiting to be kept or thrown away. The line is
        // typed in first so there is something for the rehearsal backend to
        // actually fix, and the diff has both colours in it.
        Scene {
            name: "assist-diff",
            script: [
                keys(":e ideas.md"),
                keys("\n"),
                keys("Go"),
                keys("teh quick brown fox jumped  over teh lazy dog"),
                keys("\x1b"),
                keys("V"),
                ctrl('a'),
                keys("fix the typos"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // Enter in the search box hands the keyboard to the results, on the
        // first of them.
        Scene {
            name: "search-enter",
            script: [cmd('s'), keys("vim"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 20,
            canvas: (768, 470),
        },
        // Caught mid-transition: the two views dissolving into each other.
        Scene {
            name: "tab-fade",
            script: vec![],
            mouse: Point::new(300, 25),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 7,
            canvas: (768, 470),
        },
        // The showcase note, in both views.
        Scene {
            name: "showcase-source",
            script: [keys(":e markdown-showcase.md"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (900, 660),
        },
        Scene {
            name: "showcase-preview",
            script: [
                keys(":e markdown-showcase.md"),
                keys("\n"),
                keys(":preview"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (900, 660),
        },
        // The wheel over the source pane, which moves the view and takes the
        // caret with it rather than being snapped back by it.
        Scene {
            name: "wheel-scroll",
            script: [keys(":e markdown-showcase.md"), keys("\n")].concat(),
            mouse: Point::new(600, 300),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: -1.0,
            click_after: false,
            settle: 30,
            canvas: (900, 660),
        },
        // `/` in the preview is vim's search: it finds the line in the source,
        // scrolls to the block that line was parsed into, and lights up every
        // hit in the rendered text.
        Scene {
            name: "preview-search",
            script: [
                keys(":e markdown-showcase.md"),
                keys("\n"),
                keys(":preview"),
                keys("\n"),
                keys("/alignment"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 40,
            canvas: (900, 660),
        },
        // The preview taking the vim motions that move a page: `G` to the end
        // of the document, where the gutter is numbered with the source lines
        // the blocks down there came from.
        Scene {
            name: "preview-scroll",
            script: [
                keys(":e markdown-showcase.md"),
                keys("\n"),
                keys(":preview"),
                keys("\n"),
                keys("G"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 40,
            canvas: (900, 660),
        },
        // The rendered view of the note with a table and a code block.
        Scene {
            name: "preview-table",
            script: [
                keys(":e vim-keys.md"),
                keys("\n"),
                keys(":preview"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // Task list items and a fenced code block.
        Scene {
            name: "preview-tasks",
            script: [
                keys(":e ideas.md"),
                keys("\n"),
                keys(":preview"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // Double-clicking a note in the drawer renames it in place.
        Scene {
            name: "rename",
            script: keys("about-the-toolkit"),
            mouse: Point::new(80, 118),
            click_first: true,
            double_click: true,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // Dragging in the editor selects, as a mouse-driven visual mode.
        Scene {
            name: "drag-select",
            script: vec![],
            mouse: Point::new(340, 43),
            click_first: false,
            double_click: false,
            drag: Some((Point::new(200, 43), Point::new(340, 43))),
            wheel: 0.0,
            click_after: false,
            settle: 20,
            canvas: (768, 470),
        },
        // A blockwise selection over the list items.
        Scene {
            name: "visual-block",
            script: [keys("11G"), ctrl('v'), keys("jjj"), keys("llllllll")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
        // A text object mid-flight: `ci"` inside a quoted span.
        Scene {
            name: "text-object",
            script: [keys("GA \"quoted words\" here\x1b"), keys("hhhhhhci\"")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            click_after: false,
            settle: 30,
            canvas: (768, 470),
        },
    ];

    std::fs::create_dir_all("screenshots")?;

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

        // Frames 2 and 3 are the optional click; typing starts at 5.
        let total = 8 + scene.script.len() as u32 + scene.settle;
        for f in 0..total {
            input.time = f as f32 / 60.0;
            input.mouse = scene.mouse;
            input.keys.clear();
            input.wheel = 0.0;
            input.mods = Mods::default();
            input.mouse_pressed = false;
            input.mouse_released = false;
            if scene.click_first {
                match f {
                    2 => {
                        input.mouse_down = true;
                        input.mouse_pressed = true;
                    }
                    3 => {
                        input.mouse_down = false;
                        input.mouse_released = true;
                    }
                    // A second press and release, close enough in time to
                    // register as a double.
                    4 if scene.double_click => {
                        input.mouse_down = true;
                        input.mouse_pressed = true;
                    }
                    _ => {}
                }
                if f == 5 && scene.double_click {
                    input.mouse_down = false;
                    input.mouse_released = true;
                }
            }
            if let Some((from, to)) = scene.drag {
                match f {
                    2 => {
                        input.mouse = from;
                        input.mouse_down = true;
                        input.mouse_pressed = true;
                    }
                    3..=6 => {
                        let t = (f - 2) as f32 / 4.0;
                        input.mouse = Point::new(
                            from.x + ((to.x - from.x) as f32 * t) as i32,
                            from.y + ((to.y - from.y) as f32 * t) as i32,
                        );
                        input.mouse_down = true;
                    }
                    7 => {
                        input.mouse = to;
                        input.mouse_down = false;
                        input.mouse_released = true;
                    }
                    _ => input.mouse = to,
                }
            }
            let script_start = if scene.double_click { 7 } else { 5 };
            if f >= script_start {
                if let Some(press) = scene.script.get((f - script_start) as usize) {
                    input.keys.push(press.key);
                    input.mods = press.mods;
                }
            }
            let after = script_start + scene.script.len() as u32;
            if scene.click_after {
                if f == after + 1 {
                    input.mouse_down = true;
                    input.mouse_pressed = true;
                } else if f == after + 2 {
                    input.mouse_down = false;
                    input.mouse_released = true;
                }
            }
            // Five notches, once the script has finished typing.
            let wheel_from = script_start + scene.script.len() as u32;
            if scene.wheel != 0.0 && (wheel_from..wheel_from + 5).contains(&f) {
                input.wheel = scene.wheel;
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

        let path = format!("screenshots/{}.ppm", scene.name);
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
