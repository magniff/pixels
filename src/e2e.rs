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

    /// Roll the wheel up, to see what is above what is on screen.
    fn scroll_to_top(&mut self) {
        let mid = Point {
            x: self.canvas.width() / 2,
            y: self.canvas.height() / 2,
        };
        for _ in 0..40 {
            self.input.mouse = mid;
            self.input.wheel = 3.0;
            self.step();
        }
        self.steps(3);
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

    /// A few notches of wheel, for walking down a long transcript.
    fn scroll_by(&mut self, notches: f32) {
        let mid = Point {
            x: self.canvas.width() / 2,
            y: self.canvas.height() / 2,
        };
        for _ in 0..3 {
            self.input.mouse = mid;
            self.input.wheel = notches;
            self.step();
        }
        self.steps(2);
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
        None => Err(format!(
            "nothing on disk holds it. vault: {:?}",
            app.vault()
        )),
    }
}

/// Have a conversation open, whether or not the last scene left one.
///
/// Each scene leans on the one before - which is how the application is used,
/// a question about a note made a minute ago - but leaning is not the same as
/// depending. A scene that can put itself in the state it needs is a scene
/// that can be run on its own to find out why it failed, and one whose failure
/// does not take the rest down with it.
/// Begin a conversation of this scene's own.
///
/// Each scene had been talking into the one the last scene left open, and a
/// transcript that long is its own problem: the newest change sits below the
/// fold, an older one waits above it, and only what is on the screen can be
/// clicked. Which is true for anybody - but a person having a new conversation
/// starts one, and so does this. It also means a scene that fails does not
/// hand the next one a panel still waiting for an answer.
fn fresh_chat(app: &mut App) -> Result<(), String> {
    if app.app.chat.is_some() {
        // Answer anything outstanding first, or the panel will not close.
        chatting(app)?;
        app.key(Key::Escape);
        app.steps(4);
    }
    app.press(Key::Char('e'), cmd(Key::Char('e')));
    app.key(Key::Escape);
    app.press(Key::Enter, cmd(Key::Enter));
    app.steps(4);
    // A project that already has conversations in it offers the list of them
    // rather than starting another straight away, and the first row of that
    // list is the new one.
    if app.app.chat.is_none() && app.ui.find("chat0").is_some() {
        app.click("chat0")?;
        app.steps(6);
    }
    let fresh = app.app.chat.as_ref().is_some_and(|c| c.turns.is_empty());
    if !fresh {
        return Err(format!(
            "could not start a new conversation. on screen: {:?}",
            app.on_screen()
        ));
    }
    let _ = app.click("chat-field");
    Ok(())
}

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
    // Until the box comes back, not a fixed few times: one reply can propose a
    // change to every file in the project, and asked to rewrite them all in
    // French one did exactly that. Six left standing was enough to leave the
    // next scene typing into a panel that was still waiting for an answer.
    //
    // From the top, because a change waiting to be answered may be anywhere in
    // the conversation and only what is on screen can be clicked - which is
    // true for anybody, not only for this. Rolling to the bottom and looking
    // there found nothing and gave up, with the panel still held.
    for _ in 0..40 {
        if app.ui.find("chat-field").is_some() {
            break;
        }
        app.scroll_to_top();
        let mut answered = false;
        for _ in 0..20 {
            if app.click("REJECT").is_ok() {
                answered = true;
                app.steps(4);
            } else if app.ui.find("chat-field").is_some() {
                break;
            } else {
                app.scroll_by(-4.0);
            }
        }
        if !answered {
            break;
        }
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
    for _ in 0..40 {
        app.scroll_to_end();
        // Looked for, then clicked once. Clicking to find out whether it was
        // there took the change and then reported that there had been none,
        // because the second click found the button gone - which it was,
        // having just been pressed.
        if app.ui.find("ACCEPT").is_none() {
            // It may be further up: only what is on screen can be clicked.
            app.scroll_to_top();
            for _ in 0..20 {
                if app.ui.find("ACCEPT").is_some() {
                    break;
                }
                app.scroll_by(-4.0);
            }
        }
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
    fresh_chat(app)?;
    let began = Instant::now();
    asking(
        app,
        "what is the date today? answer with the year in figures.",
    )?;
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
        return Err(format!(
            "{WRONG}answered without looking anything up: {answer:?}"
        ));
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
    fresh_chat(app)?;
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
    fresh_chat(app)?;
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
        return Err(format!(
            "{WRONG}it said something else: {first:?} became {now:?}"
        ));
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
    fresh_chat(app)?;
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
    // The first weekday the answer names, which is the one it is about. It
    // goes on to name two more - the same day a year earlier, and today - and
    // picking whichever came first in a list of weekday names rather than
    // first in the sentence had this expecting a Thursday of a Friday.
    let christmas = crate::clock::about("12-25").unwrap_or_default();
    let day = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ]
    .into_iter()
    .filter_map(|d| christmas.find(d).map(|at| (at, d)))
    .min_by_key(|(at, _)| *at)
    .map(|(_, d)| d)
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

/// What the clock says about a day, in the two shapes this scene checks.
///
/// Read off the tool rather than written down, so this still means something
/// next year: the answer to "how many days have I been alive" moves every
/// night.
fn day_count(date: &str) -> Option<(String, String)> {
    let said = crate::clock::about(date).ok()?;
    let days = said
        .split_whitespace()
        .zip(said.split_whitespace().skip(1))
        .find(|(_, next)| next.starts_with("days"))
        .map(|(n, _)| n.to_string())?;
    // "612 days ago - 1 year and 8 months." -> "1 year and 8 months"
    let age = said
        .split(" ago - ")
        .nth(1)?
        .split('.')
        .next()?
        .trim()
        .to_string();
    Some((days, age))
}

/// A conversation about birthdays, ending in a note about them.
///
/// The one that was reported, and it went wrong in four different places
/// before it went right: a date written the way people write it was refused,
/// the day count was worked out by hand and came out eight hundred days off,
/// the note was written into a block that parsed as nothing, and a child of
/// one was described as two. All four are the same mistake - a number derived
/// instead of read - so this asks for all of them at once.
fn a_family_of_birthdays(app: &mut App) -> Result<(), String> {
    fresh_chat(app)?;
    let (mine, _) = day_count("1989-07-31").ok_or("the clock cannot say")?;
    let (hers, her_age) = day_count("2024-12-23").ok_or("the clock cannot say")?;

    // A date the way somebody types it, not the way a machine takes it.
    asking(app, "my bd is jul 31 1989, how many days i've been alive")?;
    let said = last_answer(app).replace(',', "");
    if !said.contains(&mine) {
        return Err(format!("{WRONG}{mine} days, it said: {said:?}"));
    }

    // The age of a small child, which is the one it kept getting wrong: 612
    // days divided by 365 and rounded is two, and she is one.
    asking(
        app,
        "our daughter eva was born 23 dec 2024. how old is she in years, and in days?",
    )?;
    let said = last_answer(app).replace(',', "").to_lowercase();
    if !said.contains(&hers) {
        return Err(format!("{WRONG}{hers} days, it said: {said:?}"));
    }
    let years = her_age.split_whitespace().next().unwrap_or_default();
    let wrong_by_one = format!("{} years", years.parse::<i64>().unwrap_or(0) + 1);
    if said.contains(&wrong_by_one) {
        return Err(format!(
            "{WRONG}she is {her_age}, and it said {wrong_by_one}: {said:?}"
        ));
    }

    // And the note, which is where the whole thing was falling on the floor.
    asking(
        app,
        "make a note called ages.md with both of us in it, our birthdays and how many days old we each are",
    )?;
    if accept_all(app) == 0 {
        // Announcing the change instead of proposing it is the model ignoring
        // the one instruction it is given twice. The application has nothing
        // to answer for when there is nothing to offer.
        return Err(format!(
            "{WRONG}nothing was proposed: {:?}",
            last_answer(app).chars().take(200).collect::<String>()
        ));
    }
    let found = app
        .vault()
        .into_iter()
        .find(|f| f.ends_with("ages.md"))
        .ok_or_else(|| format!("no ages.md after it was accepted. vault: {:?}", app.vault()))?;
    let text = app.read(&found).unwrap_or_default().replace(',', "");
    let missing: Vec<&String> = [&mine, &hers]
        .into_iter()
        .filter(|d| !text.contains(*d))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("{WRONG}{found} is missing {missing:?}: {text:?}"))
    }
}

