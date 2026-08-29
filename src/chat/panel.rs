//! The conversation on screen: the turns, the changes on offer, the box to
//! type in, and the list of conversations to come back to.

use std::path::PathBuf;

use pixui::{font, Align, Key, Rect, Ui};

use super::change::{proposals, settle, Change, Folder, What};
use super::{
    ago, called, complete, completions, copyable, lookups, one_line, round, Chat, Drawn, Filed,
    Outcome, ROWS,
};
use crate::markdown;
use crate::render;

/// What the assistant is doing right now, in a line.
///
/// It said "reading the notes" for the whole of anything that was not yet
/// writing, and most of that was not reading the notes: it was the weights
/// going in, or the tail of a question being read while the notes sat in the
/// cache from last time, or the wait for the first token, or a two-line tool
/// response going in after a lookup. Each of those is something else, and
/// somebody watching a panel for twelve seconds deserves the right one.
pub fn doing(p: &crate::llm::Progress) -> String {
    if p.loading {
        return "LOADING THE MODEL...".to_string();
    }
    if p.looking {
        return format!("LOOKING SOMETHING UP... ({} SO FAR)", p.steps);
    }
    if p.deliberating {
        return format!("THINKING... {} TOKENS", p.written);
    }
    if p.written > 0 {
        return if p.thought > 0 {
            format!(
                "WRITING... {} TOKENS, {:.0}/S - THOUGHT FOR {}",
                p.written,
                p.rate(),
                p.thought
            )
        } else {
            format!("WRITING... {} TOKENS, {:.0}/S", p.written, p.rate())
        };
    }
    if p.prompt == 0 {
        return "ASKING...".to_string();
    }
    if p.read >= p.prompt {
        return "ABOUT TO ANSWER...".to_string();
    }
    // A share rather than a count: the number of tokens in a vault is not
    // something anybody has a feel for, and how near the end it is, is the
    // only part being watched. Of what is actually being read, which is the
    // notes the first time and the new part of the question after that.
    let fresh = p.fresh.max(1);
    let done = p.read.saturating_sub(p.prompt.saturating_sub(p.fresh));
    let share = (100 * done / fresh).min(99);
    if p.fresh * 2 < p.prompt {
        format!("READING WHAT'S NEW... {share}%")
    } else {
        format!("READING THE NOTES... {share}%")
    }
}

/// Which conversation to open, chosen from the ones there are.
pub struct Picker {
    pub selected: usize,
    fresh: bool,
    /// Which row has been asked about, while it is being asked about. Throwing
    /// a conversation away is not undoable and not worth a single click.
    confirming: Option<usize>,
}

/// What the picker decided.
pub enum Picked {
    None,
    /// Start a conversation with nothing in it.
    Fresh,
    /// Carry on with this one.
    Open(PathBuf),
    /// Throw this one away, for good.
    Delete(PathBuf),
    Close,
}

impl Picker {
    pub fn new() -> Self {
        Self {
            selected: 0,
            fresh: true,
            confirming: None,
        }
    }

