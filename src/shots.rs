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
    /// Clicks made once the script has finished typing, in order, a few frames
    /// apart. For the controls that only exist as a result of what came before
    /// them: a menu entry, then the panel it opened, then the button on it.
    clicks: Vec<Point>,
    /// Keys typed after those clicks, for what the clicks opened.
    then: Vec<Press>,
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

/// Ctrl with a key that is not a character.
fn ctrl_key(key: Key) -> Vec<Press> {
    vec![Press {
        key,
        mods: Mods {
            ctrl: true,
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![Point::new(745, 72)],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Keeping the suggestion: the same scene as above, with APPLY pressed.
        // Line 17 is the answer, and the block has closed behind it.
        Scene {
            name: "assist-applied",
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
            mouse: Point::new(650, 227),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![Point::new(650, 227)],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // The application menu, open.
        Scene {
            name: "menu",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![],
            then: vec![],
            settle: 10,
            canvas: (768, 470),
        },
        // What the app is, and which build this is.
        Scene {
            name: "about",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![Point::new(64, 33)],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        // Which weights to run, and what to tell them.
        Scene {
            name: "settings",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![Point::new(64, 21), Point::new(383, 222)],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        // The prompt is a small instance of the same editor the notes use: the
        // same vim grammar, and it scrolls the same way.
        Scene {
            name: "settings-vim",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![Point::new(64, 21), Point::new(383, 222)],
            then: [keys("GA"), keys(" AND A LINE OF MY OWN.")].concat(),
            settle: 12,
            canvas: (768, 470),
        },
        // The settings, as they open: a list of what can be set.
        Scene {
            name: "settings-index",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![Point::new(64, 21)],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        // Switched off: what is under the switch stays legible and stops
        // answering the pointer.
        Scene {
            name: "settings-off",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 222),
                Point::new(241, 172),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        // And with it off, a selection is just a selection: no mark, nothing
        // offering to rewrite it.
        Scene {
            name: "assist-off",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 222),
                Point::new(241, 172),
                Point::new(503, 325),
            ],
            then: keys("jjVj"),
            settle: 12,
            canvas: (768, 470),
        },
        // And closing it again with the button, which is the click that has to
        // reach a panel drawn over everything else.
        Scene {
            name: "settings-closed",
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![Point::new(64, 21), Point::new(503, 308)],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        // Keeping the suggestion without reaching for the mouse.
        Scene {
            name: "assist-kept",
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
                ctrl_key(Key::Enter),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            clicks: vec![],
            then: vec![],
            settle: 20,
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
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
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
    ];

    std::fs::create_dir_all("screenshots")?;

    // Settings and installed weights are the user's, and a screenshot must not
    // depend on either: point both somewhere empty so every run of this draws
    // the same thing on every machine.
    let scratch = std::env::temp_dir().join("pixui-shots");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(scratch.join("models"))?;
    std::env::set_var("PIXUI_CONFIG", scratch.join("settings.conf"));
    std::env::set_var("PIXUI_MODELS", scratch.join("models"));

    for scene in &scenes {
        // A fresh vault, a fresh UI and fresh settings per scene keeps them
        // independent: one scene that changes a setting must not turn up in
        // the next one's screenshot.
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(scratch.join("settings.conf"));
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
        let clicking = scene.clicks.len() as u32 * 4;
        let total =
            8 + scene.script.len() as u32 + clicking + scene.then.len() as u32 + scene.settle;
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
            // Late enough that anything the script asked for has come back:
            // the control being clicked may not exist until it has.
            // Whatever the clicks opened, typed into. Starts once the last of
            // them has been released.
            let typing = script_start + scene.script.len() as u32 + 7 + clicking;
            if f >= typing {
                if let Some(press) = scene.then.get((f - typing) as usize) {
                    input.keys.push(press.key);
                    input.mods = press.mods;
                }
            }
            // Four frames apart, and late enough that anything the script
            // asked for has come back: the control being clicked may not exist
            // until it has.
            let after = script_start + scene.script.len() as u32 + 5;
            for (i, at) in scene.clicks.iter().enumerate() {
                let when = after + 1 + i as u32 * 4;
                if f >= when {
                    input.mouse = *at;
                }
                if f == when {
                    input.mouse_down = true;
                    input.mouse_pressed = true;
                } else if f == when + 1 {
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
