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
/// What the panel made of a question that did not go.
///
/// Reporting the turn count alone read as "the model said nothing" when the
/// truth was that it had never been asked - and the two want looking into in
/// completely different places.
fn refusal(app: &App) -> String {
    app.app
        .chat
        .as_ref()
        .and_then(|c| c.failed.clone())
        .unwrap_or_else(|| "no reason given".into())
}

fn asking(app: &mut App, question: &str) -> Result<(), String> {
    chatting(app)?;
    // Typed into the box, or not at all: with no box on screen the question
    // goes into the note, which fails later and says nothing about why.
    if app.ui.find("chat-field").is_none() {
        let folder = app.app.folder();
        let (waiting, held) = app
            .app
            .chat
            .as_ref()
            .map(|c| (c.waiting, c.pending(&folder)))
            .unwrap_or((false, false));
        return Err(format!(
            "no box to type into: waiting {waiting}, a change held {held}. on screen: {:?}",
            app.on_screen()
        ));
    }
    let before = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    app.typed(question);
    app.key(Key::Enter);
    app.wait()?;
    let after = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    if after <= before + 1 {
        return Err(format!(
            "the question did not go ({}): {before} turns before, {after} after. \
             on screen: {:?}",
            refusal(app),
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
            "the question did not go ({}): {before} turns before, {after} after. \
             on screen: {:?}",
            refusal(app),
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
    // "612 days ago - 1 year and 8 months." -> "1 year and 8 months". A day
    // still to come has no age, and is still a count: without this a date
    // ahead was counted as nothing at all, and a right answer marked wrong.
    let age = said
        .split(" ago - ")
        .nth(1)
        .and_then(|s| s.split('.').next())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
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
    let exact = 100.0 * together / alive;
    let share = format!("{exact:.1}");

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
    // Read as a number rather than matched as a string. A share worked out
    // from the right two numbers may be written 42.4, 42.45, 42.46 or 42.5,
    // and those are one answer rounded four ways - checking for two spellings
    // of it reported the model wrong for how it had written a right answer.
    let near = said
        .split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter_map(|w| w.trim_matches('.').parse::<f64>().ok())
        .any(|n| (n - exact).abs() < 0.1);
    if !near {
        missing.push(format!("the share ({share})"));
    }
    if missing.is_empty() {
        return Ok(());
    }
    // A share worked out from the right two numbers is allowed to be rounded
    // differently; what is not allowed is the numbers themselves being wrong.
    Err(format!("{WRONG}missing {missing:?}: {said:?}"))
}

/// Put the editor on a note in this project, which is what decides the project
/// a conversation is about. The note is made if it is not there, and found by
/// name - so the name had better be one nothing else in the vault shares, as
/// the finder takes the best match and by now there are a dozen to match.
fn looking_at(app: &mut App, project: &str, seed: &str) -> Result<(), String> {
    let folder = if project.is_empty() {
        app.dir.clone()
    } else {
        app.dir.join(project)
    };
    std::fs::create_dir_all(&folder).map_err(|e| format!("{e}"))?;
    let file = folder.join(format!("{seed}.md"));
    if !file.exists() {
        std::fs::write(&file, format!("# {seed}\n\nsomething.\n")).map_err(|e| format!("{e}"))?;
    }
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
    app.typed(seed);
    app.key(Key::Enter);
    app.steps(6);
    let (at, on) = (app.app.note().project.clone(), app.app.note().filename());
    if at != project || on != format!("{seed}.md") {
        return Err(format!(
            "the editor is in {at:?} on {on:?}, not {project:?}/{seed}.md"
        ));
    }
    Ok(())
}

/// Whether the answer has a number within `tol` of `want` in it.
fn number_near(said: &str, want: f64, tol: f64) -> bool {
    said.replace(',', "")
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .filter_map(|w| w.trim_matches('.').parse::<f64>().ok())
        .any(|n| (n - want).abs() <= tol)
}

/// Whether the last answer reached for this tool.
fn looked_with(app: &App, tool: &str) -> bool {
    app.app
        .chat
        .as_ref()
        .and_then(|c| c.turns.last())
        .map(|t| crate::chat::lookups(&t.text).1)
        .unwrap_or_default()
        .iter()
        .any(|l| l.tool == tool)
}

/// A note in another project: there to be read, and not to be changed.
///
/// Only the project on screen can be changed; the whole vault can be read.
/// The fact asked about is kept off the note's first line, because the first
/// line is in the list at the top of every conversation and a model can
/// answer from that without reading anything.
fn another_project_can_be_read_but_not_changed(app: &mut App) -> Result<(), String> {
    let garden = app.dir.join("garden");
    std::fs::create_dir_all(&garden).map_err(|e| format!("{e}"))?;
    let roses = garden.join("roses.md");
    let planted = "# Roses\n\nPlanted in March.\n\nThe roses are CRIMSON.\n";
    std::fs::write(&roses, planted).map_err(|e| format!("{e}"))?;
    looking_at(app, "new-one", "zzqqseed")?;
    fresh_chat(app)?;

    asking(
        app,
        "what colour are the roses in garden/roses.md? one word.",
    )?;
    let read = looked_with(app, "read");
    said(app, &["crimson"]).map_err(|e| format!("{WRONG}{e}"))?;
    if !read {
        return Err(format!("{WRONG}it answered without reading the note"));
    }

    asking(app, "change garden/roses.md to say the roses are YELLOW")?;
    accept_all(app);
    // Whatever it proposed and whatever was clicked, the other project is as
    // it was, and nothing was made here in its place.
    let now = std::fs::read_to_string(&roses).map_err(|e| format!("{e}"))?;
    if now != planted {
        return Err(format!("a note in another project was changed: {now:?}"));
    }
    if app.dir.join("new-one").join("roses.md").exists() {
        return Err("made in this project instead of refusing".into());
    }
    Ok(())
}

/// Sums over a table, in more than one step, and then a change built on them.
///
/// What people actually ask of a note with numbers in it: a total, a share of
/// it, and then put the total in. Two questions, several tools, and an edit
/// that has to land on the right line of a table.
fn a_table_summed_and_shared(app: &mut App) -> Result<(), String> {
    let table = "# Spend\n\n| Item | Cost |\n| --- | --- |\n| Rent | 1450 |\n| Food | 386 |\n\
                 | Transport | 92 |\n| Phone | 45 |\n";
    std::fs::write(app.dir.join("spend.md"), table).map_err(|e| format!("{e}"))?;
    looking_at(app, "", "spend")?;
    fresh_chat(app)?;

    asking(
        app,
        "in spend.md, what is the total cost, and what share of the total is rent, \
         as a percentage to one decimal place?",
    )?;
    let answer = last_answer(app);
    let mut missing = Vec::new();
    if !number_near(&answer, 1973.0, 0.0) {
        missing.push("the total (1973)");
    }
    if !number_near(&answer, 73.5, 0.1) {
        missing.push("the share (73.5)");
    }
    if !missing.is_empty() {
        return Err(format!("{WRONG}missing {missing:?}: {answer:?}"));
    }

    asking(app, "add a Total row at the end of the table")?;
    if accept_all(app) == 0 {
        return Err(format!(
            "{WRONG}it proposed nothing: {:?}",
            last_answer(app).chars().take(120).collect::<String>()
        ));
    }
    app.saved();
    let now = std::fs::read_to_string(app.dir.join("spend.md")).map_err(|e| format!("{e}"))?;
    if !now.contains("1973") {
        return Err(format!("{WRONG}the total row is not there: {now:?}"));
    }
    if !now.contains("| Rent | 1450 |") || !now.contains("| Phone | 45 |") {
        return Err(format!(
            "{WRONG}a row that was there is not any more: {now:?}"
        ));
    }
    Ok(())
}

/// Three questions, each leaning on the one before, ending in a note.
///
/// The dates go to the clock, the difference to the calculator, the weeks to
/// the calculator again, and the note has to carry both numbers - which the
/// model only has if it kept what it worked out two questions ago.
fn a_thread_that_leans_on_earlier_answers(app: &mut App) -> Result<(), String> {
    fresh_chat(app)?;
    asking(
        app,
        "how many days are there from 31 July 1989 to 26 October 1989?",
    )?;
    let answer = last_answer(app);
    if !number_near(&answer, 87.0, 0.0) {
        return Err(format!("{WRONG}87 days, it said: {answer:?}"));
    }
    asking(app, "and that in weeks, to one decimal place?")?;
    let answer = last_answer(app);
    if !number_near(&answer, 12.4, 0.05) {
        return Err(format!("{WRONG}12.4 weeks, it said: {answer:?}"));
    }
    asking(
        app,
        "write both of those numbers into a new note called gap.md",
    )?;
    if accept_all(app) == 0 {
        return Err(format!(
            "{WRONG}it proposed nothing: {:?}",
            last_answer(app).chars().take(120).collect::<String>()
        ));
    }
    app.saved();
    let Some(gap) = app.vault().into_iter().find(|p| p.ends_with("gap.md")) else {
        return Err(format!("no gap.md on disk. vault: {:?}", app.vault()));
    };
    let now = std::fs::read_to_string(app.dir.join(&gap)).map_err(|e| format!("{e}"))?;
    let mut missing = Vec::new();
    if !number_near(&now, 87.0, 0.0) {
        missing.push("87");
    }
    if !number_near(&now, 12.4, 0.05) {
        missing.push("12.4");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("{WRONG}gap.md is missing {missing:?}: {now:?}"))
    }
}

/// Take the change on offer, or say what the model said instead.
fn taken(app: &mut App) -> Result<(), String> {
    if accept_all(app) == 0 {
        return Err(format!(
            "{WRONG}it proposed nothing: {:?}",
            last_answer(app).chars().take(140).collect::<String>()
        ));
    }
    app.saved();
    Ok(())
}

/// The note by that name, wherever in the vault it landed, as it is on disk.
fn on_disk(app: &App, name: &str) -> Result<(PathBuf, String), String> {
    let Some(found) = app
        .vault()
        .into_iter()
        .find(|p| p.ends_with(name) || p.ends_with(&format!("/{name}")))
    else {
        return Err(format!("no {name} on disk. vault: {:?}", app.vault()));
    };
    let path = app.dir.join(&found);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    Ok((path, text))
}

/// A list built up a line at a time, corrected, added up, changed behind the
/// model's back, added up again, and put in order.
///
/// The long way round, which is how a note actually gets made: three things
/// added one after another, one of them wrong and put right in the next
/// breath, then questions about what is there, then somebody else changes the
/// file, then more questions that are only right if that change was read.
/// Every step depends on the ones before it having landed where they should.
fn a_list_built_corrected_and_reckoned_up(app: &mut App) -> Result<(), String> {
    looking_at(app, "", "spend")?;
    fresh_chat(app)?;

    // Built up, one line at a time.
    asking(
        app,
        "make a note called shop.md with a shopping list. first item: milk, 2.50",
    )?;
    taken(app)?;
    let (path, text) = on_disk(app, "shop.md")?;
    if !text.to_lowercase().contains("milk") {
        return Err(format!("{WRONG}milk is not in it: {text:?}"));
    }
    // Each one once. An edit that rewrites the tail of the list from the
    // wrong line brings an item back a second time, and the sum three steps
    // later is then right for the wrong list - so the check is here.
    let once_each = |text: &str, items: &[&str]| -> Result<(), String> {
        let low = text.to_lowercase();
        for item in items {
            match low.matches(item).count() {
                1 => {}
                0 => return Err(format!("{WRONG}{item} did not land: {text:?}")),
                n => return Err(format!("{WRONG}{item} is in the list {n} times: {text:?}")),
            }
        }
        Ok(())
    };
    asking(app, "add bread, 1.80")?;
    taken(app)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    once_each(&text, &["milk", "bread"])?;
    asking(app, "add eggs, 3.20")?;
    taken(app)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    once_each(&text, &["milk", "bread", "eggs"])?;

    // Put right in the next breath. The edit has to find the line it just
    // added, in a file it has now changed twice.
    asking(app, "oh wait, not eggs - that should be tofu, same price")?;
    taken(app)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    if text.to_lowercase().contains("eggs") {
        return Err(format!("{WRONG}the correction did not take: {text:?}"));
    }
    once_each(&text, &["milk", "bread", "tofu"])?;

    // Added up.
    asking(app, "what does the list come to in total?")?;
    let answer = last_answer(app);
    if !number_near(&answer, 7.5, 0.01) {
        return Err(format!("{WRONG}7.50, it said: {answer:?}"));
    }

    // Changed by somebody else, in two places, with the conversation open.
    std::thread::sleep(Duration::from_millis(1100));
    let mut outside = text.replace("2.50", "2.20");
    if outside == text {
        return Err(format!(
            "the list is not in a shape that can be edited: {text:?}"
        ));
    }
    // A file saved from a block the model wrote has no newline on the end,
    // and a line added straight after it is glued to the last one.
    if !outside.ends_with('\n') {
        outside.push('\n');
    }
    outside.push_str("- cheese, 4.00\n");
    std::fs::write(&path, &outside).map_err(|e| format!("{e}"))?;
    app.steps(90);

    // Only right if the change was read: 2.20 + 1.80 + 3.20 + 4.00.
    asking(app, "and what does it come to now?")?;
    let answer = last_answer(app);
    if !number_near(&answer, 11.2, 0.01) {
        return Err(format!(
            "{WRONG}11.20 after the change outside, it said: {answer:?}"
        ));
    }

    // Put in order, which is a change to every line at once.
    asking(
        app,
        "sort the list from the most expensive item to the cheapest",
    )?;
    taken(app)?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    let low = text.to_lowercase();
    let at = |item: &str| {
        low.find(item)
            .ok_or_else(|| format!("{WRONG}{item} was dropped in the sorting: {text:?}"))
    };
    let (cheese, tofu, milk, bread) = (at("cheese")?, at("tofu")?, at("milk")?, at("bread")?);
    if !(cheese < tofu && tofu < milk && milk < bread) {
        return Err(format!("{WRONG}not in order: {text:?}"));
    }
    for price in ["4.00", "3.20", "2.20", "1.80"] {
        if !text.contains(price) {
            return Err(format!("{WRONG}{price} was lost in the sorting: {text:?}"));
        }
    }

    // And one more question that leans on all of it.
    asking(
        app,
        "which item is the cheapest, and what share of the total is it, to one decimal place?",
    )?;
    let answer = last_answer(app);
    if !answer.to_lowercase().contains("bread") || !number_near(&answer, 16.1, 0.1) {
        return Err(format!("{WRONG}bread at 16.1%, it said: {answer:?}"));
    }
    Ok(())
}

/// Figures read out of another project and brought into this one.
///
/// Reading reaches the whole vault; changing reaches the project on screen.
/// The two together are how a number gets from one note to another.
fn figures_from_another_project_brought_here(app: &mut App) -> Result<(), String> {
    let garden = app.dir.join("garden");
    std::fs::create_dir_all(&garden).map_err(|e| format!("{e}"))?;
    let harvest = "# Harvest\n\nThis year so far.\n\n- Tomatoes: 12\n- Beans: 7\n- Peppers: 4\n";
    std::fs::write(garden.join("harvest.md"), harvest).map_err(|e| format!("{e}"))?;
    looking_at(app, "new-one", "zzqqseed")?;
    fresh_chat(app)?;

    asking(
        app,
        "read garden/harvest.md and tell me how many things were harvested in all",
    )?;
    let answer = last_answer(app);
    if !number_near(&answer, 23.0, 0.0) {
        return Err(format!("{WRONG}23, it said: {answer:?}"));
    }
    asking(
        app,
        "make a note here called totals.md saying the harvest total is that number",
    )?;
    taken(app)?;
    let totals = app.dir.join("new-one").join("totals.md");
    if !totals.exists() {
        return Err(format!(
            "totals.md is not in this project. vault: {:?}",
            app.vault()
        ));
    }
    let text = std::fs::read_to_string(&totals).map_err(|e| format!("{e}"))?;
    if !number_near(&text, 23.0, 0.0) {
        return Err(format!(
            "{WRONG}the total did not make it into the note: {text:?}"
        ));
    }
    let still = std::fs::read_to_string(garden.join("harvest.md")).map_err(|e| format!("{e}"))?;
    if still != harvest {
        return Err(format!(
            "the note in the other project was changed: {still:?}"
        ));
    }
    Ok(())
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
    // In a project the editor is not looking at. A note in the open project
    // is written out whole at the top of the conversation, so there was
    // nothing to read: a model that answered from the page was marked wrong
    // for not fetching what it had been handed.
    let elsewhere = app.dir.join("outside");
    std::fs::create_dir_all(&elsewhere).map_err(|e| format!("{e}"))?;
    let note = elsewhere.join("weather.md");
    // The fact is kept off the note's first line, because the first line is
    // in the list at the top of every conversation and a model can answer
    // from that without reading anything - and one did.
    std::fs::write(
        &note,
        "# Weather\n\nChecked at breakfast.\n\nIt is RAINING today.\n",
    )
    .map_err(|e| format!("{e}"))?;
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

/// A long note changed in two places at once, and read as a diff.
///
/// The case the diff exists for. Two words changed a hundred and ninety lines
/// apart used to arrive as the hundred and ninety-one lines between them; they
/// arrive now as two hunks of a unified diff, which is a shape these models
/// have read more of than almost anything else. What is checked here is not
/// the size of it but that the model can still answer from it - both ends of
/// it, in one question, having been shown neither line in full.
fn a_long_note_changed_in_two_places(app: &mut App) -> Result<(), String> {
    let note = app.dir.join("house.md");
    let write = |kettle: &str, tap: &str| {
        let mut lines: Vec<String> = (1..=200)
            .map(|n| format!("Room note number {n}."))
            .collect();
        lines[0] = "# House".to_string();
        lines[4] = format!("The kettle is {kettle}.");
        lines[194] = format!("The tap is {tap}.");
        std::fs::write(&note, lines.join("\n") + "\n").map_err(|e| format!("{e}"))
    };
    write("BROKEN", "DRIPPING")?;
    app.steps(90);
    fresh_chat(app)?;
    // Asked once, so the whole file is written out at the front and the model
    // has been shown it. Everything after this is measured against that.
    asking(app, "in house.md, what is wrong with the kettle? one word.")?;
    let said = last_answer(app).to_lowercase();
    if !said.contains("broken") {
        return Err(format!("{WRONG}it should say broken: {said:?}"));
    }
    // Two lines, a hundred and ninety apart, changed by somebody else.
    std::thread::sleep(Duration::from_millis(1100));
    write("FIXED", "SEALED")?;
    app.steps(90);
    asking(
        app,
        "in house.md, what is the kettle and what is the tap now? two words.",
    )?;
    let said = last_answer(app).to_lowercase();
    let missing: Vec<&str> = ["fixed", "sealed"]
        .into_iter()
        .filter(|want| !said.contains(want))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{WRONG}the diff was not read - missing {missing:?}: {said:?}"
        ))
    }
}