    /// Draw it, take its keys, and say what was chosen.
    ///
    /// The new-conversation row is one of the rows rather than a button off to
    /// the side, so the same two keys reach everything and starting a new one
    /// costs the same gesture as continuing an old one.
    pub fn show(&mut self, ui: &mut Ui, project: &str, chats: &[Filed]) -> Picked {
        let th = *ui.theme;
        let line_h = font::line_h();
        let screen = ui.canvas.bounds();
        ui.canvas
            .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

        let rows = (chats.len() + 1).min(ROWS);
        let rect = screen.centered(440, rows as i32 * line_h + 4 * line_h + 26);
        let inner = ui.panel(rect, &format!("CHATS IN {}", called(project)));
        ui.capture_keyboard();

        let count = chats.len() + 1;
        let mut picked = Picked::None;
        if self.fresh {
            self.fresh = false;
        } else {
            let ctrl = ui.input.mods.ctrl;
            for key in ui.input.keys.clone() {
                // While a row is asking, the keys belong to the question: Enter
                // answers it and Escape takes it back, and neither reaches the
                // list underneath.
                if let Some(row) = self.confirming {
                    match key {
                        Key::Enter | Key::Char('y') => {
                            picked = self.remove(chats, row);
                            self.confirming = None;
                        }
                        Key::Escape | Key::Char('n') => self.confirming = None,
                        _ => {}
                    }
                    continue;
                }
                match key {
                    Key::Escape => picked = Picked::Close,
                    Key::Enter => picked = self.choose(chats),
                    Key::Down | Key::Char('j') => self.step(1, count),
                    Key::Up | Key::Char('k') => self.step(-1, count),
                    Key::Char('n') if ctrl => self.step(1, count),
                    Key::Char('p') if ctrl => self.step(-1, count),
                    // The same key the file dialog throws things away with.
                    Key::Char('d') | Key::Delete | Key::Backspace if self.selected > 0 => {
                        self.confirming = Some(self.selected);
                    }
                    _ => {}
                }
            }
        }
        if !matches!(picked, Picked::None) {
            return picked;
        }

        // Room enough at the foot for a control, because the question about
        // throwing a conversation away is asked down there rather than in the
        // row: a row is nine pixels tall and the question needs a sentence and
        // two answers.
        let (list, foot) = inner.split_bottom(line_h + 8);
        ui.canvas.box_chamfer(list, th.well, th.well_border, 1);
        let mut clicked = None;
        let mut asking = None;
        let mut answered = None;
        let mut kept = false;
        ui.clipped(list.inset(1), |ui| {
            for i in 0..count.min(ROWS) {
                let y = list.y + i as i32 * line_h;
                let row = Rect::new(list.x, y, list.w, line_h);
                // The bin asks first. Two things want the same pixels, and the
                // pointer goes to whoever asks for it first: with the row
                // asking first, a press on the bin was taken by the row and
                // the answer to "delete this" was "open it".
                let bin = Rect::new(row.right() - 11, y + 1, 9, 7);
                let bin_hit = ui.id(&format!("bin{i}"));
                let hit = ui.interact(bin_hit, Rect::new(bin.x - 2, y, 13, line_h));
                if hit.clicked {
                    asking = Some(i);
                }
                let id = ui.id(&format!("chat{i}"));
                let resp = ui.interact(id, row);
                if i == self.selected {
                    ui.canvas.fill_rect(row, th.accent.lo);
                } else if resp.hovered {
                    ui.canvas.fill_rect(row, th.well.shade(0.12));
                }
                if resp.clicked {
                    clicked = Some(i);
                }
                let picked_row = i == self.selected;
                let ink = if picked_row {
                    th.accent.ink
                } else {
                    th.ink_light
                };
                let dim = if picked_row {
                    th.accent.ink
                } else {
                    th.ink_soft
                };
                let at = Rect::new(row.x + 4, y, row.w - 8, line_h);
                if i == 0 {
                    font::draw_text_styled(ui.canvas, at.x, y, "+ A NEW CHAT", ink, true);
                    continue;
                }
                let chat = &chats[i - 1];
                // The one being asked about is marked where it sits, and the
                // question itself is asked at the foot. What is about to be
                // thrown away should stay visible while you decide.
                if self.confirming == Some(i) {
                    ui.canvas.fill_rect(row, th.danger.lo);
                    ui.draw_text_in(at, &chat.title, th.danger.hi, Align::Left);
                    continue;
                }
                // Room kept for the bin whether or not it is drawn, so the
                // columns do not shuffle sideways as the pointer moves.
                let text = Rect::new(at.x, at.y, at.w - 12, at.h);
                ui.draw_text_in(text, &chat.title, ink, Align::Left);
                let said = match chat.turns {
                    1 => "1 TURN".to_string(),
                    n => format!("{n} TURNS"),
                };
                ui.draw_text_in(
                    text,
                    &format!("{}  {}", said, ago(chat.when)),
                    dim,
                    Align::Right,
                );
                if resp.hovered || picked_row || hit.hovered {
                    let tint = if hit.hovered { th.danger.hi } else { dim };
                    pixui::icon::draw(ui.canvas, bin.x, bin.y, pixui::icon::BIN, tint);
                }
            }
        });

        let target = self
            .confirming
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| chats.get(i));
        match target {
            Some(chat) => {
                let line = Rect::new(foot.x, foot.y + 3, foot.w, line_h);
                ui.draw_text_in(
                    line,
                    &format!("DELETE \"{}\" FOR GOOD?", chat.title.to_uppercase()),
                    th.danger.face,
                    Align::Left,
                );
                let no = Rect::new(foot.right() - 40, foot.y + 1, 40, 13);
                let yes = Rect::new(no.x - 48, foot.y + 1, 46, 13);
                if ui.button_at(yes, "DELETE", pixui::Tone::Danger).clicked {
                    answered = Some(self.confirming.unwrap_or(0));
                }
                if ui.button_at(no, "KEEP", pixui::Tone::Neutral).clicked {
                    kept = true;
                }
            }
            None => {
                let line = Rect::new(foot.x, foot.y + 3, foot.w, line_h);
                let said = match chats.len() {
                    0 => "NOTHING YET".to_string(),
                    1 => "1 CHAT".to_string(),
                    n => format!("{n} CHATS"),
                };
                ui.draw_text_in(line, &said, th.ink_soft, Align::Left);
                ui.draw_text_in(
                    line,
                    "ENTER OPENS, D DELETES, ESC LEAVES",
                    th.ink_soft,
                    Align::Right,
                );
            }
        }

