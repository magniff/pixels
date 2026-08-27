//! Drive the whole application, with the real model, and check what happens.
//!
//! The screenshot pass proved the application can be run with no window and no
//! event loop, and said so: if it can be driven headlessly, so can a test. This
//! is that test. The difference is that it uses the model rather than the stub,
//! it waits for answers instead of counting frames, and it looks at the vault
//! on disk afterwards rather than at the pixels.
//!
//! It is not a unit test and does not belong in `cargo test`: it wants weights,
//! it takes minutes, and it can fail because a model had an off day rather than
//! because the code is wrong. It is the thing you run before believing the
//! assistant works. `tools/e2e.sh` sets up the sandbox and runs it.
//!
//! Everything happens in a vault made for the run and thrown away after, with
//! its own settings file, so it cannot touch the notes you actually keep.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::{frame, theme, Notes};
use pixui::{Canvas, Input, Key, Mods, Point, Theme, Ui, UiState};

/// How long to wait for one answer before calling it a failure.
///
/// Generous: the first question of a run loads the weights and reads the whole
/// vault, and on a laptop that is twenty seconds before a word is written.
const PATIENCE: Duration = Duration::from_secs(180);

/// A running application, and the means to poke it.
struct App {
    dir: PathBuf,
    app: Notes,
    canvas: Canvas,
    ui: UiState,
    theme: Theme,
    input: Input,
    frames: u64,
}

impl App {
    fn open(dir: &Path) -> Self {
        let mut ui = UiState::new();
        // So a button can be found by its label rather than by a pixel.
        ui.index(true);
        Self {
            dir: dir.to_path_buf(),
            app: Notes::open(dir.to_path_buf()),
            canvas: Canvas::new(420, 260),
            ui,
            theme: theme(),
            input: Input {
                mouse_in_window: true,
                dt: 1.0 / 60.0,
                ..Default::default()
            },
            frames: 0,
        }
    }

    /// One frame, exactly as the event loop runs it.
    fn step(&mut self) {
        self.frames += 1;
        self.input.time = self.frames as f32 / 60.0;
        self.canvas.clear(self.theme.background);
        {
            let mut ui = Ui::begin(&mut self.canvas, &self.input, &self.theme, &mut self.ui);
            frame(&mut ui, &mut self.app);
            if let Some(next) = ui.finish().theme {
                self.theme = next;
            }
        }
        self.input.begin_frame();
        self.input.keys.clear();
        self.input.mods = Mods::default();
        self.input.mouse_pressed = false;
        self.input.mouse_released = false;
        self.input.right_pressed = false;
        self.input.wheel = 0.0;
    }

    fn steps(&mut self, n: u32) {
        for _ in 0..n {
            self.step();
        }
    }

    /// One key, then a couple of frames for whatever it opened.
    fn press(&mut self, key: Key, mods: Mods) -> &mut Self {
        self.input.keys.push(key);
        self.input.mods = mods;
        self.step();
        self.steps(2);
        self
    }

    fn key(&mut self, key: Key) -> &mut Self {
        self.press(key, Mods::default())
    }