/// Close the conversation and open it again from the list.
///
/// The first row of the list is a new conversation; the one after it is the
/// most recent. Coming back to a conversation is a different path from
/// starting one - what it was shown is gone and has to be shown again - and
/// a follow-up that leans on what was said before is the test of it.
fn reopened(app: &mut App) -> Result<(), String> {
    if app.app.chat.is_some() {
        chatting(app)?;
        app.key(Key::Escape);
        app.steps(4);
    }
    app.press(Key::Char('e'), cmd(Key::Char('e')));
    app.key(Key::Escape);
    app.press(Key::Enter, cmd(Key::Enter));
    app.steps(4);
    if app.app.chat.is_none() {
        app.click("chat1")?;
        app.steps(6);
    }
    let turns = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    if turns == 0 {
        return Err(format!(
            "the conversation did not come back. on screen: {:?}",
            app.on_screen()
        ));
    }
    let _ = app.click("chat-field");
    Ok(())
}

/// Two notes folded into one, in a single step.
///
/// The one verb that changes two files at once, and the reason it exists: a
/// write plus a delete are answered separately, and half of that is a note
/// duplicated or a note lost. Never tested end to end until now.
fn two_notes_folded_into_one(app: &mut App) -> Result<(), String> {
    let (a, b) = (app.dir.join("monday.md"), app.dir.join("tuesday.md"));
    std::fs::write(&a, "# Monday\n\n- Swim\n").map_err(|e| format!("{e}"))?;
    std::fs::write(&b, "# Tuesday\n\n- Climb\n").map_err(|e| format!("{e}"))?;
    looking_at(app, "", "monday")?;
    fresh_chat(app)?;
    asking(
        app,
        "merge monday.md and tuesday.md into one note called week.md, keeping both lists",
    )?;
    taken(app)?;
    let (_, week) = on_disk(app, "week.md").map_err(|e| {
        let notes: Vec<String> = app
            .app
            .notes
            .iter()
            .map(|n| format!("{}{}", n.filename(), if n.buffer.dirty { "*" } else { "" }))
            .collect();
        format!(
            "{e}\n      in memory: {notes:?}, status: {:?}",
            app.app.status
        )
    })?;
    if !week.contains("Swim") || !week.contains("Climb") {
        return Err(format!("{WRONG}week.md is missing a list: {week:?}"));
    }
    // A write of the new note with the old ones left standing is half a
    // merge, which the instructions name as the thing not to do. The model
    // chose the verb; the application did as it was told.
    if a.exists() || b.exists() {
        return Err(format!(
            "{WRONG}the parts are still there after the merge: monday {} tuesday {}",
            a.exists(),
            b.exists()
        ));
    }
    Ok(())
}