        if let Some(i) = asking {
            self.selected = i;
            self.confirming = Some(i);
            return Picked::None;
        }
        if kept {
            self.confirming = None;
            return Picked::None;
        }
        if let Some(i) = answered {
            self.confirming = None;
            return self.remove(chats, i);
        }
        match clicked {
            // A click on the row that is asking is not a choice about the row.
            Some(i) if self.confirming.is_none() => {
                self.selected = i;
                self.choose(chats)
            }
            _ => Picked::None,
        }
    }

    /// Throw away the conversation on row `i`.
    fn remove(&self, chats: &[Filed], i: usize) -> Picked {
        match i.checked_sub(1).and_then(|i| chats.get(i)) {
            Some(chat) => Picked::Delete(chat.path.clone()),
            None => Picked::None,
        }
    }

    fn choose(&self, chats: &[Filed]) -> Picked {
        match self.selected.checked_sub(1) {
            None => Picked::Fresh,
            Some(i) => match chats.get(i) {
                Some(chat) => Picked::Open(chat.path.clone()),
                None => Picked::Fresh,
            },
        }
    }

    fn step(&mut self, by: i32, count: usize) {
        if count == 0 {
            return;
        }
        let n = count as i32;
        self.selected = ((self.selected as i32 + by).rem_euclid(n)) as usize;
    }
}

