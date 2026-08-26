//! Conversations, kept beside the note they are about.
//!
//! A chat is not a note. It is persisted like one - a markdown file you can
//! read with anything - but it is not in the vault, does not appear in the
//! sidebar, and is not in the digest the model is given. That last one matters
//! more than it looks: if chats were notes, every new conversation would change
//! the list of notes, and the list of notes is the one part of the prompt worth
//! keeping still.
//!
//! They are filed under the note they were started from, because that is how
//! they are looked for. You are reading something, you wonder about it, and
//! later you want the conversation you had while reading it - not the twelfth
//! conversation you had that Tuesday.
//!
//! The transcript is markdown with the turns marked by headings, so the file
//! reads as itself outside this program. The markers are only markers at the
//! top level of the document; a `## you` inside a fenced code block is code,
//! which is checked on the way in and enforced on the way out.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use pixui::{font, Align, Key, Rect, ScrollState, Ui};

use crate::llm::Turn;
use crate::markdown;
use crate::render;

/// Where a turn begins, and whose it is.
const MINE: &str = "## you";
const THEIRS: &str = "## assistant";

/// How many chats the picker shows before it scrolls.
const ROWS: usize = 10;

/// One conversation.
pub struct Chat {
    /// Where it is filed, once it has been saved. A conversation with nothing
    /// said in it is not written to disk.
    pub path: Option<PathBuf>,
    /// The note it belongs to, by filename.
    pub note: String,
    pub turns: Vec<Turn>,
    /// What is being typed.
    pub draft: String,
    /// True while an answer is on its way.
    pub waiting: bool,
    /// What the worker last said about it.
    pub progress: crate::llm::Progress,
    /// Why the last question failed, if it did.
    pub failed: Option<String>,
    /// True on the frame it opens, so the field takes the keyboard.
    grab: bool,
    scroll: ScrollState,
    /// True when the view should be pinned to the newest turn.
    follow: bool,
}

/// A conversation on disk, as the picker lists it.
pub struct Filed {
    pub path: PathBuf,
    pub title: String,
    /// Turns in it, so a chat can say how long it is.
    pub turns: usize,
    pub when: SystemTime,
}

/// What the chat wants the application to do about it.
pub enum Outcome {
    None,
    /// Send the conversation as it stands.
    Ask,
    /// Take it away.
    Close,
}

impl Chat {
    /// A new conversation about `note`.
    pub fn new(note: String) -> Self {
        Self {
            path: None,
            note,
            turns: Vec::new(),
            draft: String::new(),
            waiting: false,
            progress: crate::llm::Progress::default(),
            failed: None,
            grab: true,
            scroll: ScrollState::default(),
            follow: true,
        }
    }

    /// A conversation read back off disk.
    pub fn open(path: &Path, note: String) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        Self {
            path: Some(path.to_path_buf()),
            turns: parse(&text),
            ..Self::new(note)
        }
    }

    /// What it is called: the first thing that was asked, shortened.
    pub fn title(&self) -> String {
        self.turns
            .iter()
            .find(|t| t.mine)
            .map(|t| one_line(&t.text, 46))
            .unwrap_or_else(|| "NEW CHAT".to_string())
    }

    /// Take what is typed and make it a turn, ready to send.
    pub fn commit(&mut self) {
        let said = std::mem::take(&mut self.draft);
        if said.trim().is_empty() {
            return;
        }
        self.turns.push(Turn {
            mine: true,
            text: said.trim().to_string(),
        });
        self.waiting = true;
        self.failed = None;
        self.follow = true;
    }

    /// File an answer, and write the whole thing down.
    pub fn answered(&mut self, reply: crate::llm::Reply, dir: &Path) {
        self.waiting = false;
        match reply {
            Ok(text) => {
                self.turns.push(Turn {
                    mine: false,
                    text: text.trim().to_string(),
                });
                self.follow = true;
                let _ = self.save(dir);
            }
            // A question that failed is left in the transcript: it is what was
            // asked, and asking it again is a matter of pressing Enter rather
            // than typing it out a second time.
            Err(why) => self.failed = Some(why),
        }
    }

    /// Write it out, choosing a name the first time.
    pub fn save(&mut self, dir: &Path) -> std::io::Result<()> {
        if self.turns.is_empty() {
            return Ok(());
        }
        let home = folder(dir, &self.note);
        std::fs::create_dir_all(&home)?;
        if self.path.is_none() {
            self.path = Some(free_name(&home, &self.title()));
        }
        let path = self.path.as_ref().expect("named above");
        std::fs::write(path, self.to_text())
    }

    /// The transcript as it is filed.
    pub fn to_text(&self) -> String {
        let mut out = format!("# {}\n\n", self.title());
        for turn in &self.turns {
            out.push_str(if turn.mine { MINE } else { THEIRS });
            out.push_str("\n\n");
            out.push_str(&fence_markers(&turn.text));
            out.push_str("\n\n");
        }
        out
    }
}