/// A note taken away on request here, and one in another project left alone.
fn a_note_deleted_here_and_not_there(app: &mut App) -> Result<(), String> {
    let gone = app.dir.join("scratch.md");
    std::fs::write(&gone, "# Scratch\n\nnothing much\n").map_err(|e| format!("{e}"))?;
    let garden = app.dir.join("garden");
    std::fs::create_dir_all(&garden).map_err(|e| format!("{e}"))?;
    let kept = garden.join("seeds.md");
    std::fs::write(&kept, "# Seeds\n\nbeans\n").map_err(|e| format!("{e}"))?;
    looking_at(app, "", "spend")?;
    fresh_chat(app)?;

    asking(app, "delete scratch.md")?;
    taken(app)?;
    if gone.exists() {
        return Err("scratch.md is still on disk after the delete was taken".into());
    }
    asking(app, "now delete garden/seeds.md as well")?;
    accept_all(app);
    if !kept.exists() {
        return Err("a note in another project was deleted".into());
    }
    Ok(())
}

/// A change turned down, then a different one asked for.
///
/// What was refused must not come back, and the next request must land.
fn turned_down_then_asked_for_differently(app: &mut App) -> Result<(), String> {
    let path = app.dir.join("motto.md");
    let started = "# Motto\n\nSlow and steady.\n";
    std::fs::write(&path, started).map_err(|e| format!("{e}"))?;
    looking_at(app, "", "motto")?;
    fresh_chat(app)?;

    asking(app, "change the motto to 'Fast and loose.'")?;
    // Turned down, by the button.
    app.scroll_to_end();
    if app.click("REJECT").is_err() {
        return Err(format!(
            "{WRONG}nothing was proposed to turn down: {:?}",
            last_answer(app).chars().take(120).collect::<String>()
        ));
    }
    app.steps(6);
    app.saved();
    let now = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    if now != started {
        return Err(format!("a change that was turned down was made: {now:?}"));
    }

    asking(app, "no - make it 'Measure twice, cut once.'")?;
    taken(app)?;
    let now = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    if !now.contains("Measure twice") {
        return Err(format!("{WRONG}the second request did not land: {now:?}"));
    }
    if now.contains("Fast and loose") {
        return Err(format!("{WRONG}the refused text came back: {now:?}"));
    }
    Ok(())
}

/// An answer stopped halfway, and the next question still answered.
///
/// While the model writes there is no box to type in, only a button to stop
/// it. Pressing that keeps what was written so far, and the conversation has
/// to carry on afterwards as if nothing had happened - which means the
/// model's memory of the question has to be put right, since it was cut off.
fn an_answer_stopped_and_the_next_still_comes(app: &mut App) -> Result<(), String> {
    fresh_chat(app)?;
    let before = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    app.typed("write me a long story about a lighthouse keeper, at least four paragraphs");
    app.key(Key::Enter);
    // Until it has said something, then stop it.
    let began = Instant::now();
    loop {
        app.step();
        std::thread::sleep(Duration::from_millis(5));
        let said = app.app.helper.partial().split_whitespace().count();
        if said >= 12 {
            break;
        }
        if began.elapsed() > PATIENCE {
            return Err("it never started writing".into());
        }
    }
    if app.ui.find("chat-field").is_some() {
        return Err("the box is there to type in while the model is answering".into());
    }
    app.click("STOP")?;
    app.wait()?;
    let after = app.app.chat.as_ref().map(|c| c.turns.len()).unwrap_or(0);
    if after != before + 2 {
        return Err(format!(
            "stopping lost the answer so far: {before} turns before, {after} after"
        ));
    }
    let kept = last_answer(app);
    if kept.split_whitespace().count() < 6 {
        return Err(format!(
            "what was written before the stop is gone: {kept:?}"
        ));
    }
    // And the conversation carries on.
    asking(app, "never mind the story. what is 12 * 12?")?;
    let answer = last_answer(app);
    if !number_near(&answer, 144.0, 0.0) {
        return Err(format!("{WRONG}144, it said: {answer:?}"));
    }
    Ok(())
}

/// Letters in other alphabets survive being copied by the model.
///
/// The reply is folded to what the font can draw, and it used to drop every
/// letter it had no glyph for - so a name copied into a new note came back
/// misspelt and was written to disk that way. Now the letters stay and the
/// editor draws a question mark for the ones it cannot; the file is right.
fn letters_in_other_alphabets_survive_a_copy(app: &mut App) -> Result<(), String> {
    let path = app.dir.join("names.md");
    std::fs::write(
        &path,
        "# Names\n\nPeople to thank:\n\n- Müller, who fixed the café's sign\n- Данила, who painted it\n",
    )
    .map_err(|e| format!("{e}"))?;
    looking_at(app, "", "names")?;
    fresh_chat(app)?;
    asking(
        app,
        "make a note called thanks.md containing exactly the two names from the list in names.md, one per line",
    )?;
    taken(app)?;
    let (_, thanks) = on_disk(app, "thanks.md")?;
    for want in ["Müller", "Данила"] {
        if !thanks.contains(want) {
            return Err(format!("{want} did not survive the copy: {thanks:?}"));
        }
    }
    Ok(())
}

/// A section added to the end of a note, and an item moved within it.
///
/// Structure, not values: a heading and a list put in below what is there,
/// and then one line of that list moved to the top of it. The second is the
/// edit models find hardest - a line has to go away in one place and come
/// back in another, in a single block.
fn a_section_added_then_reordered(app: &mut App) -> Result<(), String> {
    let path = app.dir.join("plan.md");
    std::fs::write(&path, "# Plan\n\nA quiet week.\n").map_err(|e| format!("{e}"))?;
    looking_at(app, "", "plan")?;
    fresh_chat(app)?;

    asking(
        app,
        "add a section called Todo at the end with three items: buy milk, call mum, fix the tap",
    )?;
    taken(app)?;
    let now = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    let low = now.to_lowercase();
    if !low.contains("todo") || !low.contains("buy milk") || !low.contains("fix the tap") {
        return Err(format!("{WRONG}the section did not land: {now:?}"));
    }
    if !low.contains("a quiet week") {
        return Err(format!(
            "{WRONG}adding a section took the note with it: {now:?}"
        ));
    }

    asking(app, "move 'fix the tap' to the top of the todo list")?;
    taken(app)?;
    let now = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    let low = now.to_lowercase();
    let at = |item: &str| {
        low.find(item)
            .ok_or_else(|| format!("{WRONG}{item} is gone after the move: {now:?}"))
    };
    let (tap, milk, mum) = (at("fix the tap")?, at("buy milk")?, at("call mum")?);
    if !(tap < milk && tap < mum) {
        return Err(format!("{WRONG}not moved to the top: {now:?}"));
    }
    if low.matches("fix the tap").count() != 1 {
        return Err(format!("{WRONG}moved by copying, not by moving: {now:?}"));
    }
    Ok(())
}