impl Default for Picker {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------- the conversation

/// How many rows of a diff to draw before saying how many are left.
///
/// Ten, so that a box and its buttons fit a short pane. The buttons are at
/// the head of the box, and a conversation that has just been answered is
/// scrolled to its end: a box taller than the pane showed the tail of its
/// diff and no way to answer it, and the panel said a change was waiting.
const WHOLE: usize = 10;

/// How much room the field and its hint take at the foot of the panel.
const FOOT: i32 = 34;

impl Chat {
    /// Draw the conversation and take its keys.
    ///
    /// Call this with the rest of the frame wrapped in [`Ui::input_blocked`],
    /// the way the dialogs are: a conversation has the keyboard while it is up.
    pub fn show(&mut self, ui: &mut Ui, folder: &Folder) -> Outcome {
        let th = *ui.theme;
        let line_h = font::line_h();
        let screen = ui.canvas.bounds();
        ui.canvas
            .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

        let rect = screen.centered((screen.w - 60).min(620), screen.h - 40);
        // Named after the conversation rather than the note, because the name
        // is the thing `/rename` changes and a rename you cannot see happen is
        // a rename you do twice. The readout beside it is what the last
        // question actually came to, which is the only honest way to say how
        // much of the window a conversation is using.
        let inner = ui.panel_badged(rect, &self.title().to_uppercase(), &self.weight(folder));
        ui.capture_keyboard();

        let mut outcome = Outcome::None;
        if ui.input.key_pressed(Key::Escape) {
            return Outcome::Close;
        }
        // A change that has been offered is a question back, and it is answered
        // before anything else is asked. Otherwise the next answer is written
        // against a note that may or may not be about to change, and the diff
        // sitting above it quietly stops meaning what it says.
        let held = self.pending(folder);
        // Enter sends. A conversation is one question at a time, so the key
        // that ends a sentence is the key that asks it; a newline inside one
        // question is a thing to want later and not today.
        if !held && ui.input.key_pressed(Key::Enter) && !self.draft.trim().is_empty() {
            let typed = self.draft.clone();
            if self.command(&typed) {
                outcome = if std::mem::take(&mut self.flip_web) {
                    Outcome::Web
                } else {
                    // A rename is worth writing down straight away: it is the
                    // kind of thing you do and then close the panel.
                    Outcome::Save
                };
            } else if !self.waiting {
                self.notice = None;
                self.commit();
                outcome = Outcome::Ask;
            }
        }

        // What a half-typed command could still become, offered above the
        // field: near what is being typed, and out of the transcript's way.
        let hints = if held {
            Vec::new()
        } else {
            completions(self.draft.trim_end())
        };
        if !held && ui.input.key_pressed(Key::Tab) {
            if let Some(finished) = complete(self.draft.trim_end()) {
                self.draft = finished;
                self.retype = true;
            }
        }
        let strip = if hints.is_empty() {
            0
        } else {
            hints.len() as i32 * line_h + 4
        };
        let (body, foot) = inner.split_bottom(FOOT + strip);
        let (menu, foot) = foot.split_top(strip);
        if strip > 0 {
            ui.canvas.box_chamfer(menu, th.well, th.well_border, 1);
            for (i, hit) in hints.iter().enumerate() {
                let row = Rect::new(
                    menu.x + 3,
                    menu.y + 2 + i as i32 * line_h,
                    menu.w - 6,
                    line_h,
                );
                let head = format!("/{}{}", hit.name, hit.takes);
                font::draw_text_styled(ui.canvas, row.x, row.y, &head, th.accent.hi, true);
                ui.draw_text_in(row, hit.what, th.ink_soft, Align::Right);
            }
        }

        // ---- the transcript ------------------------------------------------
        // A strip above it for the things that are about the whole
        // conversation rather than about one turn in it. Outside the scroll
        // area on purpose: a control that scrolls away is one you go looking
        // for.
        let (bar, body) = body.split_top(line_h + 5);
        // Right edge in line with the buttons on the turns below it, which are
        // inside the scroll area and so inset by its gutter.
        let whole = Rect::new(
            bar.right() - Ui::SCROLL_GUTTER - 4 - 62,
            bar.y + 3,
            62,
            line_h,
        );
        let mut copied = if self.turns.is_empty() {
            false
        } else {
            ui.button_at(whole, "COPY ALL", pixui::Tone::Neutral)
                .clicked
        };
        if copied {
            pixui::clipboard::copy(&self.transcript());
        }
        let width = body.w - Ui::SCROLL_GUTTER - 8;
        let mut state = self.scroll;
        let mut answered = None;
        self.redraw_turns();
        ui.scroll_area_with(body, "chat-scroll", &mut state, |ui| {
            if self.turns.is_empty() {
                ui.space(4);
                ui.label_dim("  NOTHING ASKED YET.");
                ui.label_dim("  IT CAN SEE THIS NOTE AND A LINE ABOUT EVERY OTHER ONE.");
                return;
            }
            for (i, turn) in self.turns.iter().enumerate() {
                ui.space(3);
                let head = ui.alloc(line_h);
                let (who, tint) = if turn.mine {
                    ("YOU", th.accent.face)
                } else {
                    ("ASSISTANT", th.positive.face)
                };
                font::draw_text_styled(ui.canvas, head.x + 4, head.y, who, tint, true);
                // The button asks for its click before the rule is drawn under
                // it, because the first thing to ask for a place on the screen
                // is the thing that gets it.
                let take = Rect::new(head.right() - line_h, head.y, line_h, line_h);
                if ui
                    .icon_button_at(
                        take,
                        &format!("copy-turn-{i}"),
                        pixui::icon::COPY,
                        pixui::Tone::Neutral,
                    )
                    .clicked
                {
                    pixui::clipboard::copy(&copyable(turn));
                    copied = true;
                }
                // A rule from the end of the name to the button, which is what
                // separates two turns without a box around each of them.
                let from = head.x + 6 + font::advance_width(who);
                ui.canvas
                    .hline(from, head.y + line_h / 2, take.x - 4 - from, th.well_border);
                // What it said, and separately what it offered to do. The
                // blocks are lifted out so the reply reads as a sentence
                // rather than as a sentence with machinery in the middle.
                let Drawn {
                    prose,
                    looked,
                    edits,
                    ..
                } = &self.drawn[i];
                // What it went and found, before what it made of it.
                for look in looked {
                    let row = ui.alloc(line_h);
                    let head = look.said();
                    font::draw_text_styled(ui.canvas, row.x + 4, row.y, &head, th.info.hi, true);
                    let line = ui.alloc(line_h);
                    let got = look
                        .result
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("(nothing)");
                    ui.draw_text_in(
                        Rect::new(line.x + 8, line.y, line.w - 8, line.h),
                        &one_line(got, ((line.w - 16) / font::advance()).max(8) as usize),
                        th.ink_soft,
                        Align::Left,
                    );
                    ui.space(2);
                }
                let lines: Vec<String> = prose.lines().map(str::to_string).collect();
                let blocks = markdown::parse_located(&lines);
                render::draw_document(
                    ui,
                    &blocks,
                    render::Request {
                        width,
                        numbered: false,
                        search: None,
                        reveal: None,
                    },
                );
                for (j, change) in edits.iter().enumerate() {
                    if let Some(taken) = self.offer(ui, width, folder, change) {
                        answered = Some((i, j, change.clone(), taken));
                    }
                }
            }
            if self.waiting {
                ui.space(3);
                let row = ui.alloc(line_h);
                let said = doing(&self.progress);
                // Something to watch while it thinks. A number going up says
                // the machine is busy; a wave says it is alive, which is a
                // different thing to be told over twenty seconds.
                let mut at_x = row.x + 4;
                if self.progress.deliberating {
                    let wave = Rect::new(row.x + 4, row.y, 5 * 4, row.h);
                    ui.wave(wave, th.accent.hi);
                    at_x += wave.w + 6;
                }
                font::draw_text_styled(ui.canvas, at_x, row.y, &said, th.info.hi, true);
                // The answer as it arrives, in the ink it will keep. A counter
                // going up says the machine is busy; the words say what it is
                // saying, and twenty seconds of the first one is twenty
                // seconds of having to take it on trust.
                if !self.partial.trim().is_empty() {
                    let lines: Vec<String> =
                        self.partial.lines().map(str::to_string).collect::<Vec<_>>();
                    let blocks = markdown::parse_located(&lines);
                    render::draw_document(
                        ui,
                        &blocks,
                        render::Request {
                            width,
                            numbered: false,
                            search: None,
                            reveal: None,
                        },
                    );
                }
            }
            if let Some(why) = &self.failed {
                ui.space(3);
                let row = ui.alloc(line_h);
                ui.draw_text_in(row, &why.to_uppercase(), th.danger.face, Align::Left);
            }
            if let Some(said) = &self.notice {
                ui.space(3);
                // A line at a time, because `/help` is a listing and a listing
                // squeezed onto one row is a listing nobody reads.
                for line in said.lines() {
                    let row = ui.alloc(line_h);
                    ui.draw_text_in(row, &line.to_uppercase(), th.info.hi, Align::Left);
                }
            }
            // A little air under the last turn, so the newest thing said is not
            // flush against the field you say the next one into.
            ui.space(6);
        });
        // Pinned to the foot while a conversation is being had, and let go of
        // the moment the wheel is touched: reading back through it is the other
        // half of what the scrollbar is for.
        if state.target != self.scroll.target && (state.target - state.max_offset()).abs() > 1.0 {
            self.follow = false;
        }
        if self.follow {
            state.target = state.max_offset();
            state.shown = state.target;
        }
        self.scroll = state;
        if copied {
            self.notice = Some("copied to the clipboard".into());
        }

        // ---- what to say next ----------------------------------------------
        let field = Rect::new(foot.x, foot.y + 2, foot.w, 15);
        // Set again below by the one branch that draws a field.
        let was_there = std::mem::take(&mut self.had_field);
        if self.waiting {
            // Nothing to type into while an answer is on its way, so the room
            // is spent on the way out of it instead.
            let button = Rect::new(field.right() - 52, field.y, 52, 15);
            let say = Rect::new(field.x + 4, field.y, field.w - 62, field.h);
            ui.canvas
                .box_chamfer(field, th.well.shade(-0.04), th.well_border, 1);
            ui.draw_text_in(say, "ANSWERING...", th.ink_soft, Align::Left);
            if ui.button_at(button, "STOP", pixui::Tone::Danger).clicked {
                outcome = Outcome::Stop;
            }
        } else if held {
            // Not a disabled field but no field at all: one that looks like it
            // takes typing and does not is worse than one that is plainly not
            // there, and the space is better spent saying why.
            ui.canvas
                .box_chamfer(field, th.well.shade(-0.04), th.well_border, 1);
            let say = Rect::new(field.x + 4, field.y, field.w - 8, field.h);
            ui.draw_text_in(
                say,
                "ACCEPT OR REJECT THE CHANGE ABOVE TO CARRY ON",
                th.info.hi,
                Align::Left,
            );
        } else {
            let hint = "ASK SOMETHING";
            let mut draft = std::mem::take(&mut self.draft);
            // `grab` also puts the caret after the text, which is what a field
            // whose contents were just completed for it needs.
            let take = self.grab || std::mem::take(&mut self.retype) || !was_there;
            ui.text_field_grab_at(field, "chat-field", &mut draft, hint, take);
            self.draft = draft;
            self.had_field = true;
        }
        self.grab = false;

        let legend = Rect::new(foot.x, field.bottom() + 2, foot.w, line_h);
        let said = if held {
            format!("IN {} - A CHANGE IS WAITING", called(&self.project))
        } else {
            format!(
                "IN {} - LOOKING AT {} - /HELP LISTS THE COMMANDS",
                called(&self.project),
                self.focus.to_uppercase()
            )
        };
        ui.draw_text_in(legend, &said, th.ink_soft, Align::Left);
        ui.draw_text_in(
            legend,
            "ESC LEAVES - IT IS SAVED",
            th.ink_soft,
            Align::Right,
        );
        match answered {
            Some((i, j, edit, taken)) => {
                let _ = j;
                self.turns[i].text = settle(&self.turns[i].text, &edit, taken);
                if taken {
                    Outcome::Apply(edit)
                } else {
                    Outcome::Save
                }
            }
            None => outcome,
        }
    }