/// Every conversation filed under a note, newest first.
pub fn filed(dir: &Path, note: &str) -> Vec<Filed> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(folder(dir, note)) else {
        return out;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let turns = parse(&text);
        out.push(Filed {
            title: markdown::derive_title(
                &text.lines().map(str::to_string).collect::<Vec<_>>(),
                46,
            ),
            turns: turns.len(),
            when: entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH),
            path,
        });
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.when));
    out
}

/// Where a note's conversations live.
///
/// Under the vault rather than beside it, and in a directory rather than mixed
/// in: the vault is read as the `.md` files directly inside it, so everything
/// in here is invisible to it without the loader having to know this exists.
pub fn folder(dir: &Path, note: &str) -> PathBuf {
    let stem = note.strip_suffix(".md").unwrap_or(note);
    dir.join("chats").join(stem)
}

/// A file name nothing else has taken.
fn free_name(home: &Path, title: &str) -> PathBuf {
    let slug: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    let slug = if slug.is_empty() {
        "chat".to_string()
    } else {
        slug.chars().take(40).collect()
    };
    for n in 0.. {
        let name = if n == 0 {
            format!("{slug}.md")
        } else {
            format!("{slug}-{n}.md")
        };
        let path = home.join(name);
        if !path.exists() {
            return path;
        }
    }
    unreachable!("a free name exists")
}

/// Read a transcript back into turns.
///
/// A marker only counts at the top level: inside a fenced code block it is
/// code, which is what makes it safe to write a conversation about markdown
/// into a markdown file.
pub fn parse(text: &str) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    let mut mine = true;
    let mut body = String::new();
    let mut started = false;
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") || t.trim_start().starts_with("~~~") {
            in_code = !in_code;
        }
        if !in_code && (t == MINE || t == THEIRS) {
            if started {
                push(&mut turns, mine, &body);
            }
            body.clear();
            mine = t == MINE;
            started = true;
            continue;
        }
        if started {
            body.push_str(line);
            body.push('\n');
        }
    }
    if started {
        push(&mut turns, mine, &body);
    }
    turns
}

fn push(turns: &mut Vec<Turn>, mine: bool, body: &str) {
    let text = body.trim().to_string();
    if !text.is_empty() {
        turns.push(Turn { mine, text });
    }
}