/// Three notes read for one answer.
///
/// Several lookups in a single question, all of them reads, and the answer
/// is only right if every one came back and was added up.
fn three_notes_read_for_one_answer(app: &mut App) -> Result<(), String> {
    let barn = app.dir.join("barn");
    std::fs::create_dir_all(&barn).map_err(|e| format!("{e}"))?;
    for (name, text) in [
        (
            "hens.md",
            "# Hens\n\nCounted at dusk.\n\nThere are 14 hens.\n",
        ),
        (
            "goats.md",
            "# Goats\n\nCounted at dusk.\n\nThere are 5 goats.\n",
        ),
        (
            "geese.md",
            "# Geese\n\nCounted at dusk.\n\nThere are 9 geese.\n",
        ),
    ] {
        std::fs::write(barn.join(name), text).map_err(|e| format!("{e}"))?;
    }
    looking_at(app, "new-one", "zzqqseed")?;
    fresh_chat(app)?;
    asking(
        app,
        "read barn/hens.md, barn/goats.md and barn/geese.md and tell me how many animals there are in all",
    )?;
    let answer = last_answer(app);
    if !number_near(&answer, 28.0, 0.0) {
        return Err(format!("{WRONG}28, it said: {answer:?}"));
    }
    Ok(())
}

/// A conversation closed, opened again, and carried on.
fn a_conversation_reopened_carries_on(app: &mut App) -> Result<(), String> {
    fresh_chat(app)?;
    asking(
        app,
        "how many days are there from 1 March 2024 to 1 March 2025?",
    )?;
    let answer = last_answer(app);
    if !number_near(&answer, 365.0, 0.0) {
        return Err(format!("{WRONG}365, it said: {answer:?}"));
    }
    reopened(app)?;
    // A follow-up with nothing in it but "that": only right if the
    // conversation came back with what it had said.
    asking(app, "and that in weeks, to one decimal place?")?;
    let answer = last_answer(app);
    if !number_near(&answer, 52.1, 0.05) {
        return Err(format!("{WRONG}52.1 after reopening, it said: {answer:?}"));
    }
    Ok(())
}

/// A change taken, then undone in the editor, and the model asked about it.
///
/// Accepting a change is an edit like any other and `u` takes it back. The
/// model was told its change was taken; what it must not do is go on
/// believing it after the person changed their mind by hand.
fn a_change_undone_in_the_editor_is_seen(app: &mut App) -> Result<(), String> {
    let path = app.dir.join("door.md");
    std::fs::write(&path, "# Door\n\nThe door is BLUE.\n").map_err(|e| format!("{e}"))?;
    looking_at(app, "", "door")?;
    fresh_chat(app)?;
    asking(app, "make the door GREEN")?;
    taken(app)?;
    let now = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    if !now.contains("GREEN") {
        return Err(format!("{WRONG}the change did not land: {now:?}"));
    }
    // Back to the editor, and undo it by hand.
    app.key(Key::Escape);
    app.steps(4);
    app.press(Key::Char('e'), cmd(Key::Char('e')));
    app.key(Key::Escape);
    app.key(Key::Char('u'));
    app.saved();
    let now = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    if !now.contains("BLUE") {
        return Err(format!(
            "undo in the editor did not put the door back: {now:?}"
        ));
    }
    reopened(app)?;
    asking(app, "what colour is the door now? one word.")?;
    said(app, &["blue"]).map_err(|e| format!("{WRONG}after the undo, {e}"))
}

/// A project of three notes that depend on one another, laid down fresh.
///
/// Prices in one, orders in another, a summary in the third that is only
/// right if the other two are what they are. Anything worth asking about it
/// touches more than one file, and anything worth changing does too.
fn farm(app: &mut App) -> Result<PathBuf, String> {
    let farm = app.dir.join("farm");
    std::fs::create_dir_all(&farm).map_err(|e| format!("{e}"))?;
    for (name, text) in [
        (
            "prices.md",
            "# Prices\n\nPer unit, this season.\n\n| Item | Unit price |\n| --- | --- |\n\
             | Eggs | 0.50 |\n| Milk | 1.20 |\n| Bread | 2.00 |\n",
        ),
        (
            "orders.md",
            "# Orders\n\nThis week.\n\n- Alice: 12 eggs, 2 milk\n- Bob: 6 eggs, 1 bread\n",
        ),
        (
            "summary.md",
            "# Summary\n\nBest customer: Alice.\n\n- Egg price: 0.50\n- Revenue from eggs: 9.00\n\
             - Total revenue: 13.40\n",
        ),
    ] {
        std::fs::write(farm.join(name), text).map_err(|e| format!("{e}"))?;
    }
    looking_at(app, "farm", "prices")?;
    fresh_chat(app)?;
    Ok(farm)
}

/// Questions whose answers are spread across three notes.
///
/// The prices are in one, the orders in another; what a customer owes is in
/// neither. Three questions, each needing both, the last needing the answers
/// to the first two - and the third note says what the right answer is.
fn an_analysis_across_three_notes(app: &mut App) -> Result<(), String> {
    farm(app)?;
    asking(app, "how much does Alice owe, at the prices in prices.md?")?;
    let answer = last_answer(app);
    if !number_near(&answer, 8.4, 0.01) {
        return Err(format!("{WRONG}8.40, it said: {answer:?}"));
    }
    asking(app, "and Bob?")?;
    let answer = last_answer(app);
    if !number_near(&answer, 5.0, 0.01) {
        return Err(format!("{WRONG}5.00, it said: {answer:?}"));
    }
    asking(
        app,
        "so what is the total revenue this week, and does summary.md agree?",
    )?;
    let answer = last_answer(app);
    if !number_near(&answer, 13.4, 0.01) {
        return Err(format!("{WRONG}13.40, it said: {answer:?}"));
    }
    let low = answer.to_lowercase();
    if low.contains("does not agree") || low.contains("doesn't agree") || low.contains("disagree") {
        return Err(format!(
            "{WRONG}it said the summary was wrong when it was right: {answer:?}"
        ));
    }
    Ok(())
}

/// One value changed, and every note that depends on it changed with it.
///
/// The egg price lives in prices.md and is written into summary.md, and two
/// figures in summary.md are worked out from it. One request has to reach
/// both files, in one reply, and leave the third alone.
fn a_value_changed_in_every_note_it_reaches(app: &mut App) -> Result<(), String> {
    let farm = farm(app)?;
    let orders = std::fs::read_to_string(farm.join("orders.md")).map_err(|e| format!("{e}"))?;
    asking(
        app,
        "change the egg price to 0.75 everywhere it matters in this project, and fix every \
         figure that depends on it",
    )?;
    taken(app)?;
    let prices = std::fs::read_to_string(farm.join("prices.md")).map_err(|e| format!("{e}"))?;
    let summary = std::fs::read_to_string(farm.join("summary.md")).map_err(|e| format!("{e}"))?;
    let mut missing = Vec::new();
    if !prices.contains("0.75") || prices.contains("0.50") {
        missing.push("the price in prices.md");
    }
    if !summary.contains("0.75") {
        missing.push("the price in summary.md");
    }
    // 18 eggs at 0.75, and 13.50 + 2.40 + 2.00.
    if !number_near(&summary, 13.5, 0.01) {
        missing.push("the egg revenue (13.50)");
    }
    if !number_near(&summary, 17.9, 0.01) {
        missing.push("the total (17.90)");
    }
    if !missing.is_empty() {
        return Err(format!(
            "{WRONG}missing {missing:?}:\nprices: {prices:?}\nsummary: {summary:?}"
        ));
    }
    let still = std::fs::read_to_string(farm.join("orders.md")).map_err(|e| format!("{e}"))?;
    if still != orders {
        return Err(format!(
            "{WRONG}orders.md has no prices in it and was changed anyway: {still:?}"
        ));
    }
    Ok(())
}

/// A name changed in every note that has it, and not in the one that does not.
fn a_name_changed_across_the_project(app: &mut App) -> Result<(), String> {
    let farm = farm(app)?;
    let prices = std::fs::read_to_string(farm.join("prices.md")).map_err(|e| format!("{e}"))?;
    asking(app, "Alice is now called Alicia - update this project")?;
    taken(app)?;
    for name in ["orders.md", "summary.md"] {
        let text = std::fs::read_to_string(farm.join(name)).map_err(|e| format!("{e}"))?;
        if text.contains("Alice") && !text.contains("Alicia") {
            return Err(format!("{WRONG}{name} still says Alice: {text:?}"));
        }
        if text.contains("Alice:") || text.contains("Alice.") {
            return Err(format!("{WRONG}{name} was missed: {text:?}"));
        }
        if !text.contains("Alicia") {
            return Err(format!("{WRONG}{name} lost the name altogether: {text:?}"));
        }
    }
    let still = std::fs::read_to_string(farm.join("prices.md")).map_err(|e| format!("{e}"))?;
    if still != prices {
        return Err(format!(
            "{WRONG}prices.md has nobody's name in it and was changed anyway: {still:?}"
        ));
    }
    Ok(())
}

/// A summary that has drifted from the notes it summarises, found and fixed.
///
/// The total in summary.md is made wrong first. Asked whether the summary is
/// right, the model has to work the figures out from the other two notes,
/// notice, and change the one line that is off - and nothing else anywhere.
fn a_summary_that_drifted_is_put_right(app: &mut App) -> Result<(), String> {
    let farm = farm(app)?;
    let summary = farm.join("summary.md");
    let drifted = std::fs::read_to_string(&summary)
        .map_err(|e| format!("{e}"))?
        .replace("13.40", "12.40");
    std::fs::write(&summary, &drifted).map_err(|e| format!("{e}"))?;
    let prices = std::fs::read_to_string(farm.join("prices.md")).map_err(|e| format!("{e}"))?;
    let orders = std::fs::read_to_string(farm.join("orders.md")).map_err(|e| format!("{e}"))?;
    app.steps(90);
    asking(
        app,
        "check summary.md against prices.md and orders.md, and fix whatever is wrong in it",
    )?;
    taken(app)?;
    let now = std::fs::read_to_string(&summary).map_err(|e| format!("{e}"))?;
    if !now.contains("13.40") || now.contains("12.40") {
        return Err(format!("{WRONG}the total was not put right: {now:?}"));
    }
    if !now.contains("9.00") || !now.contains("0.50") || !now.contains("Alice") {
        return Err(format!(
            "{WRONG}something that was right was changed: {now:?}"
        ));
    }
    for (name, was) in [("prices.md", prices), ("orders.md", orders)] {
        let still = std::fs::read_to_string(farm.join(name)).map_err(|e| format!("{e}"))?;
        if still != was {
            return Err(format!(
                "{WRONG}{name} was right and was changed: {still:?}"
            ));
        }
    }
    Ok(())
}

