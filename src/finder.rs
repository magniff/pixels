//! Jump to a note by typing a few of its letters.
//!
//! The sidebar filters the notes it is already showing, which is the right
//! thing when the list is short enough to look at. Past that it stops being a
//! list and becomes a library, and the question changes from "which of these?"
//! to "where is the one about X?" — asked from the keyboard, answered without
//! taking your hands off it, and gone again once it is answered.
//!
//! So: a box, every note that answers it, ranked. The matching is a
//! subsequence rather than a substring — `mdsh` finds `markdown-showcase.md` —
//! which is the part that makes it feel like the thing it is modelled on. The
//! controls are the sidebar's, because there is no reason for two filtered
//! lists in one app to be driven differently.

use pixui::{font, Align, Key, Rect, Ui};

/// What the finder wants the application to do about it.
pub enum Found {
    /// Still open, still deciding.
    None,
    /// Open this note.
    Open(usize),
    /// Take it away and leave things as they were.
    Close,
}

/// A note, as far as the finder is concerned.
pub struct Candidate {
    pub title: String,
    pub file: String,
}

/// Which of a note's two names answered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum On {
    Title,
    File,
}

/// One note that answered the query.
pub struct Hit {
    /// Which note it is, in the order they were handed over.
    pub note: usize,
    /// How well it answered, for the ranking.
    pub score: i32,
    /// The name that answered, and which characters of it did — a match on the
    /// filename lights the filename, not the title that happens to share a
    /// letter with it.
    pub on: On,
    pub at: Vec<usize>,
}

pub struct Finder {
    pub query: String,
    pub selected: usize,
    /// True on the frame it opens, so the keystroke that opened it does not
    /// also act on it.
    fresh: bool,
    /// First row on screen, when the hits outrun the list.
    scroll: usize,
}

/// How many hits the panel shows at once.
const ROWS: usize = 12;