    /// Type a string one character a frame, so a modal editor parses it the way
    /// it would if somebody were typing.
    fn typed(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            let key = match c {
                '\n' => Key::Enter,
                c => Key::Char(c),
            };
            self.input.keys.push(key);
            self.step();
        }
        self.steps(2);
        self
    }

    /// Click the middle of the widget with this label.
    fn click(&mut self, name: &str) -> Result<(), String> {
        let Some(rect) = self.ui.find(name) else {
            return Err(format!(
                "nothing called {name:?} on screen. what is: {:?}",
                self.ui.names()
            ));
        };
        let at = Point {
            x: rect.x + rect.w / 2,
            y: rect.y + rect.h / 2,
        };
        self.tap(at);
        Ok(())
    }

    fn tap(&mut self, at: Point) {
        self.input.mouse = at;
        self.step();
        self.input.mouse = at;
        self.input.mouse_down = true;
        self.input.mouse_pressed = true;
        self.step();
        self.input.mouse_down = false;
        self.input.mouse_released = true;
        self.step();
        self.steps(3);
    }

    /// Run frames until the model has answered, or give up.
    fn wait(&mut self) -> Result<Duration, String> {
        let began = Instant::now();
        // A frame or two for the question to be posted before believing it is
        // not busy: `ask` happens during a frame, not before one.
        self.steps(3);
        while self.app.helper.busy() {
            if began.elapsed() > PATIENCE {
                return Err(format!("no answer in {PATIENCE:?}"));
            }
            self.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        // And a few more so the answer is taken out of the channel and drawn.
        self.steps(6);
        Ok(began.elapsed())
    }

    /// Run quiet frames until everything dirty has been written.
    ///
    /// A note goes to disk when the typing stops, and the clock that measures
    /// "stopped" counts the frame time the application is handed. So a harness
    /// that wants to look at the vault has to let those frames go by, exactly
    /// as a person letting go of the keyboard does.
    fn saved(&mut self) {
        for _ in 0..400 {
            if !self.app.notes.iter().any(|n| n.buffer.dirty) {
                break;
            }
            self.step();
        }
        self.steps(4);
    }

    /// Roll the wheel down over the middle of the window until the newest
    /// thing said is on screen.
    ///
    /// A control that has scrolled out of view is not on the frame, and so not
    /// in the index either - which is right, since it is not clickable for
    /// anybody else at that moment. The buttons on the newest change are below
    /// the fold in a conversation of any length, so getting to them is part of
    /// what a person does.
    fn scroll_to_end(&mut self) {
        let mid = Point {
            x: self.canvas.width() / 2,
            y: self.canvas.height() / 2,
        };
        for _ in 0..30 {
            self.input.mouse = mid;
            self.input.wheel = -3.0;
            self.step();
        }
        self.steps(3);
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(path)).ok()
    }

    /// Every file in the vault, for saying what is actually there when
    /// something expected is not.
    fn vault(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![self.dir.clone()];
        while let Some(at) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&at) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(&self.dir) {
                    out.push(rel.display().to_string());
                }
            }
        }
        out.sort();
        out
    }

    fn on_screen(&self) -> Vec<String> {
        self.ui.names()
    }
}

fn cmd(key: Key) -> Mods {
    let _ = key;
    Mods {
        cmd: true,
        ..Default::default()
    }
}

/// The last thing the conversation said, with the wiring taken out.
fn last_answer(app: &App) -> String {
    app.app
        .chat
        .as_ref()
        .and_then(|c| c.turns.last())
        .map(crate::chat::copyable)
        .unwrap_or_default()
}

fn said(app: &App, want: &[&str]) -> Result<(), String> {
    let got = last_answer(app).to_lowercase();
    if want.iter().any(|w| got.contains(&w.to_lowercase())) {
        return Ok(());
    }
    Err(format!("expected one of {want:?}, got: {got:?}"))
}

/// The mark that says a failure is the model's rather than the application's.
///
/// Worth telling apart, and only these two kinds exist. A button that is not
/// there, a question that never went, a file not written after the change was
/// taken - those are the application, and they are what this exists to catch.
/// An answer that is simply wrong is a model having a poor day: it will pass
/// tomorrow against the same build, so failing the run over it would teach
/// whoever runs it to stop reading the result. Both are printed; only the
/// application's kind sets the exit code.
const WRONG: &str = "the model was wrong: ";

// --------------------------------------------------------------- the scenes

/// Write a note the way a person does: open one, type into it, save it.
///
/// No model in this one. It is here because everything after it depends on the
/// vault being real, and a failure here means the rest is not worth reading.
fn a_note_typed_by_hand(app: &mut App) -> Result<(), String> {
    app.press(Key::Char(':'), Mods::default());
    app.typed("new\n");
    app.key(Key::Char('i'));
    app.typed("# Tap\n\nThe kitchen tap drips at night.\n");
    app.key(Key::Escape);
    app.press(Key::Char(':'), Mods::default());
    app.typed("w\n");
    app.steps(4);
    // A note nobody has named is saved through the dialog that asks, which is
    // the same dialog a person gets.
    if app.ui.find("SAVE").is_some() {
        app.typed("tap");
        app.click("SAVE")?;
        app.saved();
    }

    let written = app
        .vault()
        .into_iter()
        .find(|f| f.ends_with(".md") && app.read(f).is_some_and(|t| t.contains("kitchen tap")));
    match written {
        Some(f) => {
            let text = app.read(&f).unwrap_or_default();
            if !text.contains("# Tap") {
                return Err(format!("{f} lost its heading: {text:?}"));
            }
            Ok(())
        }
        None => Err(format!("nothing on disk holds it. vault: {:?}", app.vault())),
    }
}

