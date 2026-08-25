//! The editing assistant: ask for a change to a selection, read what came
//! back, and keep it or throw it away.
//!
//! It sits *in* the text rather than over it. A panel floating beside the
//! selection covers the words either side of the one thing you are trying to
//! judge; a block opened between the lines pushes them apart instead, and the
//! note reads as a note with a question in it. The editor scrolls it like any
//! other rows, because that is what it is made of.
//!
//! The whole point is that nothing is applied behind your back. A model that
//! rewrites your note the moment it has an opinion is a model you cannot trust
//! with a note; one that shows you a diff and waits is a colleague. So the
//! suggestion arrives as a diff with two buttons, and the buffer is not touched
//! until one of them is pressed.

use pixui::{font, Align, Key, Rect, Tone, Ui};

use crate::diff::{self, Change, Piece};
use crate::llm::{Ask, Reply};
use crate::text::Cursor;

/// The most rows of diff to open up for. Past this it is a rewrite rather than
/// an edit, and burying the note under it helps nobody.
const DIFF_ROWS: usize = 10;
/// Height of the row holding the question and the buttons.
const CONTROLS: i32 = 15;

/// Where the conversation has got to.
pub enum Phase {
    /// Waiting for a request to be typed.
    Asking,
    /// Sent, and waiting.
    Thinking,
    /// An answer came back and is being looked at.
    Reviewing {
        proposal: String,
        pieces: Vec<Piece>,
    },
    Failed(String),
}

/// What the panel wants the application to do about it.
pub enum Outcome {
    None,
    /// Put this question to the model.
    Ask(Ask),
    /// Replace the selection with this text.
    Apply(String),
    /// Take the panel away and leave the note alone.
    Close,
}

pub struct Assist {
    pub phase: Phase,
    /// The range this is about, frozen when the panel opened: the selection
    /// underneath may be anything by the time an answer arrives, and the answer
    /// is about what was selected when it was asked.
    pub from: Cursor,
    pub to: Cursor,
    /// The text as it was, which is one side of every diff drawn here.
    pub source: String,
    /// What is being typed.
    pub request: String,
    /// What was last sent, kept above the diff so the answer has a question.
    pub asked: String,
    /// True on the frame the block appears, so the field takes the keyboard.
    grab: bool,
}

impl Assist {
    pub fn new(from: Cursor, to: Cursor, source: String) -> Self {
        Self {
            phase: Phase::Asking,
            from,
            to,
            source,
            request: String::new(),
            asked: String::new(),
            grab: true,
        }
    }

    /// The line the block opens under: the last one the selection covered.
    pub fn anchor(&self) -> usize {
        self.to.line
    }

    /// How tall the block needs to be at this width.
    ///
    /// Measured the same way it is drawn, from the same word layout, so the
    /// space reserved for it and the space it takes cannot disagree.
    pub fn height(&self, width: i32) -> i32 {
        let body = match &self.phase {
            Phase::Reviewing { pieces, .. } => {
                rows(pieces, cols(width)).len().min(DIFF_ROWS + 1) as i32 * font::LINE_H
            }
            _ => font::LINE_H,
        };
        // Top rule, header, body, controls, and a pixel of air at each seam.
        3 + font::LINE_H + 2 + body + 3 + CONTROLS + 3
    }

    /// Take an answer from the worker.
    pub fn answered(&mut self, reply: Reply) {
        self.phase = match reply {
            Ok(proposal) => {
                let pieces = diff::words(&self.source, &proposal);
                if diff::is_empty(&pieces) {
                    // A small model handed the text back rather than admit it
                    // had no idea what to do with the instruction. Say which of
                    // the two happened, since only one of them is worth
                    // retrying.
                    Phase::Failed("no change suggested - try a more specific instruction".into())
                } else {
                    Phase::Reviewing { proposal, pieces }
                }
            }
            Err(why) => Phase::Failed(why),
        };
        // A question that came to nothing goes back in the box, so it can be
        // edited into a better one rather than typed again from scratch.
        if matches!(self.phase, Phase::Failed(_)) && self.request.is_empty() {
            self.request = std::mem::take(&mut self.asked);
        }
    }

    /// One line about where this has got to, for the status bar.
    ///
    /// The panel cannot say it itself: its own hint line only shows while the
    /// field is unfocused, and the field is focused the whole time the block is
    /// open. The status bar is already where this app says what just happened.
    pub fn headline(&self) -> String {
        match &self.phase {
            Phase::Asking => "ASK FOR A CHANGE, ENTER TO SEND".to_string(),
            Phase::Thinking => "WORKING ON IT".to_string(),
            Phase::Reviewing { .. } => format!("SUGGESTED - {} KEEPS IT", chord()),
            Phase::Failed(why) => why.to_uppercase(),
        }
    }