impl Finder {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            fresh: true,
            scroll: 0,
        }
    }

    /// Draw it, take its keys, and say what it decided.
    ///
    /// Call this after the rest of the frame, with the rest wrapped in
    /// [`Ui::input_blocked`], the way the file dialogs are.
    pub fn show(&mut self, ui: &mut Ui, notes: &[Candidate]) -> Found {
        let th = *ui.theme;
        let line_h = font::line_h();
        let screen = ui.canvas.bounds();
        ui.canvas
            .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

        let hits = search(notes, &self.query);
        self.selected = self.selected.min(hits.len().saturating_sub(1));

        // A fixed height rather than one that follows the hit count: a panel
        // that resizes under every keystroke is a panel that is hard to read
        // while typing into it.
        let rect = screen.centered(420, ROWS as i32 * line_h + 4 * line_h + 26);
        let inner = ui.panel(rect, "FIND A NOTE");
        ui.capture_keyboard();

        let mut found = Found::None;
        if self.fresh {
            self.fresh = false;
        } else {
            let count = hits.len();
            let ctrl = ui.input.mods.ctrl;
            for key in ui.input.keys.clone() {
                match key {
                    Key::Escape => found = Found::Close,
                    Key::Enter if count > 0 => found = Found::Open(hits[self.selected].note),
                    Key::Down => self.step(1, count),
                    Key::Up => self.step(-1, count),
                    // The other spelling of the same two, for a hand that never
                    // leaves the home row while typing a query. The sidebar
                    // walks on the same pair.
                    Key::Char('n') if ctrl => self.step(1, count),
                    Key::Char('p') if ctrl => self.step(-1, count),
                    _ => {}
                }
            }
        }
        if matches!(found, Found::Close | Found::Open(_)) {
            return found;
        }

        let (top, foot) = inner.split_bottom(line_h + 2);
        let (box_row, list) = top.split_top(17);

        // ---- the query ----------------------------------------------------
        // Grabbed every frame: the panel is only up while it is being typed
        // into, and there is nothing else here to give the keyboard to.
        let field = Rect::new(box_row.x, box_row.y, box_row.w, 15);
        let mut query = std::mem::take(&mut self.query);
        let before = query.clone();
        ui.search_field_grab_at(field, "finder", &mut query, "TYPE A BIT OF THE NAME", true);
        if query != before {
            // A new query is a new list; staying on row seven of a list that
            // has changed underneath is how you end up opening something else.
            self.selected = 0;
            self.scroll = 0;
        }
        self.query = query;

        // ---- the hits ------------------------------------------------------
        // Kept in view by scrolling the window of rows rather than the panel:
        // the selection walks, the panel does not move.
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + ROWS {
            self.scroll = self.selected + 1 - ROWS;
        }

        let mut clicked = None;
        // A well under the rows, so the list reads as a list rather than as
        // text lying on the panel.
        ui.canvas.box_chamfer(list, th.well, th.well_border, 1);
        ui.clipped(list.inset(1), |ui| {
            if hits.is_empty() {
                let at = Rect::new(list.x + 4, list.y + 2, list.w - 8, line_h);
                ui.draw_text_in(at, "NO NOTES MATCH", th.ink_soft, Align::Left);
                return;
            }
            // The filename column starts where the longest title on screen
            // ends, so the two names line up into columns instead of drifting.
            let widest = hits
                .iter()
                .skip(self.scroll)
                .take(ROWS)
                .map(|h| notes[h.note].title.chars().count())
                .max()
                .unwrap_or(0)
                .clamp(8, 26);
            let split = (widest + 2) as i32 * font::advance();

            for (i, hit) in hits.iter().enumerate().skip(self.scroll).take(ROWS) {
                let y = list.y + (i - self.scroll) as i32 * line_h;
                let row = Rect::new(list.x, y, list.w, line_h);
                let id = ui.id(&format!("hit{i}"));
                let resp = ui.interact(id, row);
                let picked = i == self.selected;
                if picked {
                    ui.canvas.fill_rect(row, th.accent.lo);
                } else if resp.hovered {
                    ui.canvas.fill_rect(row, th.well.shade(0.12));
                }
                if resp.clicked {
                    clicked = Some(hit.note);
                }

                // On the selected row the ink is the one the theme picked to
                // read against that fill; everywhere else it is the list's own.
                // The lit characters differ by weight rather than by colour
                // there, because a second colour on a coloured row is a guess
                // at a contrast nobody measured.
                let (title_ink, file_ink, lit_ink) = if picked {
                    (th.accent.ink, th.accent.ink, th.accent.ink)
                } else {
                    (th.ink_light, th.ink_soft, th.accent.hi)
                };
                let note = &notes[hit.note];
                draw_lit(
                    ui,
                    row.x + 4,
                    y,
                    &note.title,
                    (hit.on == On::Title).then_some(&hit.at),
                    title_ink,
                    lit_ink,
                    widest,
                );
                draw_lit(
                    ui,
                    row.x + 4 + split,
                    y,
                    &note.file,
                    (hit.on == On::File).then_some(&hit.at),
                    file_ink,
                    lit_ink,
                    ((row.w - split - 8) / font::advance()).max(1) as usize,
                );
            }
        });

        // ---- how many, and how to leave -------------------------------------
        let count = match hits.len() {
            0 => "NOTHING".to_string(),
            1 => "1 NOTE".to_string(),
            n => format!("{n} NOTES"),
        };
        ui.draw_text_in(foot, &count, th.ink_soft, Align::Left);
        ui.draw_text_in(foot, "ENTER OPENS, ESC LEAVES", th.ink_soft, Align::Right);

        match clicked {
            Some(note) => Found::Open(note),
            None => Found::None,
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

impl Default for Finder {
    fn default() -> Self {
        Self::new()
    }
}

/// One name, with the characters that answered the query lit.
///
/// Drawn a character at a time because that is what lighting some of them
/// costs: the run-based text drawing would need the runs worked out first, and
/// the runs are what this is.
#[allow(clippy::too_many_arguments)]
fn draw_lit(
    ui: &mut Ui,
    x0: i32,
    y: i32,
    text: &str,
    at: Option<&Vec<usize>>,
    plain: pixui::Color,
    lit_ink: pixui::Color,
    room: usize,
) {
    for (i, ch) in text.chars().take(room).enumerate() {
        let lit = at.is_some_and(|a| a.contains(&i));
        let ink = if lit { lit_ink } else { plain };
        let x = x0 + i as i32 * font::advance();
        font::draw_text_styled(ui.canvas, x, y, &ch.to_string(), ink, lit);
    }
}

/// The notes that answer `query`, best first.
///
/// An empty query is all of them, in the order they were handed over — the
/// panel opens onto the library rather than onto nothing.
pub fn search(notes: &[Candidate], query: &str) -> Vec<Hit> {
    let needle = query.trim();
    let mut hits: Vec<Hit> = Vec::new();
    for (note, cand) in notes.iter().enumerate() {
        if needle.is_empty() {
            hits.push(Hit {
                note,
                score: 0,
                on: On::Title,
                at: Vec::new(),
            });
            continue;
        }
        // Both names are worth typing at: one is what the note calls itself and
        // the other is what it is filed under, and which one somebody has in
        // mind is not knowable from four letters. The better match wins, and
        // the title takes a tie because it is the one being read.
        // The extension is not part of the name: every note here ends in
        // `.md`, so scoring against it means the letters m and d match every
        // note in the library and the ranking is decided by an accident of
        // filing rather than by what anybody typed.
        let stem = cand.file.rsplit_once('.').map_or(&cand.file[..], |(s, _)| s);
        let on_title = fuzzy(&cand.title, needle).map(|(s, at)| (s, On::Title, at));
        let on_file = fuzzy(stem, needle).map(|(s, at)| (s - 1, On::File, at));
        let best = match (on_title, on_file) {
            (Some(a), Some(b)) => Some(if b.0 > a.0 { b } else { a }),
            (some, None) | (None, some) => some,
        };
        if let Some((score, on, at)) = best {
            hits.push(Hit {
                note,
                score,
                on,
                at,
            });
        }
    }
    // Best first, and ties in the order they came: two notes that answer
    // equally well are best offered in the order the sidebar has them.
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.note.cmp(&b.note)));
    hits
}

