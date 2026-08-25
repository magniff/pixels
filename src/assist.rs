//! The editing assistant: ask for a change to a selection, read what came
//! back, and keep it or throw it away.
//!
//! The whole point is that nothing is applied behind your back. A model that
//! rewrites your note the moment it has an opinion is a model you cannot trust
//! with a note; one that shows you a diff and waits is a colleague. So the
//! suggestion arrives as a diff with two buttons, and the buffer is not touched
//! until one of them is pressed.
//!
//! The panel is drawn last, over everything, and owns the keyboard while it is
//! up — the editor underneath is modal, and two modal things fighting over a
//! keystroke is how a keystroke goes missing.

use pixui::{font, Align, Key, Point, Rect, Tone, Ui};

use crate::diff::{self, Change, Piece};
use crate::llm::{Ask, Reply};
use crate::text::Cursor;

const WIDTH: i32 = 320;
/// The most the diff is allowed to take before it scrolls instead of growing.
const DIFF_MAX: i32 = 130;

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
    /// The corner the panel hangs from — where the selection was.
    pub at: Point,
    /// True on the frame the panel appears, so the field takes the keyboard.
    grab: bool,
    scroll: pixui::ScrollState,
}

impl Assist {
    pub fn new(from: Cursor, to: Cursor, source: String, at: Point) -> Self {
        Self {
            phase: Phase::Asking,
            from,
            to,
            source,
            request: String::new(),
            asked: String::new(),
            at,
            grab: true,
            scroll: pixui::ScrollState::default(),
        }
    }

    /// Take an answer from the worker.
    pub fn answered(&mut self, reply: Reply) {
        self.phase = match reply {
            Ok(proposal) => {
                let pieces = diff::words(&self.source, &proposal);
                if diff::is_empty(&pieces) {
                    Phase::Failed("nothing to change".into())
                } else {
                    Phase::Reviewing { proposal, pieces }
                }
            }
            Err(why) => Phase::Failed(why),
        };
    }

    /// Whether the model is working on this one.
    pub fn waiting(&self) -> bool {
        matches!(self.phase, Phase::Thinking)
    }

    pub fn show(&mut self, ui: &mut Ui, model: &str) -> Outcome {
        let th = *ui.theme;
        ui.capture_keyboard();

        // ---- how tall the body needs to be -------------------------------
        let cols = ((WIDTH - 20) / font::ADVANCE).max(8) as usize;
        let rows = match &self.phase {
            Phase::Reviewing { pieces, .. } => layout(pieces, cols),
            _ => Vec::new(),
        };
        let body_h = match &self.phase {
            Phase::Reviewing { .. } => {
                (rows.len() as i32 * font::LINE_H + 4).clamp(font::LINE_H, DIFF_MAX)
            }
            _ => font::LINE_H + 2,
        };
        let footer_h = 15 + 4 + 15;
        // What the panel spends on itself before any of this is drawn: its
        // border, its title strip and the line under it, and the padding
        // inside. Guessed once here rather than discovered by the body coming
        // out short.
        let chrome = 2 + th.metrics.title_h + 1 + th.metrics.pad * 2;
        let height = body_h + 4 + footer_h + chrome;

        // ---- where it hangs ----------------------------------------------
        // Below the selection where there is room, above it where there is
        // not, and never off the side.
        let screen = ui.canvas.bounds();
        let x = self.at.x.min(screen.right() - WIDTH - 4).max(4);
        let below = self.at.y + 4;
        let y = if below + height <= screen.bottom() - 4 {
            below
        } else {
            (self.at.y - height - 6).max(4)
        };
        let rect = Rect::new(x, y, WIDTH, height);
        let inner = ui.panel(rect, "ASSIST");

        // ---- keys ---------------------------------------------------------
        let mut outcome = Outcome::None;
        if ui.input.key_pressed(Key::Escape) {
            return Outcome::Close;
        }
        let submit = ui.input.key_pressed(Key::Enter) && !self.request.trim().is_empty();

        let (body, footer) = inner.split_bottom(footer_h + 4);

        // ---- the body -----------------------------------------------------
        match &self.phase {
            Phase::Asking => {
                let what = one_line(&self.source);
                ui.canvas.fill_rect(body, th.well);
                let at = Rect::new(body.x + 3, body.y + 2, body.w - 6, font::LINE_H);
                ui.draw_text_in(at, &what, th.ink_soft, Align::Left);
            }
            Phase::Thinking => {
                ui.canvas.fill_rect(body, th.well);
                // Three dots that fill and empty: a spinner would be a second
                // idiom for the same idea the press springs already express.
                let dots = ((ui.input.time * 3.0) as usize % 4).min(3);
                let label = format!("{model} IS THINKING{}", ".".repeat(dots));
                let at = Rect::new(body.x + 3, body.y + 2, body.w - 6, font::LINE_H);
                ui.draw_text_in(at, &label, th.info.hi, Align::Left);
            }
            Phase::Failed(why) => {
                ui.canvas.fill_rect(body, th.well);
                let at = Rect::new(body.x + 3, body.y + 2, body.w - 6, font::LINE_H);
                ui.draw_text_in(at, &why.to_uppercase(), th.danger.face, Align::Left);
            }
            Phase::Reviewing { .. } => {
                // The diff is written in the inks the editor uses, which are
                // meant for a dark well; on the panel's cream face they would
                // be invisible. So it gets a well to sit in.
                ui.canvas.fill_rect(body, th.well);
                let mut scroll = self.scroll;
                ui.scroll_area_with(body, "assist-diff", &mut scroll, |ui| {
                    for row in &rows {
                        let at = ui.alloc(font::LINE_H);
                        draw_row(ui, at, row);
                    }
                });
                self.scroll = scroll;
            }
        }

        // ---- the footer ---------------------------------------------------
        let (field, buttons) = footer.split_top(15 + 4);
        let hint = match self.phase {
            Phase::Reviewing { .. } => "ASK FOR ANOTHER CHANGE",
            _ => "WHAT SHOULD CHANGE?",
        };
        let grab = std::mem::take(&mut self.grab);
        ui.text_field_grab_at(
            Rect::new(field.x, field.y, field.w, 15),
            "assist-request",
            &mut self.request,
            hint,
            grab,
        );

        let half = (buttons.w - 4) / 2;
        let left = Rect::new(buttons.x, buttons.y, half, 15);
        let right = Rect::new(buttons.right() - half, buttons.y, half, 15);
        match &self.phase {
            Phase::Reviewing { proposal, .. } => {
                if ui.button_at(left, "APPLY", Tone::Positive).clicked {
                    outcome = Outcome::Apply(proposal.clone());
                }
                if ui.button_at(right, "REJECT", Tone::Danger).clicked {
                    outcome = Outcome::Close;
                }
            }
            Phase::Thinking => {
                ui.button_at(left, "WORKING", Tone::Neutral);
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

/// Break the diff into rows of words that fit the width.
fn layout(pieces: &[Piece], cols: usize) -> Vec<Vec<(Change, String)>> {
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