    /// Whether a change has been offered and not yet answered.
    ///
    /// Only one that could still be made counts. A block whose lines have since
    /// gone is not something anybody can accept or reject, and letting one of
    /// those hold the field would be a conversation nobody can get out of.
    pub fn pending(&self, folder: &Folder) -> bool {
        self.turns
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.mine)
            .any(|(i, turn)| {
                // From the drawn turn when it is current, and read afresh when
                // it is not - this is asked with the conversation borrowed, so it
                // cannot bring the drawn turns up to date itself.
                let fresh;
                let edits = match self.drawn.get(i) {
                    Some(d) if d.key == Drawn::key_of(turn) => &d.edits,
                    _ => {
                        fresh = proposals(&lookups(&turn.text).0).1;
                        &fresh
                    }
                };
                edits
                    .iter()
                    .any(|c| c.state.is_none() && c.replacing(folder).is_some())
            })
    }

    /// How much context this conversation is carrying, said in tokens.
    ///
    /// Measured once there is something to measure: the worker counts the
    /// prompt on its way in and says so, and that number is exact. Before the
    /// first question there is nothing to have counted, so what is shown is an
    /// estimate off the characters, marked as one.
    fn weight(&self, folder: &Folder) -> String {
        if self.progress.prompt > 0 {
            return format!("{} TOKENS", round(self.progress.prompt));
        }
        let said: usize = self.turns.iter().map(|t| t.text.len()).sum();
        let here: usize = folder
            .files
            .iter()
            .flat_map(|(_, lines)| lines.iter())
            .map(|l| l.len() + 1)
            .sum();
        format!("~{} TOKENS", round(self.overhead + (said + here) / 4))
    }