/// A note found by part of its name, with the find tool.
fn a_note_found_by_part_of_its_name(app: &mut App) -> Result<(), String> {
    for (project, name) in [
        ("garden", "roses.md"),
        ("garden", "rosemary.md"),
        ("kitchen", "rosehip-tea.md"),
    ] {
        std::fs::create_dir_all(app.dir.join(project)).map_err(|e| format!("{e}"))?;
        std::fs::write(app.dir.join(project).join(name), "# A note\n\nAbout it.\n")
            .map_err(|e| format!("{e}"))?;
    }
    looking_at(app, "new-one", "zzqqseed")?;
    fresh_chat(app)?;
    asking(
        app,
        "which notes have 'rose' in their name? list each one's path and how many lines it has.",
    )?;
    let used = looked_with(app, "find");
    let answer = last_answer(app).to_lowercase();
    let missing: Vec<&str> = [
        "garden/roses.md",
        "garden/rosemary.md",
        "kitchen/rosehip-tea.md",
    ]
    .into_iter()
    .filter(|p| !answer.contains(p))
    .collect();
    if !missing.is_empty() {
        return Err(format!("{WRONG}missing {missing:?}: {answer:?}"));
    }
    if !used {
        return Err(format!("{WRONG}it answered without the find tool"));
    }
    Ok(())
}

/// Where a word is said, with the grep tool.
///
/// The word is well down in two notes in two projects, off the first line
/// the list at the top shows, so nothing but a search can find it.
fn where_a_word_is_said(app: &mut App) -> Result<(), String> {
    for (project, name, text) in [
        (
            "garden",
            "beds.md",
            "# Beds\n\nThe long bed.\n\nPelargoniums by the wall.\n",
        ),
        (
            "kitchen",
            "sills.md",
            "# Sills\n\nSouth-facing.\n\nA pelargonium in the blue pot.\n",
        ),
        (
            "garden",
            "shed.md",
            "# Shed\n\nTools.\n\nSpades and forks.\n",
        ),
    ] {
        std::fs::create_dir_all(app.dir.join(project)).map_err(|e| format!("{e}"))?;
        std::fs::write(app.dir.join(project).join(name), text).map_err(|e| format!("{e}"))?;
    }
    looking_at(app, "new-one", "zzqqseed")?;
    fresh_chat(app)?;
    asking(app, "which notes mention a pelargonium? name each note.")?;
    let used = looked_with(app, "grep");
    let answer = last_answer(app).to_lowercase();
    if !answer.contains("beds.md") || !answer.contains("sills.md") {
        return Err(format!("{WRONG}beds.md and sills.md, it said: {answer:?}"));
    }
    if answer.contains("shed.md") {
        return Err(format!("{WRONG}shed.md does not mention them: {answer:?}"));
    }
    if !used {
        return Err(format!("{WRONG}it answered without the grep tool"));
    }
    Ok(())
}

// ---------------------------------------------------------- long sessions
//
// Forty or fifty questions in one conversation, each checking the one thing
// it changed or asked. The short scenes cannot find what these find: a file
// the model made and was then told nothing about, a thought it stopped
// having after ten turns of not having one, a grep whose line numbers had
// moved on. The machinery is shared; the conversations differ.

/// A note's text, read from disk by name, relative to the scene's project.
type Read<'a> = &'a dyn Fn(&str) -> String;
/// What a step checks, given the answer and a way to read a note.
type Check = Box<dyn Fn(&str, Read) -> Result<(), String>>;
/// Something done to the vault from outside, before a step.
type Outside = Box<dyn Fn(&Path)>;

/// What to do after asking.
enum Then {
    Nothing,
    Accept,
    /// Accepted - and if nothing was offered, said again with the nudge a
    /// person would give, and accepted then. Counted as a wrong answer
    /// either way; a step that needs the nudge is a step the model got
    /// wrong the first time.
    AcceptOr(&'static str),
    Reject,
}

struct Step {
    ask: &'static str,
    then: Then,
    before: Option<Outside>,
    check: Check,
}

fn step(ask: &'static str, then: Then, check: Check) -> Step {
    Step {
        ask,
        then,
        before: None,
        check,
    }
}

/// The note has every one of these in it, case aside.
fn file_has(name: &'static str, wants: &'static [&'static str]) -> Check {
    Box::new(move |_: &str, read: Read| {
        let text = read(name);
        let low = text.to_lowercase();
        for w in wants {
            if !low.contains(&w.to_lowercase()) {
                return Err(format!("{WRONG}{name} is missing {w:?}: {text:?}"));
            }
        }
        Ok(())
    })
}

/// The note no longer has this in it.
fn file_lacks(name: &'static str, gone: &'static str) -> Check {
    Box::new(move |_: &str, read: Read| {
        let text = read(name);
        if text.to_lowercase().contains(&gone.to_lowercase()) {
            Err(format!("{WRONG}{name} still has {gone:?}: {text:?}"))
        } else {
            Ok(())
        }
    })
}

/// The note is not there at all.
fn file_gone(name: &'static str) -> Check {
    Box::new(move |_: &str, read: Read| {
        let text = read(name);
        if text.is_empty() {
            Ok(())
        } else {
            Err(format!("{WRONG}{name} is still there: {text:?}"))
        }
    })
}

/// The answer has this number in it.
fn number(want: f64, tol: f64) -> Check {
    Box::new(move |said: &str, _: Read| {
        if number_near(said, want, tol) {
            Ok(())
        } else {
            Err(format!("{WRONG}{want}, it said: {said:?}"))
        }
    })
}

/// The answer has every one of these words in it, case aside.
fn says(wants: &'static [&'static str]) -> Check {
    Box::new(move |said: &str, _: Read| {
        let low = said.to_lowercase();
        for w in wants {
            if !low.contains(&w.to_lowercase()) {
                return Err(format!("{WRONG}expected {w:?}: {said:?}"));
            }
        }
        Ok(())
    })
}

/// Both at once.
fn both(a: Check, b: Check) -> Check {
    Box::new(move |said: &str, read: Read| {
        a(said, read)?;
        b(said, read)
    })
}

fn ok() -> Check {
    Box::new(|_: &str, _: Read| Ok(()))
}

/// One string in a note swapped for another, from outside the app.
fn outside(name: &'static str, from: &'static str, to: &'static str) -> Outside {
    Box::new(move |dir: &Path| {
        let path = dir.join(name);
        if let Ok(t) = std::fs::read_to_string(&path) {
            let _ = std::fs::write(&path, t.replace(from, to));
        }
    })
}

/// A line put on the end of a note, from outside the app.
fn appended(name: &'static str, line: &'static str) -> Outside {
    Box::new(move |dir: &Path| {
        let path = dir.join(name);
        if let Ok(mut t) = std::fs::read_to_string(&path) {
            if !t.ends_with('\n') {
                t.push('\n');
            }
            t.push_str(line);
            t.push('\n');
            let _ = std::fs::write(&path, t);
        }
    })
}

/// A note made, or taken away, from outside the app.
fn made_outside(name: &'static str, text: &'static str) -> Outside {
    Box::new(move |dir: &Path| {
        let _ = std::fs::write(dir.join(name), text);
    })
}

fn removed_outside(name: &'static str) -> Outside {
    Box::new(move |dir: &Path| {
        let _ = std::fs::remove_file(dir.join(name));
    })
}

/// A long conversation, run and then audited.
struct Long {
    /// The project's folder, which `Read` reads relative to.
    folder: PathBuf,
    steps: Vec<Step>,
    /// Before which steps the conversation is closed and opened again.
    reopen_at: &'static [usize],
}

fn long_session(app: &mut App, long: Long) -> Result<(), String> {
    let Long {
        folder,
        steps,
        reopen_at,
    } = long;
    let read =
        |name: &str| -> String { std::fs::read_to_string(folder.join(name)).unwrap_or_default() };
    let count = steps.len();
    let began = Instant::now();
    let mut times = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    for (i, s) in steps.iter().enumerate() {
        if reopen_at.contains(&i) {
            // Closed and opened again, in the middle of it all.
            reopened(app)?;
        }
        if let Some(before) = &s.before {
            std::thread::sleep(Duration::from_millis(1100));
            before(&folder);
            app.steps(90);
        }
        let at = Instant::now();
        asking(app, s.ask).map_err(|e| format!("step {}: {e}", i + 1))?;
        match s.then {
            Then::Nothing => {}
            Then::Accept => taken(app).map_err(|e| format!("step {}: {e}", i + 1))?,
            Then::AcceptOr(nudge) => {
                if taken(app).is_err() {
                    wrong.push(format!(
                        "step {}: nothing was offered until nudged: {:?}",
                        i + 1,
                        last_answer(app).chars().take(140).collect::<String>()
                    ));
                    asking(app, nudge).map_err(|e| format!("step {}: {e}", i + 1))?;
                    taken(app).map_err(|e| format!("step {}, nudged: {e}", i + 1))?;
                }
            }
            Then::Reject => {
                app.scroll_to_end();
                if app.click("REJECT").is_err() {
                    wrong.push(format!("step {}: nothing was proposed to turn down", i + 1));
                } else {
                    app.steps(6);
                    app.saved();
                }
            }
        }
        times.push(at.elapsed().as_secs_f32());
        let answer = last_answer(app);
        // A wrong answer is written down and the conversation goes on, as it
        // would: one miscounted day at step sixteen must not hide the
        // thirty-four steps after it. Anything that is not the model's
        // doing stops it here.
        match (s.check)(&answer, &read) {
            Ok(()) => {}
            Err(why) if why.starts_with(WRONG) => {
                wrong.push(format!("step {}: {}", i + 1, why.trim_start_matches(WRONG)));
            }
            Err(why) => return Err(format!("step {}: {why}", i + 1)),
        }
    }
    let ten = count.min(10);
    println!(
        "\n      {count} steps in {:.0}s; slowest {:.1}s, median {:.1}s, first ten {:.1}s, last ten {:.1}s",
        began.elapsed().as_secs_f32(),
        times.iter().cloned().fold(0.0, f32::max),
        {
            let mut t = times.clone();
            t.sort_by(|a, b| a.partial_cmp(b).unwrap());
            t[t.len() / 2]
        },
        times[..ten].iter().sum::<f32>() / ten as f32,
        times[count - ten..].iter().sum::<f32>() / ten as f32
    );

    // The transcript: every turn there, nothing of the machinery in what is
    // shown, and the conversation on disk reads back the same.
    let chat = app.app.chat.as_ref().ok_or("the conversation is gone")?;
    let turns = chat.turns.len();
    if turns < 2 * count {
        return Err(format!(
            "{count} questions should be {} turns; there are {turns}",
            2 * count
        ));
    }
    for (i, turn) in chat.turns.iter().enumerate() {
        let shown = crate::chat::copyable(turn);
        for mark in [
            "<told",
            "<thinking",
            "</thinking",
            "<tool_call",
            "<function=",
            "<parameter=",
            "<|channel",
            "<|tool_call",
            "<used ",
        ] {
            if shown.contains(mark) {
                return Err(format!("turn {i} shows the machinery ({mark}): {shown:?}"));
            }
        }
        if !turn.mine && shown.trim().is_empty() {
            return Err(format!("turn {i} is empty"));
        }
    }
    let path = chat
        .path
        .clone()
        .ok_or("the conversation was never saved")?;
    let saved = std::fs::read_to_string(&path).map_err(|e| format!("{e}"))?;
    let back = crate::chat::parse(&saved);
    if back.len() != turns {
        return Err(format!(
            "{turns} turns in memory, {} read back from disk",
            back.len()
        ));
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{WRONG}{} of {count} steps went wrong:\n        {}",
            wrong.len(),
            wrong.join("\n        ")
        ))
    }
}