/// A sum whose parts all have to be looked up, and a share of a life.
///
/// The one that looked right and was out by 929 days. Every figure in it comes
/// from somewhere different - two birthdays, a third date years later, and a
/// division of one by another - and the only way to get the last one right is
/// to have got the three before it right. Which is why it is here: it fails
/// loudly for any of half a dozen reasons, and quietly for none.
fn a_share_of_a_lifetime(app: &mut App) -> Result<(), String> {
    fresh_chat(app)?;
    let (mine, _) = day_count("1989-07-31").ok_or("the clock cannot say")?;
    let (met, _) = day_count("2010-12-01").ok_or("the clock cannot say")?;
    let (alive, together) = (
        mine.parse::<f64>().unwrap_or(1.0),
        met.parse::<f64>().unwrap_or(0.0),
    );
    // To the tenth and to the hundredth, because a share said as 42.4 and one
    // said as 42.45 are the same answer and only one of them is a failure.
    let exact = 100.0 * together / alive;
    let share = format!("{exact:.1}");
    let finer = format!("{exact:.2}");

    asking(
        app,
        "my bd is jul 31 1989 and i met my wife on dec 1 2010. how many days have i been alive, \
         how many days have we been together, and what percentage of my life is that?",
    )?;
    let said = last_answer(app).replace(',', "");
    let mut missing: Vec<String> = [(&mine, "days alive"), (&met, "days together")]
        .into_iter()
        .filter(|(want, _)| !said.contains(want.as_str()))
        .map(|(want, what)| format!("{what} ({want})"))
        .collect();
    if !said.contains(&share) && !said.contains(&finer) {
        missing.push(format!("the share ({share})"));
    }
    if missing.is_empty() {
        return Ok(());
    }
    // A share worked out from the right two numbers is allowed to be rounded
    // differently; what is not allowed is the numbers themselves being wrong.
    Err(format!("{WRONG}missing {missing:?}: {said:?}"))
}