    /// One proposed change, with what it would do to the note as it is now.
    ///
    /// The diff is against the *current* note rather than against the note the
    /// model was shown, which is the whole safety of this: if the lines have
    /// moved since it answered, what is drawn is the nonsense that would
    /// actually happen, and nobody presses accept on nonsense.
    fn offer(&self, ui: &mut Ui, width: i32, folder: &Folder, change: &Change) -> Option<bool> {
        let th = *ui.theme;
        let line_h = font::line_h();
        ui.space(3);
        let head = ui.alloc(line_h);
        let span = change.headline(&folder.here);

        // Settled, so there is nothing left to decide and nothing to compare
        // against: the project has already moved on, and a diff against it now
        // would be a diff of the change with itself. What is worth keeping is
        // what it did, in the shape a diff says it in.
        if let Some(taken) = change.state {
            let (plus, minus) = change.tally(folder);
            let (word, tint) = if taken {
                ("APPLIED", th.positive.face)
            } else {
                ("REJECTED", th.ink_soft)
            };
            font::draw_text_styled(ui.canvas, head.x + 4, head.y, &span, th.ink_soft, false);
            let counts = format!("+{plus} -{minus}");
            let at_x = head.right() - font::text_width(&counts);
            font::draw_text_styled(ui.canvas, at_x, head.y, &counts, tint, true);
            let name = Rect::new(
                head.x,
                head.y,
                head.w - font::text_width(&counts) - 8,
                head.h,
            );
            ui.draw_text_in(name, word, tint, Align::Right);
            // One line of what it came to, so the summary says what was done
            // and not only how much of it there was.
            let becoming = change.becoming(folder);
            let gist =
                becoming
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or(match change.what {
                        What::Delete => "(the file was taken away)",
                        _ => "(the lines were taken out)",
                    });
            let row = ui.alloc(line_h);
            ui.draw_text_in(
                Rect::new(row.x + 8, row.y, row.w - 8, row.h),
                &one_line(gist, ((row.w - 16) / font::advance()).max(8) as usize),
                th.ink_soft,
                Align::Left,
            );
            return None;
        }

        let Some(before) = change.replacing(folder) else {
            // Nothing to act on: lines past the end, a file that is not there,
            // one being made that already is, or one in another project. Said
            // rather than offered, because there is no honest diff to draw
            // for any of them - and said which, because the last one has a
            // remedy and the others do not.
            let why = match change.misplaced(folder) {
                Some(project) => {
                    format!(
                        "{span} - IN {} - OPEN IT TO CHANGE IT",
                        project.to_uppercase()
                    )
                }
                None if matches!(change.what, What::Lay { .. }) => {
                    format!("{span} - WHICH IS THERE ALREADY, AND NO LINES SAID")
                }
                None => format!("{span} - WHICH IS NOT THERE TO CHANGE"),
            };
            ui.draw_text_in(head, &why, th.danger.face, Align::Left);
            return None;
        };
        let tint = match change.what {
            What::Delete => th.danger.face,
            What::Write { .. } | What::Lay { .. } | What::Merge { .. } => th.positive.face,
            What::Edit { .. } | What::Insert { .. } => th.info.hi,
        };
        font::draw_text_styled(ui.canvas, head.x + 4, head.y, &span, tint, true);
        // Both answers, side by side and equally reachable. A change offered
        // with only a way to take it is a change you have to take.
        let mut answer = None;
        let no = Rect::new(head.right() - 42, head.y - 2, 42, 13);
        let yes = Rect::new(no.x - 46, head.y - 2, 44, 13);
        if ui.button_at(yes, "ACCEPT", pixui::Tone::Positive).clicked {
            answer = Some(true);
        }
        if ui.button_at(no, "REJECT", pixui::Tone::Neutral).clicked {
            answer = Some(false);
        }

        // The diff, drawn the way the assistant's own block draws one: this is
        // the same question - what would this do to what is there - and two
        // answers to it that looked different would be two things to learn.
        let cols = ((width - 12) / font::advance()).max(8) as usize;
        let pieces = crate::diff::words(&before, &change.becoming(folder));
        for row in crate::assist::rows(&pieces, cols).into_iter().take(WHOLE) {
            let at = ui.alloc(line_h);
            crate::assist::draw_row(ui, Rect::new(at.x + 8, at.y, at.w - 8, at.h), &row);
        }
        // A whole file arriving or leaving is more than anybody reads in a
        // panel, and the count above it already said how much there is.
        let rows = crate::assist::rows(&pieces, cols).len();
        if rows > WHOLE {
            let at = ui.alloc(line_h);
            ui.draw_text_in(
                Rect::new(at.x + 8, at.y, at.w - 8, at.h),
                &format!("... AND {} MORE LINES", rows - WHOLE),
                th.ink_soft,
                Align::Left,
            );
        }
        answer
    }
}