/// Score `text` against `needle`, and say which characters answered.
///
/// A subsequence match: every character of the needle appears in order, not
/// necessarily together. What the score is for is deciding which of the many
/// names that satisfy that are the ones somebody meant — a run of characters
/// together is worth far more than the same characters scattered, and a match
/// at the start of a word is worth more than one in the middle of one.
pub fn fuzzy(text: &str, needle: &str) -> Option<(i32, Vec<usize>)> {
    let hay: Vec<char> = text.chars().collect();
    let mut score = 0;
    let mut at = Vec::new();
    let mut from = 0usize;
    let mut last: Option<usize> = None;
    for want in needle.chars().flat_map(char::to_lowercase) {
        let found = (from..hay.len())
            .find(|&i| hay[i].to_lowercase().next().is_some_and(|c| c == want))?;
        score += 1;
        if last == Some(found.wrapping_sub(1)) {
            // Contiguous, which is the strongest signal there is that this is
            // the word rather than a coincidence of letters.
            score += 8;
        }
        let starts_word = found == 0 || hay.get(found - 1).is_some_and(|c| !c.is_alphanumeric());
        if starts_word {
            score += 4;
        }
        at.push(found);
        last = Some(found);
        from = found + 1;
    }
    // A short name that answered is more likely the one than a long name that
    // happens to contain the letters, and an early match beats a late one.
    score += 10 - (hay.len() as i32 / 4).min(9);
    score -= (at.first().copied().unwrap_or(0) as i32 / 4).min(6);
    Some((score, at))
}