/// Have a conversation open, whether or not the last scene left one.
///
/// Each scene leans on the one before - which is how the application is used,
/// a question about a note made a minute ago - but leaning is not the same as
/// depending. A scene that can put itself in the state it needs is a scene
/// that can be run on its own to find out why it failed, and one whose failure
/// does not take the rest down with it.
fn chatting(app: &mut App) -> Result<(), String> {
    if app.app.chat.is_none() {
        app.press(Key::Char('e'), cmd(Key::Char('e')));
        app.key(Key::Escape);
        app.press(Key::Enter, cmd(Key::Enter));
        app.steps(4);
    }
    if app.app.chat.is_none() {
        return Err(format!(
            "cmd-enter did not open a conversation. on screen: {:?}",
            app.on_screen()
        ));
    }
    // A change that has been offered is a question back, and the application
    // holds the box until it is answered - deliberately, so an answer is not
    // written against a note that may be about to change. So anything a
    // previous scene left standing is turned down before this one asks
    // anything: turned down rather than taken, because a scene should only
    // change the vault on purpose.
    for _ in 0..6 {
        app.scroll_to_end();
        if app.ui.find("chat-field").is_some() || app.click("REJECT").is_err() {
            break;
        }
        app.steps(4);
    }
    // Click into the box before typing, the way somebody coming back to a
    // conversation does. The keyboard may be anywhere after a button was
    // pressed, and a scene that types into nothing fails in a way that says
    // nothing about the application.
    let _ = app.click("chat-field");
    Ok(())
}

/// Type a question and send it, and be sure it went.
fn asking(app: &mut App, question: &str) -> Result<(), String> {
    chatting(app)?;
    let before = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    app.typed(question);
    app.key(Key::Enter);
    app.wait()?;
    let after = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    if after <= before + 1 {
        return Err(format!(
            "the question did not go: {before} turns before, {after} after. on screen: {:?}",
            app.on_screen()
        ));
    }
    Ok(())
}

/// Take every change on offer, one after another.
fn accept_all(app: &mut App) -> usize {
    let mut taken = 0;
    for _ in 0..6 {
        app.scroll_to_end();
        if app.click("ACCEPT").is_err() {
            break;
        }
        taken += 1;
        app.saved();
    }
    taken
}

/// Ask a question that only a tool can answer, and check it went and asked.
fn a_question_the_model_must_look_up(app: &mut App) -> Result<(), String> {
    let began = Instant::now();
    asking(app, "what is the date today? answer with the year in figures.")?;
    let took = began.elapsed();
    let answer = last_answer(app);
    let used = app
        .app
        .chat
        .as_ref()
        .and_then(|c| c.turns.last())
        .map(|t| crate::chat::lookups(&t.text).1.len())
        .unwrap_or(0);
    if used == 0 {
        return Err(format!("{WRONG}answered without looking anything up: {answer:?}"));
    }
    // The year the machine says, not one written down here: this file will
    // still be run next year.
    let year = crate::clock::about("today")
        .unwrap_or_default()
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_digit()))
        .find(|w| w.len() == 4 && w.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or_default()
        .to_string();
    if year.is_empty() {
        return Err("this machine has no clock to check against".into());
    }
    said(app, &[&year]).map_err(|e| format!("{WRONG}{e} (after {took:?})"))
}

/// Ask it to write a file, take the change, and look for the file.
fn a_file_the_model_writes(app: &mut App) -> Result<(), String> {
    asking(
        app,
        "create a note called kettle.md containing one line: the kettle is broken.",
    )?;
    let after = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    let before = after.saturating_sub(2);
    if after <= before + 1 {
        return Err(format!(
            "the question did not go: {before} turns before, {after} after. on screen: {:?}",
            app.on_screen()
        ));
    }
    if accept_all(app) == 0 {
        return Err(format!(
            "no change was offered. on screen: {:?}\n      it said: {:?}",
            app.on_screen(),
            last_answer(app).chars().take(300).collect::<String>()
        ));
    }
    let found = app
        .vault()
        .into_iter()
        .find(|f| f.ends_with("kettle.md"))
        .ok_or_else(|| format!("no kettle.md. vault: {:?}", app.vault()))?;
    let text = app.read(&found).unwrap_or_default();
    if !text.to_lowercase().contains("kettle") {
        return Err(format!("{found} does not mention the kettle: {text:?}"));
    }
    Ok(())
}