    /// Whether the model is working on this one.
    pub fn waiting(&self) -> bool {
        matches!(self.phase, Phase::Thinking)
    }

    /// Draw the block into the rows the editor opened for it.
    pub fn show(&mut self, ui: &mut Ui, rect: Rect, model: &str) -> Outcome {
        let th = *ui.theme;
        ui.capture_keyboard();

        // A well the width of the text, with a lit top edge: it reads as a
        // drawer opened between two lines rather than as something dropped on
        // top of them.
        ui.canvas.fill_rect(rect, th.well.shade(0.06));
        ui.canvas.hline(rect.x, rect.y, rect.w, th.accent.lo);
        ui.canvas
            .hline(rect.x, rect.bottom() - 1, rect.w, th.panel_border);

        let mut outcome = Outcome::None;
        if ui.input.key_pressed(Key::Escape) {
            return Outcome::Close;
        }
        // Enter asks; Enter with a modifier keeps. Both are the key the hand is
        // already on, which is the point of having them at all.
        let enter = ui.input.key_pressed(Key::Enter);
        let held = ui.input.mods.ctrl || ui.input.mods.cmd;
        let submit = enter && !held && !self.request.trim().is_empty();
        let keep = enter && held;

        // ---- header -------------------------------------------------------
        let head = Rect::new(rect.x + 3, rect.y + 3, rect.w - 6, font::LINE_H);
        let (badge, tint) = match &self.phase {
            Phase::Thinking => ("THINKING", th.info.hi),
            Phase::Reviewing { .. } => ("SUGGESTED", th.positive.face),
            Phase::Failed(_) => ("FAILED", th.danger.face),
            Phase::Asking => ("ASSIST", th.accent.face),
        };
        font::draw_text_styled(ui.canvas, head.x, head.y, badge, tint, true);
        let name = Rect::new(head.x, head.y, head.w, head.h);
        ui.draw_text_in(name, model, th.ink_soft, Align::Right);

        // ---- body ---------------------------------------------------------
        let body = Rect::new(
            rect.x + 3,
            head.bottom() + 2,
            rect.w - 6,
            rect.h - (head.bottom() + 2 - rect.y) - CONTROLS - 6,
        );
        match &self.phase {
            Phase::Asking => {
                let what = one_line(&self.source);
                ui.draw_text_in(body, &what, th.ink_soft, Align::Left);
            }
            Phase::Thinking => {
                // Three dots that fill and empty. A spinner would be a second
                // idiom for something the press springs already say.
                let dots = ((ui.input.time * 3.0) as usize % 4).min(3);
                let label = format!("WORKING ON IT{}", ".".repeat(dots));
                ui.draw_text_in(body, &label, th.info.hi, Align::Left);
            }
            Phase::Failed(why) => {
                ui.draw_text_in(body, &why.to_uppercase(), th.danger.face, Align::Left);
            }
            Phase::Reviewing { pieces, .. } => {
                let all = rows(pieces, cols(rect.w - 6));
                let shown = all.len().min(DIFF_ROWS);
                for (i, row) in all.iter().take(shown).enumerate() {
                    let at = Rect::new(
                        body.x,
                        body.y + i as i32 * font::LINE_H,
                        body.w,
                        font::LINE_H,
                    );
                    draw_row(ui, at, row);
                }
                if all.len() > shown {
                    let at = Rect::new(
                        body.x,
                        body.y + shown as i32 * font::LINE_H,
                        body.w,
                        font::LINE_H,
                    );
                    let more = all.len() - shown;
                    ui.draw_text_in(at, &format!("+{more} MORE LINES"), th.ink_soft, Align::Left);
                }
            }
        }

        // ---- the question, and what to do with the answer -----------------
        let controls = Rect::new(
            rect.x + 3,
            rect.bottom() - CONTROLS - 3,
            rect.w - 6,
            CONTROLS,
        );
        let button_w = 62;
        let (wide, rest) = controls.split_left(controls.w - button_w * 2 - 8);
        let left = Rect::new(rest.x + 4, rest.y, button_w, CONTROLS);
        let right = Rect::new(left.right() + 4, rest.y, button_w, CONTROLS);

        let hint = match self.phase {
            Phase::Reviewing { .. } => "ASK FOR ANOTHER CHANGE",
            _ => "WHAT SHOULD CHANGE?",
        };
        let grab = std::mem::take(&mut self.grab);
        ui.text_field_grab_at(wide, "assist-request", &mut self.request, hint, grab);

        match &self.phase {
            Phase::Reviewing { proposal, .. } => {
                if ui.button_at(left, "APPLY", Tone::Positive).clicked || keep {
                    outcome = Outcome::Apply(proposal.clone());
                }
                if ui.button_at(right, "REJECT", Tone::Danger).clicked {
                    outcome = Outcome::Close;
                }
            }
            Phase::Thinking => {
                if ui.button_at(right, "CANCEL", Tone::Neutral).clicked {
                    outcome = Outcome::Close;
                }
            }
            _ => {
                let ask = ui.button_at(left, "ASK", Tone::Accent).clicked;
                if ui.button_at(right, "CANCEL", Tone::Neutral).clicked {
                    outcome = Outcome::Close;
                }
                if ask && !self.request.trim().is_empty() {
                    outcome = Outcome::Ask(self.send());
                }
            }
        }
        // Enter sends whatever is typed, whichever phase we are in: a follow-up
        // to a suggestion is asked the same way the first question was.
        if submit && !matches!(self.phase, Phase::Thinking) {
            outcome = Outcome::Ask(self.send());
        }
        outcome
    }