/// Stop a turn's own text from looking like the start of the next one.
///
/// A model asked about this very format will happily write `## assistant` in
/// its answer. One space in front of it is still a heading to nobody and reads
/// the same to everybody.
fn fence_markers(text: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim_end();
        if t.trim_start().starts_with("```") || t.trim_start().starts_with("~~~") {
            in_code = !in_code;
        }
        if !in_code && (t == MINE || t == THEIRS) {
            out.push(' ');
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// The first line of something, for a list that has one line to give it.
fn one_line(text: &str, room: usize) -> String {
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let first = first.trim().trim_start_matches(['#', '>', '-', '*', ' ']);
    if first.chars().count() <= room {
        return first.to_string();
    }
    let head: String = first.chars().take(room).collect();
    let cut = head.rfind(' ').unwrap_or(head.len());
    format!("{}...", head[..cut].trim_end())
}

/// How long ago, in the roundest terms that are still true.
pub fn ago(when: SystemTime) -> String {
    let Ok(gap) = SystemTime::now().duration_since(when) else {
        return "just now".into();
    };
    let secs = gap.as_secs();
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

// ------------------------------------------------------------------ the list

/// Which conversation to open, chosen from the ones there are.
pub struct Picker {
    pub selected: usize,
    fresh: bool,
}

/// What the picker decided.
pub enum Picked {
    None,
    /// Start a conversation with nothing in it.
    Fresh,
    /// Carry on with this one.
    Open(PathBuf),
    Close,
}

impl Picker {
    pub fn new() -> Self {
        Self {
            selected: 0,
            fresh: true,
        }
    }

    /// Draw it, take its keys, and say what was chosen.
    ///
    /// The new-conversation row is one of the rows rather than a button off to
    /// the side, so the same two keys reach everything and starting a new one
    /// costs the same gesture as continuing an old one.
    pub fn show(&mut self, ui: &mut Ui, note: &str, chats: &[Filed]) -> Picked {
        let th = *ui.theme;
        let line_h = font::line_h();
        let screen = ui.canvas.bounds();
        ui.canvas
            .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

        let rows = (chats.len() + 1).min(ROWS);
        let rect = screen.centered(440, rows as i32 * line_h + 4 * line_h + 20);
        let inner = ui.panel(rect, &format!("CHATS ABOUT {}", note.to_uppercase()));
        ui.capture_keyboard();

        let count = chats.len() + 1;
        let mut picked = Picked::None;
        if self.fresh {
            self.fresh = false;
        } else {
            let ctrl = ui.input.mods.ctrl;
            for key in ui.input.keys.clone() {
                match key {
                    Key::Escape => picked = Picked::Close,
                    Key::Enter => picked = self.choose(chats),
                    Key::Down | Key::Char('j') => self.step(1, count),
                    Key::Up | Key::Char('k') => self.step(-1, count),
                    Key::Char('n') if ctrl => self.step(1, count),
                    Key::Char('p') if ctrl => self.step(-1, count),
                    _ => {}
                }
            }
        }
        if !matches!(picked, Picked::None) {
            return picked;
        }

        let (list, foot) = inner.split_bottom(line_h + 2);
        ui.canvas.box_chamfer(list, th.well, th.well_border, 1);
        let mut clicked = None;
        ui.clipped(list.inset(1), |ui| {
            for i in 0..count.min(ROWS) {
                let y = list.y + i as i32 * line_h;
                let row = Rect::new(list.x, y, list.w, line_h);
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
                ui.draw_text_in(at, &chat.title, ink, Align::Left);
                let said = match chat.turns {
                    1 => "1 TURN".to_string(),
                    n => format!("{n} TURNS"),
                };
                ui.draw_text_in(
                    at,
                    &format!("{}  {}", said, ago(chat.when)),
                    dim,
                    Align::Right,
                );
            }
        });

        let said = match chats.len() {
            0 => "NOTHING YET".to_string(),
            1 => "1 CHAT".to_string(),
            n => format!("{n} CHATS"),
        };
        ui.draw_text_in(foot, &said, th.ink_soft, Align::Left);
        ui.draw_text_in(foot, "ENTER OPENS, ESC LEAVES", th.ink_soft, Align::Right);

        match clicked {
            Some(i) => {
                self.selected = i;
                self.choose(chats)
            }
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

/// How much room the field and its hint take at the foot of the panel.
const FOOT: i32 = 34;

impl Chat {
    /// Draw the conversation and take its keys.
    ///
    /// Call this with the rest of the frame wrapped in [`Ui::input_blocked`],
    /// the way the dialogs are: a conversation has the keyboard while it is up.
    pub fn show(&mut self, ui: &mut Ui) -> Outcome {
        let th = *ui.theme;
        let line_h = font::line_h();
        let screen = ui.canvas.bounds();
        ui.canvas
            .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

        let rect = screen.centered((screen.w - 60).min(620), screen.h - 40);
        let inner = ui.panel(rect, &format!("CHAT - {}", self.note.to_uppercase()));
        ui.capture_keyboard();

        let mut outcome = Outcome::None;
        if ui.input.key_pressed(Key::Escape) {
            return Outcome::Close;
        }
        // Enter sends. A conversation is one question at a time, so the key
        // that ends a sentence is the key that asks it; a newline inside one
        // question is a thing to want later and not today.
        if ui.input.key_pressed(Key::Enter) && !self.waiting && !self.draft.trim().is_empty() {
            self.commit();
            outcome = Outcome::Ask;
        }

        let (body, foot) = inner.split_bottom(FOOT);

        // ---- the transcript ------------------------------------------------
        let width = body.w - Ui::SCROLL_GUTTER - 8;
        let mut state = self.scroll;
        ui.scroll_area_with(body, "chat-scroll", &mut state, |ui| {
            if self.turns.is_empty() {
                ui.space(4);
                ui.label_dim("  NOTHING ASKED YET.");
                ui.label_dim("  IT CAN SEE THIS NOTE AND A LINE ABOUT EVERY OTHER ONE.");
                return;
            }
            for turn in &self.turns {
                ui.space(3);
                let head = ui.alloc(line_h);
                let (who, tint) = if turn.mine {
                    ("YOU", th.accent.face)
                } else {
                    ("ASSISTANT", th.positive.face)
                };
                font::draw_text_styled(ui.canvas, head.x + 4, head.y, who, tint, true);
                // A rule from the end of the name to the edge, which is what
                // separates two turns without a box around each of them.
                let from = head.x + 6 + font::advance_width(who);
                ui.canvas.hline(
                    from,
                    head.y + line_h / 2,
                    head.right() - from,
                    th.well_border,
                );
                let lines: Vec<String> = turn.text.lines().map(str::to_string).collect();
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
            if self.waiting {
                ui.space(3);
                let row = ui.alloc(line_h);
                let p = self.progress;
                let said = if p.deliberating {
                    format!("THINKING... {} TOKENS", p.written)
                } else if p.written > 0 {
                    format!("WRITING... {} TOKENS, {:.0}/S", p.written, p.rate())
                } else {
                    "READING THE NOTES...".to_string()
                };
                font::draw_text_styled(ui.canvas, row.x + 4, row.y, &said, th.info.hi, true);
            }
            if let Some(why) = &self.failed {
                ui.space(3);
                let row = ui.alloc(line_h);
                ui.draw_text_in(row, &why.to_uppercase(), th.danger.face, Align::Left);
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

        // ---- what to say next ----------------------------------------------
        let field = Rect::new(foot.x, foot.y + 2, foot.w, 15);
        let hint = if self.waiting {
            "WAITING FOR AN ANSWER"
        } else {
            "ASK SOMETHING"
        };
        let mut draft = std::mem::take(&mut self.draft);
        ui.text_field_grab_at(field, "chat-field", &mut draft, hint, self.grab);
        self.draft = draft;
        self.grab = false;

        let legend = Rect::new(foot.x, field.bottom() + 2, foot.w, line_h);
        ui.draw_text_in(legend, "ENTER ASKS", th.ink_soft, Align::Left);
        ui.draw_text_in(
            legend,
            "ESC LEAVES - IT IS SAVED",
            th.ink_soft,
            Align::Right,
        );
        outcome
    }
}