/// Offer a change and turn it down. The point is that nothing moves.
fn a_change_turned_down(app: &mut App) -> Result<(), String> {
    chatting(app)?;
    // Notes only. The transcript of this very conversation is written as it
    // goes, and that is not the vault being changed behind anybody's back.
    let before = app
        .vault()
        .into_iter()
        .filter(|f| !f.contains(".chats"))
        .filter_map(|f| app.read(&f).map(|t| (f, t)))
        .collect::<Vec<_>>();
    asking(app, "rewrite every note in this project in French.")?;
    // Nothing offered is a fine outcome for this one - it is the taking of it
    // that must not happen behind your back.
    if app.click("REJECT").is_ok() {
        app.steps(8);
    }
    for (f, was) in before {
        let now = app.read(&f).unwrap_or_default();
        if now != was {
            return Err(format!("{f} changed after a refusal"));
        }
    }
    Ok(())
}

/// A passage in the editor, changed by the model and kept.
fn a_passage_the_model_rewrites(app: &mut App) -> Result<(), String> {
    if app.app.chat.is_some() {
        app.key(Key::Escape);
        app.steps(4);
    }
    app.press(Key::Char('e'), cmd(Key::Char('e')));
    // The note this scene is about, found by name rather than by whichever one
    // the last scene happened to leave open.
    app.press(Key::Char('p'), cmd(Key::Char('p')));
    app.typed("tap");
    app.key(Key::Enter);
    app.steps(4);
    if !app.app.note().filename().contains("tap") {
        return Err(format!(
            "could not get to tap.md; open is {:?}",
            app.app.note().filename()
        ));
    }
    let before = app.app.note().buffer.to_text();
    let first = before.lines().next().unwrap_or_default().to_string();

    app.key(Key::Char('g'));
    app.key(Key::Char('g'));
    app.key(Key::Char('V'));
    app.press(Key::Enter, cmd(Key::Enter));
    app.steps(4);
    if app.app.assist.is_none() {
        return Err(format!(
            "cmd-enter on a selection opened nothing. on screen: {:?}",
            app.on_screen()
        ));
    }
    app.typed("rewrite this line in capital letters, same words");
    app.key(Key::Enter);
    app.wait()?;
    app.press(Key::Enter, cmd(Key::Enter));
    app.saved();

    let after = app.app.note().buffer.to_text();
    let now = after.lines().next().unwrap_or_default().to_string();
    if now == first {
        return Err(format!("{WRONG}the line is unchanged: {first:?}"));
    }
    // The words are still the words, whatever case they are in now.
    let letters = |t: &str| {
        t.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
    };
    if letters(&now) != letters(&first) {
        return Err(format!("{WRONG}it said something else: {first:?} became {now:?}"));
    }
    Ok(())
}

/// One question that needs the calculator, the calendar, and a file written.
///
/// The interesting part is not any one of the three but that they come from a
/// single sentence: the model has to reach twice, keep both answers, and then
/// put them somewhere. Each of those works on its own; this is the one that
/// says whether they work together.
fn several_tools_from_one_question(app: &mut App) -> Result<(), String> {
    asking(
        app,
        "work out 384 * 517, and find out what day of the week christmas falls on this year. \
         then write both answers into a note called facts.md.",
    )?;
    let reply = last_answer(app);
    let looked = app
        .app
        .chat
        .as_ref()
        .and_then(|c| c.turns.last())
        .map(|t| crate::chat::lookups(&t.text).1)
        .unwrap_or_default();
    // At least one, because a sum this size is not something to answer from
    // memory. How many is the model's business: what is checked below is
    // whether both answers are right and both reached the file, which is the
    // question the application is being asked here.
    if looked.is_empty() {
        return Err(format!(
            "{WRONG}reached for nothing at all: {:?}",
            reply.chars().take(200).collect::<String>()
        ));
    }
    if accept_all(app) == 0 {
        // A reply with nothing in it means the application lost the answer,
        // which is its fault. A reply that answers the question and forgets to
        // propose the file is the model doing three quarters of what it was
        // asked.
        let empty = reply.trim().is_empty();
        return Err(format!(
            "{}nothing to accept after {} lookups: {:?}",
            if empty { "" } else { WRONG },
            looked.len(),
            reply.chars().take(200).collect::<String>()
        ));
    }
    // Worked out here rather than written down, so this still runs next year.
    let sum = crate::calc::evaluate("384 * 517").unwrap_or_default();
    // Up to "Today is", because the answer about another day says what day
    // *this* is as well and that is not the one being asked about.
    let christmas = crate::clock::about("12-25").unwrap_or_default();
    let about_it = christmas.split("Today is").next().unwrap_or_default().to_string();
    let day = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
        .into_iter()
        .find(|d| about_it.contains(d))
        .unwrap_or("");
    let found = app
        .vault()
        .into_iter()
        .find(|f| f.ends_with("facts.md"))
        .ok_or_else(|| format!("no facts.md. vault: {:?}", app.vault()))?;
    let text = app.read(&found).unwrap_or_default();
    let mut missing = Vec::new();
    if !text.contains(&sum) && !text.contains("198,528") {
        missing.push(format!("the sum ({sum})"));
    }
    if !day.is_empty() && !text.contains(day) {
        missing.push(format!("the day ({day})"));
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("{WRONG}{found} is missing {missing:?}: {text:?}"))
    }
}