    /// Take what is typed and turn it into a question.
    ///
    /// A follow-up asks about the suggestion on screen rather than the original
    /// text — "now make it shorter" means shorter than what you are looking at.
    fn send(&mut self) -> Ask {
        let source = match &self.phase {
            Phase::Reviewing { proposal, .. } => proposal.clone(),
            _ => self.source.clone(),
        };
        self.asked = std::mem::take(&mut self.request);
        self.phase = Phase::Thinking;
        Ask {
            source,
            request: self.asked.clone(),
        }
    }
}

/// What to call the modifier on this keyboard.
///
/// The toolkit maps `cmd` onto whichever key the platform means by "the
/// primary one", so the binding is the same everywhere and only its name
/// changes.
fn chord() -> &'static str {
    if cfg!(target_os = "macos") {
        "CMD-ENTER"
    } else {
        "CTRL-ENTER"
    }
}

/// The first line of the selection, shortened, for saying what is being edited.
fn one_line(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let cut: String = first.chars().take(28).collect();
    let tail = if first.chars().count() > 28 {
        "..."
    } else {
        ""
    };
    format!("EDITING \"{cut}{tail}\"").to_uppercase()
}

/// How many characters fit in a block this wide.
fn cols(width: i32) -> usize {
    ((width / font::ADVANCE).max(8)) as usize
}

/// Break the diff into rows of words that fit the width.
fn rows(pieces: &[Piece], cols: usize) -> Vec<Vec<(Change, String)>> {
    let mut rows: Vec<Vec<(Change, String)>> = vec![Vec::new()];
    let mut used = 0usize;
    for piece in pieces {
        if piece.text == "\n" {
            rows.push(Vec::new());
            used = 0;
            continue;
        }
        for word in piece.text.split(' ') {
            let len = word.chars().count();
            if used > 0 && used + 1 + len > cols {
                rows.push(Vec::new());
                used = 0;
            }
            let sep = if used > 0 { 1 } else { 0 };
            used += sep + len;
            let row = rows.last_mut().expect("a row is always open");
            match row.last_mut() {
                Some((change, text)) if *change == piece.change => {
                    text.push(' ');
                    text.push_str(word);
                }
                _ => row.push((piece.change, word.to_string())),
            }
        }
    }
    rows
}

/// One row of the diff: kept words in the ordinary ink, removed ones struck
/// through in red, added ones in green.
fn draw_row(ui: &mut Ui, rect: Rect, row: &[(Change, String)]) {
    let th = *ui.theme;
    let mut x = rect.x;
    for (change, text) in row {
        let w = font::advance_width(text);
        let color = match change {
            Change::Same => th.ink_light,
            Change::Removed => th.danger.face,
            Change::Added => th.positive.face,
        };
        if *change != Change::Same {
            let tint = if *change == Change::Added {
                th.positive.lo
            } else {
                th.danger.lo
            };
            ui.canvas
                .fill_rect(Rect::new(x - 1, rect.y - 1, w + 1, font::LINE_H), tint);
        }
        font::draw_text(ui.canvas, x, rect.y, text, color);
        if *change == Change::Removed {
            ui.canvas
                .hline(x, rect.y + font::GLYPH_H / 2, w - 1, th.danger.hi);
        }
        x += w + font::ADVANCE;
    }
}

/// How big the mark beside a selection is.
pub const MARK: i32 = 11;