/// One conversation, fifty questions long.
///
/// The way a note is actually made: a table started and added to and put
/// right, questions about it, another note, a list, things changed outside
/// the app in the middle, a change turned down, the conversation closed and
/// opened again, and questions that lean on what was said thirty questions
/// earlier. Every step checks the one thing it changed or asked, so the
/// first thing to go wrong is the thing reported - and at the end the
/// transcript itself is read: nothing of the machinery in it, every turn
/// there, and the notes on disk what fifty steps should have left.
///
/// The one scene long enough for the conversation to outgrow the room the
/// model is given, which is otherwise only known to work from a unit test.
fn a_long_session(app: &mut App) -> Result<(), String> {
    let trip = app.dir.join("trip");
    std::fs::create_dir_all(&trip).map_err(|e| format!("{e}"))?;
    looking_at(app, "trip", "zzqqtrip")?;
    fresh_chat(app)?;
    let (days_to, _) = day_count("2026-10-14").unwrap_or_default();
    let days_to_n: f64 = days_to.parse().unwrap_or(0.0);

    let mut steps: Vec<Step> = vec![
        step("make a note called budget.md with a table of Item and Cost: flights 420, hotel 610", Then::Accept, file_has("budget.md", &["flights", "420", "hotel", "610"])),
        step("add a row: food 150", Then::Accept, file_has("budget.md", &["food", "150", "flights"])),
        step("add a row: museum tickets 45", Then::Accept, file_has("budget.md", &["museum", "45", "food"])),
        step("oh, the hotel is 590, not 610", Then::Accept, Box::new(|_: &str, read: Read| {
            let t = read("budget.md");
            if t.contains("590") && !t.contains("610") && t.contains("420") { Ok(()) } else { Err(format!("{WRONG}the correction did not take: {t:?}")) }
        })),
        step("what is the total cost?", Then::Nothing, number(1205.0, 0.0)),
        step("what share of the total is flights, to one decimal place?", Then::Nothing, number(34.9, 0.1)),
        step("make a note called packing.md with a list: passport, charger, boots", Then::Accept, file_has("packing.md", &["passport", "charger", "boots"])),
        step("add sunscreen to the packing list", Then::Accept, file_has("packing.md", &["sunscreen", "passport"])),
        step("and a hat", Then::Accept, file_has("packing.md", &["hat", "sunscreen"])),
        step("take the boots off the packing list", Then::Accept, file_lacks("packing.md", "boots")),
        step("how many items are on the packing list now?", Then::Nothing, number(4.0, 0.0)),
        step("make a note called days.md with three lines: Day 1: fly. Day 2: museum. Day 3: hike.", Then::Accept, file_has("days.md", &["day 1", "fly", "day 2", "museum", "day 3", "hike"])),
        step("swap what happens on day 2 and day 3", Then::Accept, Box::new(|_: &str, read: Read| {
            let t = read("days.md").to_lowercase();
            let (d2, d3) = (t.find("day 2").unwrap_or(0), t.find("day 3").unwrap_or(0));
            let after2 = &t[d2..d3.max(d2)];
            if after2.contains("hike") && t[d3..].contains("museum") { Ok(()) } else { Err(format!("{WRONG}not swapped: {t:?}")) }
        })),
        step("what day of the week is 14 October 2026?", Then::Nothing, says(&["wednesday"])),
        step("how many days from today until then?", Then::Nothing, number(days_to_n, 0.0)),
        step("the trip is three days starting then - what is the last day?", Then::Nothing, says(&["16"])),
        step("add a line at the top of days.md: Dates: 14 to 16 October 2026", Then::Accept, file_has("days.md", &["14", "16 october"])),
        step("which notes mention the museum? name them.", Then::Nothing, says(&["budget.md", "days.md"])),
        step("which notes are in the trip project? list their names.", Then::Nothing, says(&["budget.md", "packing.md", "days.md"])),
        step("what does the hotel cost now?", Then::Nothing, number(600.0, 0.0)),
        step("and the total now?", Then::Nothing, number(1215.0, 0.0)),
        step("add a row: souvenirs 60", Then::Accept, file_has("budget.md", &["souvenirs", "60", "hotel"])),
        step("total?", Then::Nothing, number(1275.0, 0.0)),
        step("which item costs the most?", Then::Nothing, says(&["hotel"])),
        step("and the least?", Then::Nothing, says(&["museum"])),
        step("over three days, how much is the total per day?", Then::Nothing, number(425.0, 0.0)),
        step("change flights to 999", Then::Reject, Box::new(|_: &str, read: Read| {
            let t = read("budget.md");
            if t.contains("420") && !t.contains("999") { Ok(()) } else { Err(format!("a change that was turned down was made: {t:?}")) }
        })),
        step("no, leave flights as they are. what is the total again?", Then::Nothing, number(1275.0, 0.0)),
        step("make a note called ideas.md saying: book the museum early", Then::Accept, file_has("ideas.md", &["museum early"])),
        step("add to ideas.md: buy a hat before the trip", Then::Accept, file_has("ideas.md", &["hat", "museum early"])),
        step("how many notes are in the trip project now?", Then::Nothing, number(5.0, 0.0)),
        step("what happens on day 3?", Then::Nothing, says(&["museum"])),
        step("how many days are planned now?", Then::Nothing, number(4.0, 0.0)),
        step("the trip is four days now - update the dates line to 14 to 17 October 2026", Then::Accept, file_has("days.md", &["17 october"])),
        step("what is the cost per day over four days, to two decimal places?", Then::Nothing, number(318.75, 0.01)),
        step("delete ideas.md", Then::Accept, Box::new(|_: &str, read: Read| {
            if read("ideas.md").is_empty() { Ok(()) } else { Err("ideas.md is still there after the delete was taken".into()) }
        })),
        step("is ideas.md still in the project? yes or no.", Then::Nothing, says(&["no"])),
        step("what do the two cheapest items add up to?", Then::Nothing, number(105.0, 0.0)),
        step("in budget.md, rename 'museum tickets' to just 'museum'", Then::AcceptOr("nothing changed. read budget.md, then rename that row"), Box::new(|_: &str, read: Read| {
            let t = read("budget.md").to_lowercase();
            if t.contains("museum") && !t.contains("museum tickets") { Ok(()) } else { Err(format!("{WRONG}not renamed: {t:?}")) }
        })),
        step("what did the hotel cost before it was changed to 600?", Then::Nothing, number(590.0, 0.0)),
        step("how many rows does the budget table have, not counting the header?", Then::Nothing, Box::new(|said: &str, read: Read| {
            // Counted from the file: a model that added a Total row of its
            // own accord, and had it accepted, is right to count it.
            let rows = read("budget.md").lines().filter(|l| l.trim_start().starts_with('|')).count().saturating_sub(2);
            if number_near(said, rows as f64, 0.0) { Ok(()) } else { Err(format!("{WRONG}{rows}, it said: {said:?}")) }
        }) as Check),
        step("add a row: train 80", Then::Accept, file_has("budget.md", &["train", "80"])),
        step("total?", Then::Nothing, number(1355.0, 0.0)),
        step("what fraction of the total is the hotel, as a percentage to one decimal?", Then::Nothing, number(44.3, 0.1)),
        step("write a note called summary.md with the total, the cost per day over four days, and the dates", Then::Accept, file_has("summary.md", &["1355", "338.75", "october"])),
        step("what changed in the budget?", Then::Nothing, says(&["flights", "400"])),
        step("and the total now?", Then::Nothing, number(1335.0, 0.0)),
        step("update the total in summary.md to match", Then::Accept, file_has("summary.md", &["1335"])),
        step("list every note in the trip project with how many lines each has", Then::Nothing, says(&["budget.md", "packing.md", "days.md", "summary.md"])),
        step("thanks. in one sentence, what did we do today?", Then::Nothing, ok()),
    ];
    assert_eq!(steps.len(), 50, "fifty steps, not {}", steps.len());
    // Things that happen outside the app, before certain steps.
    steps[19].before = Some(outside("budget.md", "590", "600"));
    steps[32].before = Some(appended("days.md", "Day 4: rest."));
    steps[45].before = Some(outside("budget.md", "420", "400"));

    long_session(
        app,
        Long {
            folder: trip,
            steps,
            reopen_at: &[39],
        },
    )
}