/// The conversation is on disk afterwards, and it is the conversation.
fn the_conversation_is_kept(app: &mut App) -> Result<(), String> {
    app.steps(6);
    let chats: Vec<String> = app
        .vault()
        .into_iter()
        .filter(|f| f.contains(".chats"))
        .collect();
    if chats.is_empty() {
        return Err(format!("nothing filed. vault: {:?}", app.vault()));
    }
    let any = chats
        .iter()
        .filter_map(|f| app.read(f))
        .any(|t| t.to_lowercase().contains("date") || t.to_lowercase().contains("kettle"));
    if !any {
        return Err(format!("{WRONG}{chats:?} hold none of what was asked"));
    }
    Ok(())
}

// --------------------------------------------------------------- the running

type Scene = (&'static str, fn(&mut App) -> Result<(), String>);

/// In order, and against one application: each leans on what the last left
/// behind, which is also how somebody uses this - a conversation about a note
/// they made a minute ago.
const SCENES: &[Scene] = &[
    ("a note typed by hand", a_note_typed_by_hand),
    ("a question the model must look up", a_question_the_model_must_look_up),
    ("a file the model writes", a_file_the_model_writes),
    ("several tools from one question", several_tools_from_one_question),
    ("a change turned down", a_change_turned_down),
    ("a passage the model rewrites", a_passage_the_model_rewrites),
    ("the conversation is kept", the_conversation_is_kept),
];

/// Run the lot. Returns the number that failed.
pub fn run() -> std::io::Result<i32> {
    let dir = std::env::var_os("PIXUI_NOTES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("notes"));
    std::fs::create_dir_all(&dir)?;
    let only = std::env::var("E2E_ONLY").unwrap_or_default();

    println!("vault {}", dir.display());
    let mut app = App::open(&dir);
    // A few frames to open on, before anything is asked of it.
    app.steps(6);
    println!("model {}\n", app.app.helper.name());

    let mut failed = 0;
    let mut wrong = 0;
    for (name, scene) in SCENES {
        if !only.is_empty() && !name.contains(&only) {
            continue;
        }
        let began = Instant::now();
        print!("  {name} ... ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        match scene(&mut app) {
            Ok(()) => println!("ok   ({:.1}s)", began.elapsed().as_secs_f32()),
            Err(why) if why.starts_with(WRONG) => {
                wrong += 1;
                println!(
                    "answered wrongly ({:.1}s)\n      {}",
                    began.elapsed().as_secs_f32(),
                    why.trim_start_matches(WRONG)
                );
            }
            Err(why) => {
                failed += 1;
                println!("FAILED ({:.1}s)\n      {why}", began.elapsed().as_secs_f32());
            }
        }
    }
    println!();
    match (failed, wrong) {
        (0, 0) => println!("all {} scenes passed", SCENES.len()),
        (0, w) => println!(
            "the application did everything asked of it; the model was wrong in {w} of {}",
            SCENES.len()
        ),
        (f, 0) => println!("{f} of {} scenes failed", SCENES.len()),
        (f, w) => println!(
            "{f} of {} scenes failed, and the model was wrong in {w} more",
            SCENES.len()
        ),
    }
    Ok(failed)
}
