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
    /// Files written into the vault before the frames run, by path relative to
    /// it. For the parts of the app that draw what is on disk rather than what
    /// was typed - a conversation had yesterday has to have been had.
    seed: &'static [(&'static str, &'static str)],
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
    /// A press of the other mouse button, made just before the clicks below:
    /// for the menus that only exist once something has been asked about.
    right_click: Option<Point>,
    /// Clicks made once the script has finished typing, in order, a few frames
    /// apart. For the controls that only exist as a result of what came before
    /// them: a menu entry, then the panel it opened, then the button on it.
    clicks: Vec<Point>,
    /// Keys typed after those clicks, for what the clicks opened.
    then: Vec<Press>,
    /// Notches of wheel, rolled at `mouse` for a few frames after the script.
    wheel: f32,
    /// Where the pointer goes before the wheel rolls, for a view that has to be
    /// opened by a click somewhere else first. Without it the pointer is left
    /// wherever the last click put it, and a click is not a hover: it moves the
    /// caret, which is the very thing some of these scenes are about.
    hover: Option<Point>,
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

/// The primary modifier with a key that is not a character.
fn cmd_key(key: Key) -> Vec<Press> {
    vec![Press {
        key,
        mods: Mods {
            cmd: true,
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
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(430, 120),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "insert",
            seed: &[],
            right_click: None,
            script: [keys("Go"), keys("A new thought, typed in insert mode.")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "visual",
            seed: &[],
            right_click: None,
            script: keys("jjjvwwwl"),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "command",
            seed: &[],
            right_click: None,
            script: keys(":w notes-are-files"),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "dialog-open",
            seed: &[],
            right_click: None,
            script: [keys(":e"), keys("\n"), keys("jj")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 40,
            canvas: (768, 470),
        },
        Scene {
            name: "dialog-save",
            seed: &[],
            right_click: None,
            script: [keys(":new"), keys("\n"), keys(":w"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 40,
            canvas: (768, 470),
        },
        // The same app on a larger window. Under `Scaling::Adaptive` a resize
        // produces exactly this: the same pixel size, with more room in it.
        Scene {
            name: "resized",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 40,
            canvas: (1050, 620),
        },
        // Search hits highlighted, with the pointer over the divider.
        Scene {
            // Cmd-P, and a few letters of the note wanted.
            name: "finder",
            seed: &[],
            right_click: None,
            script: [
                keys(":e welcome.md"),
                keys("\n"),
                cmd_key(Key::Char('p')),
                keys("m"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            // A pattern half typed: vim lights the hits before you commit.
            name: "incsearch",
            seed: &[],
            right_click: None,
            script: [keys(":e markdown-showcase.md"), keys("\n"), keys("/ital")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            // The conversations already had about this note, when there are any.
            name: "chat-picker",
            seed: &[
                (
                    ".chats/what-the-toolkit-is.md",
                    "# what the toolkit is\n\n## you\n\nwhat is the toolkit called\n\n## assistant\n\nIt is called pixui.\n",
                ),
                (
                    ".chats/whether-to-wrap-long-lines.md",
                    "# whether to wrap long lines\n\n## you\n\nshould long lines wrap or scroll\n\n## assistant\n\nWrap. A note is prose.\n\n## you\n\nwhat about code fences\n\n## assistant\n\nThose scroll.\n",
                ),
            ],
            right_click: None,
            script: [keys(":e welcome.md"), keys("\n"), cmd_key(Key::Enter)].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            // The bin on a row asks before it throws a conversation away, and the
            // row underneath it does not take the click on the way past.
            name: "chat-bin",
            seed: &[
                (
                    ".chats/what-the-toolkit-is.md",
                    "# what the toolkit is\n\n## you\n\nwhat is the toolkit called\n\n## assistant\n\nIt is called pixui.\n",
                ),
                (
                    ".chats/whether-to-wrap-long-lines.md",
                    "# whether to wrap long lines\n\n## you\n\nshould long lines wrap or scroll\n\n## assistant\n\nWrap. A note is prose.\n\n## you\n\nwhat about code fences\n\n## assistant\n\nThose scroll.\n",
                ),
            ],
            right_click: None,
            script: [keys(":e welcome.md"), keys("\n"), cmd_key(Key::Enter)].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(590, 221)],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            // One of them opened, with an answer in it that has markdown in it.
            name: "chat-talk",
            right_click: None,
            seed: &[(
                ".chats/how-does-wrapping-work.md",
                "# how does wrapping work\n\n## you\n\nhow does wrapping work in the source view\n\n## assistant\n\nAt the width of the pane, on spaces, with two rules:\n\n- a word longer than the line is broken rather than pushed out\n- a wrapped row is marked in the gutter so it is not read as a new line\n\nThe wrapping is computed from the text and the styled runs are sliced to\nmatch, so `wrap_ranges` and the highlighter cannot disagree.\n\n## you\n\nand in the preview\n\n## assistant\n\nSame width, but per block: a paragraph is one block however many rows it\ntakes.\n",
            )],
            script: [
                keys(":e welcome.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            // A half-typed command, with what it could still become.
            name: "chat-command",
            right_click: None,
            seed: &[(
                ".chats/how-does-wrapping-work.md",
                "# how does wrapping work\n\n## you\n\nhow does wrapping work in the source view\n\n## assistant\n\nAt the width of the pane, on spaces. A word longer than the line is broken\nrather than pushed out.\n",
            )],
            script: [
                keys(":e welcome.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("\n"),
                keys("/"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            // A change the model offered, with the diff it would make.
            name: "chat-settled",
            right_click: None,
            seed: &[(
                ".chats/tighten-the-opening.md",
                "# tighten the opening\n\n## you\n\nline 6 is doing too much, split it\n\n## assistant\n\nIt runs three claims together. Split at the colon and let the second half stand on its own.\n\n<edit lines=\"6-6\" state=\"applied\">\nEverything you can see is a pixel buffer.\nThe sidebar, the caret and the save dialog are all drawn the same way.\n</edit>\n\n## you\n\nand line 7\n\n## assistant\n\nThat one is fine as it stands.\n",
            )],
            script: [
                keys(":e welcome.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            name: "backlinks",
            right_click: None,
            seed: &[
                ("aquarium/water.md", "# Water\n\nNitrate under 20, nitrite zero, ammonia zero. Test on Sundays before the\nwater change so the numbers are always from the same point in the week.\n\nTwenty five percent out every week, replaced with water that has stood for a\nday.\n"),
                ("aquarium/stock.md", "# Stock\n\nThe amano shrimp keep dying, and I think it is the change rather than\nanything in the tank. See [the water note](water.md).\n"),
                ("aquarium/plants.md", "# Plants\n\nFerts twice a week, half dose. Full dose grew algae faster than plants.\nStanding [[water]] is warmer than it looks.\n"),
            ],
            script: [keys(":e water.md"), keys("\n"), keys(":preview"), keys("\n"), keys("G")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "chat-web",
            right_click: None,
            seed: &[
                ("aquarium/water.md", "# Water\n\nNitrate under 20, nitrite zero, ammonia zero.\n"),
                (".chats/aquarium/is-it-warm-enough.md", "# is it warm enough\n\n## you\n\nis it warm enough outside to do the water change today?\n\n## assistant\n\n<used tool=\"weather\" arg=\"Berlin\">\nBerlin right now: 23C, Overcast, feels like 19C, wind 16 km/h, humidity 37%\n</used>\n\nYes - 23C and overcast, so the fresh water will not be far off tank temperature. Overcast is better than sun for it: standing water in a bucket in direct sun climbs faster than you would think.\n"),
            ],
            script: [
                keys(":e water.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            name: "chat-merge",
            seed: &[
                ("reading/queue.md", "# Queue\n\n- [ ] Invisible Cities - Calvino\n- [x] A Pattern Language - Alexander\n"),
                ("reading/patterns.md", "# Patterns\n\nNotes on A Pattern Language, which I finished in March.\n"),
                (".chats/reading/two-notes-one-subject.md", "# two notes one subject\n\n## you\n\nthese two overlap - fold patterns.md into queue.md\n\n## assistant\n\nThe queue already marks the book done, so the notes belong under it.\n\n<merge into=\"queue.md\" from=\"queue.md, patterns.md\">\n# Queue\n\n- [ ] Invisible Cities - Calvino\n- [x] A Pattern Language - Alexander\n\n## On A Pattern Language\n\nFinished in March.\n</merge>\n"),
            ],
            right_click: None,
            script: [
                keys(":e queue.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            name: "chat-project",
            seed: &[
                ("pixel-editor/rendering.md", "# Rendering\n\nEverything is drawn by hand into a `Vec<u32>`. No GPU canvas underneath and no\nsystem widget anywhere.\n\nA frame costs about a third of a millisecond.\n"),
                ("pixel-editor/fonts.md", "# Fonts\n\nFive bitmap faces, chosen in settings.\n"),
                (".chats/pixel-editor/a-glossary.md", "# a glossary\n\n## you\n\nmake a glossary for this project\n\n## assistant\n\nThree terms are used across both files without being defined anywhere.\n\n<write file=\"glossary.md\">\n# Glossary\n\n- **Bitmap face** - a font stored as a table of bits rather than as outlines\n- **Frame budget** - how long one redraw may take before it is felt\n</write>\n"),
            ],
            right_click: None,
            script: [
                keys(":e rendering.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            // A change the model offered, with the diff it would make.
            name: "chat-diff",
            right_click: None,
            seed: &[(
                ".chats/tighten-the-opening.md",
                "# tighten the opening\n\n## you\n\nline 6 is doing too much, split it\n\n## assistant\n\nIt runs three claims together. Split at the colon and let the second half stand on its own.\n\n<edit lines=\"6-6\">\nEverything you can see is a pixel buffer.\nThe sidebar, the caret and the save dialog are all drawn the same way.\n</edit>\n",
            )],
            script: [
                keys(":e welcome.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            // Throwing one away asks first.
            name: "chat-delete",
            seed: &[
                (
                    ".chats/what-the-toolkit-is.md",
                    "# what the toolkit is\n\n## you\n\nwhat is the toolkit called\n\n## assistant\n\nIt is called pixui.\n",
                ),
                (
                    ".chats/whether-to-wrap-long-lines.md",
                    "# whether to wrap long lines\n\n## you\n\nshould long lines wrap or scroll\n\n## assistant\n\nWrap. A note is prose.\n",
                ),
            ],
            right_click: None,
            script: [
                keys(":e welcome.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("j"),
                keys("d"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            // A conversation about the note, opened on nothing selected.
            name: "chat",
            seed: &[],
            right_click: None,
            script: [
                keys(":e welcome.md"),
                keys("\n"),
                cmd_key(Key::Enter),
                keys("what does this note say the toolkit is"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            // The other mouse button on a note: what can be done with it.
            name: "menu-note",
            seed: &[
                ("aquarium/water.md", "# Water\n\nNitrate under 20, nitrite zero, ammonia zero.\n"),
                ("aquarium/stock.md", "# Stock\n\n- 12 ember tetras\n- 6 corydoras pygmaeus\n"),
                ("bicycle/routes.md", "# Routes\n\nRiver loop, 32km, flat.\n"),
            ],
            right_click: Some(Point::new(80, 110)),
            script: vec![],
            mouse: Point::new(80, 110),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            // And on a project, which can also hold a new note.
            name: "menu-project",
            seed: &[
                ("aquarium/water.md", "# Water\n\nNitrate under 20, nitrite zero, ammonia zero.\n"),
                ("aquarium/stock.md", "# Stock\n\n- 12 ember tetras\n- 6 corydoras pygmaeus\n"),
                ("bicycle/routes.md", "# Routes\n\nRiver loop, 32km, flat.\n"),
            ],
            right_click: Some(Point::new(80, 92)),
            script: vec![],
            mouse: Point::new(80, 92),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            // Deleting a project asks first, and says what it would take.
            name: "menu-confirm",
            seed: &[
                ("aquarium/water.md", "# Water\n\nNitrate under 20, nitrite zero, ammonia zero.\n"),
                ("aquarium/stock.md", "# Stock\n\n- 12 ember tetras\n- 6 corydoras pygmaeus\n"),
                ("bicycle/routes.md", "# Routes\n\nRiver loop, 32km, flat.\n"),
            ],
            right_click: Some(Point::new(80, 92)),
            script: vec![],
            mouse: Point::new(80, 92),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(110, 128)],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            // The vault as a forest: a heading per project, its notes under it.
            name: "projects",
            seed: &[
                ("pixel-editor/rendering.md", "# Rendering\n\nEverything is drawn by hand into a `Vec<u32>`. No GPU canvas underneath and no\nsystem widget anywhere.\n"),
                ("pixel-editor/fonts.md", "# Fonts\n\nFive bitmap faces, chosen in settings. Four of them are set solid.\n"),
                ("pixel-editor/assistant.md", "# Assistant\n\nA quantised model on this machine, through llama.cpp. Nothing leaves the laptop.\n"),
                ("allotment/beds.md", "# Beds\n\nFour raised beds, one metre by three, on a four year rotation.\n"),
                ("allotment/watering.md", "# Watering\n\nEarly morning, at the root, never the leaves.\n"),
                ("reading/queue.md", "# Queue\n\n- [ ] Invisible Cities - Calvino\n- [x] A Pattern Language - Alexander\n"),
                ("reading/patterns.md", "# Patterns\n\nNotes on A Pattern Language, which I finished in March.\n"),
            ],
            right_click: None,
            script: vec![],
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        Scene {
            name: "search",
            seed: &[],
            right_click: None,
            script: keys("/pixui\n"),
            mouse: Point::new(153, 200),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Typing in the sidebar filter narrows the list live.
        Scene {
            name: "filter",
            seed: &[],
            right_click: None,
            script: keys("export"),
            mouse: Point::new(80, 44),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // The rendered view of the welcome note.
        Scene {
            name: "preview",
            seed: &[],
            right_click: None,
            script: [keys(":preview"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Hovering a link in the rendering: it lights up and the pointer
        // turns, because clicking it goes somewhere.
        Scene {
            name: "link-hover",
            seed: &[],
            right_click: None,
            script: [keys(":preview"), keys("\n")].concat(),
            mouse: Point::new(220, 197),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // The moment cmd-N lands: a ring around the list that has just taken
        // the keyboard, on its way out again.
        Scene {
            name: "pane-flare",
            seed: &[],
            right_click: None,
            script: cmd('n'),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 2,
            canvas: (768, 470),
        },
        // Cmd-N hands the keyboard to the note list, which says so with a
        // marching ring on the row j and k will move.
        Scene {
            name: "pane-notes",
            seed: &[],
            right_click: None,
            script: [cmd('n'), keys("jj")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        // `o` on a list item opens the next one already marked up, and Tab
        // takes it a level in.
        Scene {
            name: "auto-indent",
            seed: &[],
            right_click: None,
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
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        // The mark that appears beside a selection: the assistant, offering.
        Scene {
            name: "assist-mark",
            seed: &[],
            right_click: None,
            script: [keys(":e ideas.md"), keys("\n"), keys("jjVj")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        // Clicking the mark, which is how the assistant is usually reached.
        Scene {
            name: "assist-open",
            seed: &[],
            right_click: None,
            script: [keys(":e ideas.md"), keys("\n"), keys("jjVj")].concat(),
            mouse: Point::new(745, 72),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(745, 72)],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        // Asked, answered, and waiting to be kept or thrown away. The line is
        // typed in first so there is something for the rehearsal backend to
        // actually fix, and the diff has both colours in it.
        Scene {
            // A whole long note selected: the block belongs under the last
            // line, and the last line is the foot of the pane. It has to end
            // up somewhere you can read it.
            name: "assist-bottom",
            seed: &[],
            right_click: None,
            script: [
                keys(":e markdown-showcase.md"),
                keys("\n"),
                keys("ggVG"),
                cmd_key(Key::Enter),
                keys("tighten this"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        Scene {
            name: "assist-diff",
            seed: &[],
            right_click: None,
            script: [
                keys(":e ideas.md"),
                keys("\n"),
                keys("Go"),
                keys("teh quick brown fox jumped  over teh lazy dog"),
                keys("\x1b"),
                keys("V"),
                cmd_key(Key::Enter),
                keys("fix the typos"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Keeping the suggestion: the same scene as above, with APPLY pressed.
        // Line 17 is the answer, and the block has closed behind it.
        Scene {
            name: "assist-applied",
            seed: &[],
            right_click: None,
            script: [
                keys(":e ideas.md"),
                keys("\n"),
                keys("Go"),
                keys("teh quick brown fox jumped  over teh lazy dog"),
                keys("\x1b"),
                keys("V"),
                cmd_key(Key::Enter),
                keys("fix the typos"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(650, 227),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(650, 227)],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Caught mid-choice: the old highlight shrinking away and the new one
        // springing open under it.
        Scene {
            name: "note-pick",
            seed: &[],
            right_click: None,
            script: [cmd('n'), keys("j")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 4,
            canvas: (768, 470),
        },
        // The application menu, open.
        Scene {
            name: "menu",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 10,
            canvas: (768, 470),
        },
        // What the app is, and which build this is.
        Scene {
            name: "about",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(64, 33)],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        // Which weights to run, and what to tell them.
        Scene {
            name: "settings",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(64, 21), Point::new(383, 236)],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        // The prompt is a small instance of the same editor the notes use: the
        // same vim grammar, and it scrolls the same way.
        Scene {
            // The wheel over the system prompt. It used to chase the caret
            // straight back to the top; the view should stay where it was put.
            name: "settings-scroll",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: -2.0,
            // Hovered rather than clicked: a click in the prompt would move the
            // caret to the row under it, and the caret staying at the top while
            // the view is rolled away from it is the whole case.
            hover: Some(Point::new(400, 300)),
            clicks: vec![Point::new(64, 21), Point::new(383, 236)],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        Scene {
            name: "settings-vim",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(64, 21), Point::new(383, 236)],
            then: [keys("GA"), keys(" AND A LINE OF MY OWN.")].concat(),
            settle: 12,
            canvas: (768, 470),
        },
        // The colour schemes, each with a strip of its own colours.
        Scene {
            name: "appearance",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(64, 21), Point::new(383, 206)],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        // Walking the list with j, which wears each scheme as it goes.
        Scene {
            name: "appearance-walk",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(64, 21), Point::new(383, 206)],
            then: keys("jjj"),
            settle: 12,
            canvas: (768, 470),
        },
        // Read in: the whole app in another face. Every row, gutter and control
        // is sized from the line height, so this is the layout answering.
        Scene {
            name: "font-gohu",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 206),
                Point::new(567, 347),
                Point::new(558, 370),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            name: "font-creep",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 206),
                Point::new(567, 302),
                Point::new(558, 370),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            name: "font-tamzen",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 206),
                Point::new(567, 317),
                Point::new(558, 370),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            name: "font-cozette",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 206),
                Point::new(567, 332),
                Point::new(558, 370),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        // Worn: the whole app in somebody else's colours, dark and light.
        Scene {
            name: "scheme-nord",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 206),
                Point::new(565, 266),
                Point::new(558, 319),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        Scene {
            name: "scheme-latte",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 206),
                Point::new(565, 296),
                Point::new(558, 319),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        // The settings, as they open: a list of what can be set.
        Scene {
            name: "settings-index",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(64, 21)],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        // Switched off: what is under the switch stays legible and stops
        // answering the pointer.
        Scene {
            name: "settings-off",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 236),
                Point::new(250, 153),
            ],
            then: vec![],
            settle: 12,
            canvas: (768, 470),
        },
        // And with it off, a selection is just a selection: no mark, nothing
        // offering to rewrite it.
        Scene {
            name: "assist-off",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![
                Point::new(64, 21),
                Point::new(383, 236),
                Point::new(250, 153),
                Point::new(558, 344),
            ],
            then: keys("jjVj"),
            settle: 12,
            canvas: (768, 470),
        },
        // And closing it again with the button, which is the click that has to
        // reach a panel drawn over everything else.
        Scene {
            name: "settings-closed",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(30, 6),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![Point::new(64, 21), Point::new(558, 257)],
            then: vec![],
            settle: 14,
            canvas: (768, 470),
        },
        // Keeping the suggestion without reaching for the mouse.
        Scene {
            name: "assist-kept",
            seed: &[],
            right_click: None,
            script: [
                keys(":e ideas.md"),
                keys("\n"),
                keys("Go"),
                keys("teh quick brown fox jumped  over teh lazy dog"),
                keys("\x1b"),
                keys("V"),
                cmd_key(Key::Enter),
                keys("fix the typos"),
                keys("\n"),
                cmd_key(Key::Enter),
            ]
            .concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        // Enter in the search box hands the keyboard to the results, on the
        // first of them.
        Scene {
            name: "search-enter",
            seed: &[],
            right_click: None,
            script: [cmd('s'), keys("vim"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        // Caught mid-transition: the two views dissolving into each other.
        Scene {
            name: "tab-fade",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(300, 25),
            click_first: true,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 7,
            canvas: (768, 470),
        },
        // The showcase note, in both views.
        Scene {
            name: "showcase-source",
            seed: &[],
            right_click: None,
            script: [keys(":e markdown-showcase.md"), keys("\n")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (900, 660),
        },
        Scene {
            name: "showcase-preview",
            seed: &[],
            right_click: None,
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
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (900, 660),
        },
        // The wheel over the source pane, which moves the view and takes the
        // caret with it rather than being snapped back by it.
        Scene {
            // Rolled far past the end: the last line of the note should be
            // sitting on the last row of the pane, not fifteen rows above it.
            name: "source-bottom",
            seed: &[],
            right_click: None,
            script: [keys(":e markdown-showcase.md"), keys("\n")].concat(),
            mouse: Point::new(600, 300),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: -60.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (900, 660),
        },
        Scene {
            // Rolled far past the end of the rendered page, which should leave
            // its last line on the last row rather than somewhere above it.
            name: "preview-bottom",
            seed: &[],
            right_click: None,
            script: [
                keys(":e markdown-showcase.md"),
                keys("\n"),
                keys(":preview"),
                keys("\n"),
            ]
            .concat(),
            mouse: Point::new(600, 300),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: -60.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 40,
            canvas: (900, 660),
        },
        Scene {
            name: "wheel-scroll",
            seed: &[],
            right_click: None,
            script: [keys(":e markdown-showcase.md"), keys("\n")].concat(),
            mouse: Point::new(600, 300),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: -1.0,
            hover: None,
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
            seed: &[],
            right_click: None,
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
            hover: None,
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
            seed: &[],
            right_click: None,
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
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 40,
            canvas: (900, 660),
        },
        // The rendered view of the note with a table and a code block.
        Scene {
            name: "preview-table",
            seed: &[],
            right_click: None,
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
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Task list items and a fenced code block.
        Scene {
            name: "preview-tasks",
            seed: &[],
            right_click: None,
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
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Double-clicking a note in the drawer renames it in place.
        Scene {
            name: "rename",
            seed: &[],
            right_click: None,
            script: keys("about-the-toolkit"),
            mouse: Point::new(80, 118),
            click_first: true,
            double_click: true,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // Dragging in the editor selects, as a mouse-driven visual mode.
        Scene {
            name: "drag-select",
            seed: &[],
            right_click: None,
            script: vec![],
            mouse: Point::new(340, 43),
            click_first: false,
            double_click: false,
            drag: Some((Point::new(200, 43), Point::new(340, 43))),
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 20,
            canvas: (768, 470),
        },
        // A blockwise selection over the list items.
        Scene {
            name: "visual-block",
            seed: &[],
            right_click: None,
            script: [keys("11G"), ctrl('v'), keys("jjj"), keys("llllllll")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
            clicks: vec![],
            then: vec![],
            settle: 30,
            canvas: (768, 470),
        },
        // A text object mid-flight: `ci"` inside a quoted span.
        Scene {
            name: "text-object",
            seed: &[],
            right_click: None,
            script: [keys("GA \"quoted words\" here\x1b"), keys("hhhhhhci\"")].concat(),
            mouse: Point::new(-9, -9),
            click_first: false,
            double_click: false,
            drag: None,
            wheel: 0.0,
            hover: None,
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
        // Written before the vault opens, not after: a seeded note has to be
        // there to be read, and a seeded project is what stops the vault
        // seeding itself with the default ones instead.
        for (path, text) in scene.seed {
            let at = dir.join(path);
            if let Some(parent) = at.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(at, text);
        }
        let mut app = Notes::open(dir.clone());
        let mut canvas = Canvas::new(scene.canvas.0, scene.canvas.1);
        let mut ui_state = UiState::new();
        let mut theme: Theme = theme();
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
            input.right_pressed = false;
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
            // Two frames before the clicks, so whatever it opens is on screen
            // and hit-testable by the time the first of them lands.
            if let Some(at) = scene.right_click {
                if f >= after - 2 {
                    input.mouse = at;
                }
                if f == after - 2 {
                    input.right_pressed = true;
                }
            }
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
            // Five notches, once everything else the scene asked for has
            // happened: a pane opened by a click is not there to be scrolled
            // until the click has landed.
            let wheel_from =
                script_start + scene.script.len() as u32 + clicking + scene.then.len() as u32 + 7;
            if let Some(at) = scene.hover.filter(|_| f >= wheel_from) {
                input.mouse = at;
            }
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
                // A frame may ask to be re-skinned, and the real event loop
                // obliges. A harness that drops the request would show every
                // scheme looking like the one it started in.
                if let Some(next) = ui.finish().theme {
                    theme = next;
                }
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