/// Forty questions about a garden: three notes to begin with, another
/// project to read from and be refused a change to, tasks ticked off and
/// taken away, a merge, notes made and removed from outside, letters from
/// another alphabet, the solstice, and the conversation reopened twice.
///
/// The trip session makes its notes as it goes; this one starts with notes
/// that are there, so the front of the conversation is full from the first
/// question and every edit is against numbers the model was shown at the
/// top - then against numbers it was shown since, once the files move.
fn a_long_session_in_the_garden(app: &mut App) -> Result<(), String> {
    let garden = app.dir.join("garden");
    let kitchen = app.dir.join("kitchen");
    std::fs::create_dir_all(&garden).map_err(|e| format!("{e}"))?;
    std::fs::create_dir_all(&kitchen).map_err(|e| format!("{e}"))?;
    let beds = "# Beds\n\n\
        ## Bed 1\nTomatoes, six plants.\nWatered on Mondays.\n\n\
        ## Bed 2\nCourgettes.\nNeeds mulch.\n\n\
        ## Bed 3\nBeans, climbing.\n\n\
        ## Bed 4\nPotatoes, early.\n\n\
        ## Bed 5\nTomatoes, cherry, four plants.\n\n\
        ## Bed 6\nHerbs: basil, thyme, sage.\n\n\
        ## Bed 7\nEmpty this year.\n\n\
        ## Bed 8\nPelargoniums along the edge.\n";
    let harvest = "# Harvest\n\nWeighed at the shed.\n\n\
        | Crop | Kg |\n| --- | --- |\n| tomatoes | 12.5 |\n| courgettes | 8 |\n\
        | beans | 3.25 |\n| potatoes | 20 |\n";
    let tasks = "# Tasks\n\n- [ ] water the beds\n- [ ] buy mulch\n- [x] sow beans\n- [ ] stake the tomatoes\n";
    let recipes = "# Recipes\n\nThings to cook from the garden.\n\n\
        ## Ratatouille\nNeeds 4 tomatoes and 2 courgettes.\nSimmer for 40 minutes.\n";
    std::fs::write(garden.join("beds.md"), beds).map_err(|e| format!("{e}"))?;
    std::fs::write(garden.join("harvest.md"), harvest).map_err(|e| format!("{e}"))?;
    std::fs::write(garden.join("tasks.md"), tasks).map_err(|e| format!("{e}"))?;
    std::fs::write(kitchen.join("recipes.md"), recipes).map_err(|e| format!("{e}"))?;
    looking_at(app, "garden", "beds")?;
    fresh_chat(app)?;
    let (to_solstice, _) = day_count("2026-12-21").unwrap_or_default();
    let to_solstice: f64 = to_solstice.parse().unwrap_or(0.0);

    let mut steps: Vec<Step> = vec![
        // Reading what is at the front.
        step("which beds have tomatoes in them?", Then::Nothing, says(&["bed 1", "bed 5"])),
        step("how many tomato plants is that altogether?", Then::Nothing, number(10.0, 0.0)),
        step("what did we harvest the most of?", Then::Nothing, says(&["potatoes"])),
        step("how many kilos did we harvest in total?", Then::Nothing, number(43.75, 0.01)),
        step("how many tasks are still open?", Then::Nothing, number(3.0, 0.0)),
        // Edits against the numbers at the front.
        step("mark buy mulch as done", Then::Accept, file_has("tasks.md", &["[x] buy mulch", "[ ] water"])),
        step("add a task: net the brassicas", Then::Accept, file_has("tasks.md", &["net the brassicas", "stake"])),
        step("take the sow beans task off the list, it's done and dusted", Then::Accept, both(file_lacks("tasks.md", "sow beans"), file_has("tasks.md", &["net the brassicas", "water the beds"]))),
        step("how many are open now?", Then::Nothing, number(3.0, 0.0)),
        step("add a row to the harvest: żurawina 1.5", Then::Accept, file_has("harvest.md", &["żurawina", "1.5", "potatoes"])),
        step("and the total now?", Then::Nothing, number(45.25, 0.01)),
        step("bed 7 is not empty any more - it has leeks", Then::Accept, both(file_has("beds.md", &["leeks", "pelargoniums"]), file_lacks("beds.md", "empty this year"))),
        // Another project: read, and refused a change.
        step("what does the recipes note in the kitchen project say ratatouille needs?", Then::Nothing, says(&["4", "tomatoes"])),
        step("change that recipe to need 3 tomatoes instead", Then::Nothing, Box::new(|_: &str, read: Read| {
            let t = read("../kitchen/recipes.md");
            if t.contains("4 tomatoes") && !t.contains("3 tomatoes") { Ok(()) } else { Err(format!("a note in another project was changed: {t:?}")) }
        })),
        step("fine. do we have enough tomatoes in the harvest for ten batches of ratatouille, if a tomato weighs 100 grams?", Then::Nothing, says(&["yes"])),
        // The vault's tools.
        step("which notes in the whole vault mention tomatoes? just the names.", Then::Nothing, says(&["beds.md", "harvest.md", "recipes.md"])),
        step("which notes have 'bed' in their name?", Then::Nothing, says(&["beds.md"])),
        step("list every note in the garden project", Then::Nothing, says(&["beds.md", "harvest.md", "tasks.md"])),
        // Things done from outside.
        step("which notes are in the garden project now?", Then::Nothing, says(&["seeds.md", "beds.md"])),
        step("what does seeds.md say?", Then::Nothing, says(&["carrots", "radish"])),
        step("is seeds.md still there? yes or no.", Then::Nothing, says(&["no"])),
        step("what is the courgette harvest now?", Then::Nothing, number(9.0, 0.0)),
        step("and the total?", Then::Nothing, number(46.25, 0.01)),
        // Dates.
        step("how many days until the winter solstice, 21 December 2026?", Then::Nothing, number(to_solstice, 0.0)),
        step("what day of the week is that?", Then::Nothing, says(&["monday"])),
        step("add a task: order seed catalogues before the solstice", Then::Accept, file_has("tasks.md", &["seed catalogues", "net the brassicas"])),
        // A merge, then a note made again.
        step("merge tasks.md into beds.md, tasks at the end", Then::Accept, both(file_has("beds.md", &["net the brassicas", "pelargoniums"]), file_gone("tasks.md"))),
        step("how many notes are in the garden project now?", Then::Nothing, number(2.0, 0.0)),
        step("make a note called seeds.md with a list: carrots, radish, parsnips", Then::Accept, file_has("seeds.md", &["carrots", "radish", "parsnips"])),
        step("add beetroot to it", Then::Accept, file_has("seeds.md", &["beetroot", "carrots"])),
        step("how many kinds of seed is that?", Then::Nothing, number(4.0, 0.0)),
        // Back to the beds, now longer.
        step("which bed has the herbs?", Then::Nothing, says(&["bed 6"])),
        step("add mint to the herbs in that bed", Then::Accept, file_has("beds.md", &["mint", "basil", "thyme"])),
        step("what is in bed 2?", Then::Nothing, says(&["courgettes"])),
        step("bed 2 got its mulch - take that line out", Then::Accept, both(file_lacks("beds.md", "needs mulch"), file_has("beds.md", &["courgettes", "bed 3"]))),
        // A change turned down, and the same thing asked for another way.
        step("delete harvest.md", Then::Reject, file_has("harvest.md", &["potatoes", "żurawina"])),
        step("no, keep it. how many crops are in it?", Then::Nothing, number(5.0, 0.0)),
        step("what share of the harvest is potatoes, to one decimal place?", Then::Nothing, number(43.2, 0.1)),
        step("list every note in the garden project with how many lines each has", Then::Nothing, says(&["beds.md", "harvest.md", "seeds.md"])),
        step("in one sentence, what did we do today?", Then::Nothing, ok()),
    ];
    assert_eq!(steps.len(), 40, "forty steps, not {}", steps.len());
    steps[18].before = Some(made_outside("seeds.md", "# Seeds\n\n- carrots\n- radish\n"));
    steps[20].before = Some(removed_outside("seeds.md"));
    steps[21].before = Some(outside(
        "harvest.md",
        "| courgettes | 8 |",
        "| courgettes | 9 |",
    ));
    long_session(
        app,
        Long {
            folder: garden,
            steps,
            reopen_at: &[15, 31],
        },
    )
}