/// A note edited by something other than this, mid-conversation.
///
/// The vault used to be read once at startup and never again, so this was
/// invisible: asked what colour the bike was after the file had been rewritten
/// in another window, the answer was the colour it used to be.
fn a_note_changed_behind_its_back(app: &mut App) -> Result<(), String> {
    fresh_chat(app)?;
    // Made the way it was made when this was reported: by the model, and
    // accepted - not written here. A note this program created and saved
    // itself is a different path to one it found on disk.
    asking(app, "create a note called bike.md saying the bike is RED")?;
    if accept_all(app) == 0 {
        return Err(format!(
            "{WRONG}it proposed nothing: {:?}",
            last_answer(app).chars().take(120).collect::<String>()
        ));
    }
    let bike = app.dir.join("bike.md");
    if !bike.exists() {
        return Err(format!("no bike.md on disk. vault: {:?}", app.vault()));
    }
    asking(app, "what colour is the bike? one word.")?;
    let said = last_answer(app).to_lowercase();
    if !said.contains("red") {
        return Err(format!("{WRONG}it should say red: {said:?}"));
    }
    // Somebody else writes it. A whole second, because a filesystem that keeps
    // seconds cannot tell two writes inside one apart.
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(&bike, "# Bike\n\nThe bike is GREEN.\n")
        .map_err(|e| format!("could not rewrite the note: {e}"))?;
    app.steps(90);
    asking(app, "and now? one word.")?;
    let said = last_answer(app).to_lowercase();
    if !said.contains("green") {
        return Err(format!("the first change was missed: {said:?}"));
    }

    // And again, with a change of the model's own in between - which is where
    // it stopped noticing when this was reported. Its own edit leaves the note
    // unsaved for a moment, and a file saved in that moment used to lose.
    asking(app, "make it PURPLE")?;
    if accept_all(app) == 0 {
        return Err(format!(
            "{WRONG}it proposed nothing: {:?}",
            last_answer(app).chars().take(120).collect::<String>()
        ));
    }
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(&bike, "# Bike\n\nThe bike is YELLOW.\n")
        .map_err(|e| format!("could not rewrite the note: {e}"))?;
    app.steps(20);
    asking(app, "and now? one word.")?;
    let said = last_answer(app).to_lowercase();
    if said.contains("yellow") {
        Ok(())
    } else {
        Err(format!("missed the change after its own edit: {said:?}"))
    }
}