/// Forty questions of bookkeeping: a ledger with dates, clients and amounts,
/// added to and corrected and put in order, totals that have to be right
/// after every change, a client renamed in two notes at once, an invoice
/// written from the figures and then updated, and figures changed outside.
///
/// The numbers are the point. A total asked for after every change is a
/// total the model has to take from the file as it is now, not from what it
/// said three questions ago.
fn a_long_session_of_bookkeeping(app: &mut App) -> Result<(), String> {
    let books = app.dir.join("books");
    std::fs::create_dir_all(&books).map_err(|e| format!("{e}"))?;
    let ledger = "# Ledger\n\n| Date | Client | Item | Amount |\n| --- | --- | --- | --- |\n\
        | 2026-01-12 | Acme | logo | 400 |\n| 2026-02-03 | Bloom | website | 1200 |\n\
        | 2026-02-20 | Acme | cards | 150 |\n| 2026-03-14 | Cedar | poster | 300 |\n\
        | 2026-03-30 | Bloom | hosting | 60 |\n| 2026-04-08 | Acme | banner | 220 |\n";
    let clients = "# Clients\n\n- Acme - acme@example.com\n- Bloom - hello@bloom.example\n- Cedar - cedar@example.org\n";
    std::fs::write(books.join("ledger.md"), ledger).map_err(|e| format!("{e}"))?;
    std::fs::write(books.join("clients.md"), clients).map_err(|e| format!("{e}"))?;
    looking_at(app, "books", "ledger")?;
    fresh_chat(app)?;

    let mut steps: Vec<Step> = vec![
        // Sums over what is there.
        step("what do the amounts in the ledger add up to?", Then::Nothing, number(2330.0, 0.0)),
        step("how much of that is Acme?", Then::Nothing, number(770.0, 0.0)),
        step("which single entry was the biggest?", Then::Nothing, says(&["website"])),
        step("what is the average amount, to two decimal places?", Then::Nothing, number(388.33, 0.01)),
        step("how many entries are there in March?", Then::Nothing, number(2.0, 0.0)),
        step("how many days from the first entry to the last?", Then::Nothing, number(86.0, 0.0)),
        step("what day of the week was the poster entry?", Then::Nothing, says(&["saturday"])),
        // Changes, and the total after each.
        step("add an entry: 2026-04-20, Cedar, flyer, 90", Then::Accept, file_has("ledger.md", &["2026-04-20", "flyer", "90", "banner"])),
        step("total now?", Then::Nothing, number(2420.0, 0.0)),
        step("the poster was 350, not 300", Then::Accept, both(file_has("ledger.md", &["| 350 |"]), file_lacks("ledger.md", "| 300 |"))),
        step("total now?", Then::Nothing, number(2470.0, 0.0)),
        step("remove the hosting entry, that was a mistake", Then::Accept, both(file_lacks("ledger.md", "hosting"), file_has("ledger.md", &["website", "flyer"]))),
        step("and the total?", Then::Nothing, number(2410.0, 0.0)),
        step("how much is Cedar now?", Then::Nothing, number(440.0, 0.0)),
        step("which client brought in the most?", Then::Nothing, says(&["bloom"])),
        // The other note.
        step("add a client: Dune - dune@example.net", Then::Accept, file_has("clients.md", &["dune", "dune@example.net", "cedar"])),
        step("which notes mention Cedar? names only.", Then::Nothing, says(&["ledger.md", "clients.md"])),
        step("rename Acme to Acme Ltd everywhere - in the ledger and in the clients note", Then::Accept, both(file_has("ledger.md", &["acme ltd", "logo", "cards", "banner"]), file_has("clients.md", &["acme ltd", "acme@example.com"]))),
        step("how many entries are Acme Ltd?", Then::Nothing, number(3.0, 0.0)),
        // Turned down, then kept.
        step("delete the cards entry", Then::Reject, file_has("ledger.md", &["cards", "150"])),
        step("no, keep it. what is the total again?", Then::Nothing, number(2410.0, 0.0)),
        // A note written from the figures, and kept right.
        step("write a note called invoice.md for Bloom: their entries and their total", Then::Accept, file_has("invoice.md", &["bloom", "website", "1200"])),
        step("what does invoice.md say the total is?", Then::Nothing, number(1200.0, 0.0)),
        step("add a ledger entry: 2026-05-02, Bloom, newsletter, 180", Then::Accept, file_has("ledger.md", &["newsletter", "180"])),
        step("update invoice.md to match", Then::Accept, file_has("invoice.md", &["newsletter", "1380"])),
        step("what is the ledger total now?", Then::Nothing, number(2590.0, 0.0)),
        // Changed from outside.
        step("what changed in the ledger?", Then::Nothing, says(&["banner"])),
        step("total now?", Then::Nothing, number(2620.0, 0.0)),
        step("how many clients are in the clients note now?", Then::Nothing, number(5.0, 0.0)),
        step("who was added?", Then::Nothing, says(&["elm"])),
        // Order and sections.
        step("what is 15% of the total, to the nearest whole number?", Then::Nothing, number(393.0, 0.0)),
        step("sort the ledger by amount, largest first", Then::Accept, Box::new(|_: &str, read: Read| {
            let t = read("ledger.md");
            let rows: Vec<&str> = t.lines().filter(|l| l.starts_with("| 2026")).collect();
            let amounts: Vec<f64> = rows.iter().filter_map(|r| r.trim_end_matches('|').rsplit('|').next()?.trim().parse().ok()).collect();
            if amounts.len() != 7 { return Err(format!("{WRONG}{} rows after sorting, not 7: {t:?}", amounts.len())); }
            if amounts.windows(2).all(|w| w[0] >= w[1]) { Ok(()) } else { Err(format!("{WRONG}not in order: {amounts:?}")) }
        })),
        step("which entry is first now?", Then::Nothing, says(&["website"])),
        step("add a section at the end of clients.md called Notes, saying: invoices go out on the 1st", Then::Accept, file_has("clients.md", &["## notes", "invoices go out on the 1st", "elm"])),
        step("move the Notes section to just under the title, above the clients", Then::Accept, Box::new(|_: &str, read: Read| {
            let t = read("clients.md").to_lowercase();
            let (n, a) = (t.find("## notes"), t.find("- acme"));
            match (n, a) {
                (Some(n), Some(a)) if n < a && t.contains("invoices go out") && t.contains("elm") => Ok(()),
                _ => Err(format!("{WRONG}the section did not move: {t:?}")),
            }
        })),
        step("how many entries does the ledger have?", Then::Nothing, number(7.0, 0.0)),
        step("and how much is Bloom in total?", Then::Nothing, number(1380.0, 0.0)),
        step("does invoice.md still match? yes or no.", Then::Nothing, says(&["yes"])),
        step("list every note in the books project with how many lines each has", Then::Nothing, says(&["ledger.md", "clients.md", "invoice.md"])),
        step("in one sentence, what did we do today?", Then::Nothing, ok()),
    ];
    assert_eq!(steps.len(), 40, "forty steps, not {}", steps.len());
    steps[26].before = Some(outside("ledger.md", "| banner | 220 |", "| banner | 250 |"));
    steps[28].before = Some(appended("clients.md", "- Elm - elm@example.com"));
    long_session(
        app,
        Long {
            folder: books,
            steps,
            reopen_at: &[21],
        },
    )
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
    (
        "a long note changed in two places",
        a_long_note_changed_in_two_places,
    ),
    (
        "another project can be read but not changed",
        another_project_can_be_read_but_not_changed,
    ),
    ("a table summed and shared", a_table_summed_and_shared),
    (
        "a thread that leans on earlier answers",
        a_thread_that_leans_on_earlier_answers,
    ),
    (
        "a list built, corrected and reckoned up",
        a_list_built_corrected_and_reckoned_up,
    ),
    (
        "figures from another project brought here",
        figures_from_another_project_brought_here,
    ),
    ("two notes folded into one", two_notes_folded_into_one),
    (
        "a note deleted here and not there",
        a_note_deleted_here_and_not_there,
    ),
    (
        "turned down, then asked for differently",
        turned_down_then_asked_for_differently,
    ),
    (
        "an answer stopped and the next still comes",
        an_answer_stopped_and_the_next_still_comes,
    ),
    (
        "letters in other alphabets survive a copy",
        letters_in_other_alphabets_survive_a_copy,
    ),
    (
        "a section added then reordered",
        a_section_added_then_reordered,
    ),
    (
        "three notes read for one answer",
        three_notes_read_for_one_answer,
    ),
    (
        "a conversation reopened carries on",
        a_conversation_reopened_carries_on,
    ),
    (
        "a change undone in the editor is seen",
        a_change_undone_in_the_editor_is_seen,
    ),
    (
        "an analysis across three notes",
        an_analysis_across_three_notes,
    ),
    (
        "a value changed in every note it reaches",
        a_value_changed_in_every_note_it_reaches,
    ),
    (
        "a name changed across the project",
        a_name_changed_across_the_project,
    ),
    (
        "a summary that drifted is put right",
        a_summary_that_drifted_is_put_right,
    ),
    (
        "a note found by part of its name",
        a_note_found_by_part_of_its_name,
    ),
    ("where a word is said", where_a_word_is_said),
    ("a long session", a_long_session),
    ("a long session in the garden", a_long_session_in_the_garden),
    (
        "a long session of bookkeeping",
        a_long_session_of_bookkeeping,
    ),
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