/// The reported one, to the letter: a note in a project, made by the model,
/// then the file changed from outside.
fn a_note_in_a_project_changed_outside(app: &mut App) -> Result<(), String> {
    // A project, and the editor looking at something in it - which is what
    // decides the project a conversation is about.
    let folder = app.dir.join("new-one");
    std::fs::create_dir_all(&folder).map_err(|e| format!("{e}"))?;
    // A name nothing else in the vault shares, because the finder takes the
    // best match and by this point there are a dozen notes to match against.
    std::fs::write(folder.join("zzqqseed.md"), "# Seed\n\nsomething.\n")
        .map_err(|e| format!("{e}"))?;
    app.steps(90);
    // The keyboard first: a conversation left open by the scene before takes
    // every key, and the finder never hears about it.
    if app.app.chat.is_some() {
        chatting(app)?;
        app.key(Key::Escape);
        app.steps(4);
    }
    app.press(Key::Char('e'), cmd(Key::Char('e')));
    app.press(Key::Char('p'), cmd(Key::Char('p')));
    app.typed("zzqqseed");
    app.key(Key::Enter);
    app.steps(6);
    if app.app.note().project != "new-one" {
        return Err(format!(
            "the editor is in {:?} on {:?}, not the project",
            app.app.note().project,
            app.app.note().filename()
        ));
    }
    fresh_chat(app)?;
    asking(
        app,
        "make a note stating that the cycle is red, call the note cycle.md",
    )?;
    if accept_all(app) == 0 {
        return Err(format!(
            "{WRONG}nothing proposed: {:?}",
            last_answer(app).chars().take(120).collect::<String>()
        ));
    }
    let bike = folder.join("cycle.md");
    if !bike.exists() {
        return Err(format!("no new-one/cycle.md. vault: {:?}", app.vault()));
    }
    asking(app, "what colour is the cycle? one word.")?;
    if !last_answer(app).to_lowercase().contains("red") {
        return Err(format!("{WRONG}should be red: {:?}", last_answer(app)));
    }
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(&bike, "# Cycle\n\nThe cycle is green.\n").map_err(|e| format!("{e}"))?;
    app.steps(90);
    asking(app, "what colour is the cycle now? one word.")?;
    let said = last_answer(app).to_lowercase();
    if said.contains("green") {
        Ok(())
    } else {
        Err(format!("still says: {said:?}"))
    }
}

/// Asked to read a note, it reads the note.
///
/// Typed in a real conversation and answered from memory: "read the file" came
/// back with the contents of a version that had stopped existing, three times
/// over, and "are you sure, i just changed it" got the same answer again. It
/// had nothing to read with.
fn reading_a_note_when_asked(app: &mut App) -> Result<(), String> {
    fresh_chat(app)?;
    let note = app.dir.join("weather.md");
    std::fs::write(&note, "# Weather\n\nIt is RAINING today.\n").map_err(|e| format!("{e}"))?;
    app.steps(90);
    asking(app, "read weather.md and tell me the weather in one word")?;
    let said = last_answer(app).to_lowercase();
    let looked = app
        .app
        .chat
        .as_ref()
        .and_then(|c| c.turns.last())
        .map(|t| crate::chat::lookups(&t.text).1)
        .unwrap_or_default();
    if !looked.iter().any(|l| l.tool == "read") {
        return Err(format!(
            "{WRONG}it answered without reading: {:?}",
            said.chars().take(90).collect::<String>()
        ));
    }
    if !said.contains("rain") {
        return Err(format!("{WRONG}it read the wrong thing: {said:?}"));
    }
    Ok(())
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
    (
        "a question the model must look up",
        a_question_the_model_must_look_up,
    ),
    ("a file the model writes", a_file_the_model_writes),
    (
        "several tools from one question",
        several_tools_from_one_question,
    ),
    ("a change turned down", a_change_turned_down),
    ("a family of birthdays", a_family_of_birthdays),
    ("a share of a lifetime", a_share_of_a_lifetime),
    ("a passage the model rewrites", a_passage_the_model_rewrites),
    (
        "a note changed behind its back",
        a_note_changed_behind_its_back,
    ),
    (
        "a note in a project changed outside",
        a_note_in_a_project_changed_outside,
    ),
    ("reading a note when asked", reading_a_note_when_asked),
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
                println!(
                    "FAILED ({:.1}s)\n      {why}",
                    began.elapsed().as_secs_f32()
                );
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
