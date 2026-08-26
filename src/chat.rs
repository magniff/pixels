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

/// One thing that can be typed with a slash in front of it.
pub struct Command {
    pub name: &'static str,
    /// What it takes after the name, for the listing and for completing it.
    pub takes: &'static str,
    pub what: &'static str,
}

/// Every command there is.
///
/// The table is what `/help` prints and what completion offers, so a command
/// that is not in it does not exist as far as anybody typing is concerned.
pub const COMMANDS: &[Command] = &[
    Command {
        name: "rename",
        takes: " <name>",
        what: "call this conversation something",
    },
    Command {
        name: "help",
        takes: "",
        what: "list what can be typed here",
    },
];

/// The commands, one per line, laid out in a column.
fn manual() -> String {
    let widest = COMMANDS
        .iter()
        .map(|c| c.name.len() + c.takes.len())
        .max()
        .unwrap_or(0);
    COMMANDS
        .iter()
        .map(|c| {
            let head = format!("/{}{}", c.name, c.takes);
            format!("{head:<0$}   {1}", widest + 1, c.what)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The commands a half-typed one could still become.
///
/// Nothing once there is a space: the name is settled by then and what follows
/// is an argument, which this knows nothing about.
pub fn completions(draft: &str) -> Vec<&'static Command> {
    let Some(rest) = draft.strip_prefix('/') else {
        return Vec::new();
    };
    if rest.contains(' ') {
        return Vec::new();
    }
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(rest))
        .collect()
}

/// What Tab should make of a half-typed command.
///
/// The longest beginning every candidate shares, the way a shell completes:
/// one match finishes the word, several take it as far as they agree.
pub fn complete(draft: &str) -> Option<String> {
    let hits = completions(draft);
    let first = hits.first()?;
    let mut common = first.name.to_string();
    for hit in &hits[1..] {
        let keep = common
            .chars()
            .zip(hit.name.chars())
            .take_while(|(a, b)| a == b)
            .count();
        common.truncate(keep);
    }
    if hits.len() == 1 {
        // A finished name gets the space its argument would go after, so the
        // next keystroke is the argument rather than another Tab.
        return Some(format!(
            "/{}{}",
            common,
            if first.takes.is_empty() { "" } else { " " }
        ));
    }
    Some(format!("/{common}"))
}

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
    /// What it has been called, if it has been given a name. Otherwise it is
    /// known by the first thing that was asked in it.
    pub name: Option<String>,
    pub turns: Vec<Turn>,
    /// What is being typed.
    pub draft: String,
    /// True while an answer is on its way.
    pub waiting: bool,
    /// What the worker last said about it.
    pub progress: crate::llm::Progress,
    /// Why the last question failed, if it did.
    pub failed: Option<String>,
    /// Something worth saying that is not a failure - what a command did.
    pub notice: Option<String>,
    /// True on the frame it opens, so the field takes the keyboard.
    grab: bool,
    /// True on the frame a completion rewrote the draft, so the field picks up
    /// the new text and puts the caret after it.
    retype: bool,
    scroll: ScrollState,
    /// True when the view should be pinned to the newest turn.
    follow: bool,
    /// Roughly how many tokens the context around the conversation comes to -
    /// the vault list and the note. Told to it once by the application, which
    /// is the thing that assembles them, rather than worked out every frame.
    pub overhead: usize,
    /// Which proposals have been settled, and how, by turn and position in it.
    /// A change is offered until it is answered, and then it says what happened
    /// instead of asking again.
    applied: std::collections::HashMap<(usize, usize), bool>,
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
    /// Something changed that should be written down.
    Save,
    /// Put this change into the note.
    Apply(Edit),
    /// Take it away.
    Close,
}

/// A change to the note, as the model proposed it.
///
/// Line numbers rather than text to find: the note is shown numbered, and a
/// number cannot be misquoted. Both are one-based and inclusive, the way they
/// are written in the margin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub from: usize,
    pub to: usize,
    /// What those lines should become. Empty means delete them.
    pub text: String,
}

impl Edit {
    /// The lines it would replace, as they are now.
    pub fn replacing(&self, lines: &[String]) -> Option<String> {
        let from = self.from.checked_sub(1)?;
        if from >= lines.len() || self.to < self.from {
            return None;
        }
        let to = self.to.min(lines.len());
        Some(lines[from..to].join("\n"))
    }
}

/// Split a reply into what it said and what it proposed.
///
/// The blocks are lifted out of the prose rather than left in it: a reply is
/// read as a sentence and a change, and showing the raw block would be showing
/// somebody the machinery instead of the change.
pub fn proposals(reply: &str) -> (String, Vec<Edit>) {
    let mut prose = String::new();
    let mut edits = Vec::new();
    let mut rest = reply;
    let mut in_code = false;
    while let Some(at) = rest.find("<edit") {
        // A block inside a fence is a block being talked about, not one being
        // made. Counting fences up to the marker is enough to tell which.
        in_code ^= fences(&rest[..at]);
        let head = &rest[..at];
        if in_code {
            prose.push_str(&rest[..at + 5]);
            rest = &rest[at + 5..];
            continue;
        }
        let Some(open) = rest[at..].find('>').map(|i| at + i + 1) else {
            break;
        };
        let Some(close) = rest[open..].find("</edit>").map(|i| open + i) else {
            break;
        };
        if let Some(range) = lines_attr(&rest[at..open]) {
            edits.push(Edit {
                from: range.0,
                to: range.1,
                text: rest[open..close].trim_matches('\n').to_string(),
            });
            prose.push_str(head);
        } else {
            // Not a range this understands. Left in the prose rather than
            // swallowed, so a malformed block is visible instead of missing.
            prose.push_str(&rest[..close + 7]);
        }
        rest = &rest[close + 7..];
    }
    prose.push_str(rest);
    (prose.trim().to_string(), edits)
}

/// Whether an odd number of code fences opened in this text.
fn fences(text: &str) -> bool {
    text.lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("```") || t.starts_with("~~~")
        })
        .count()
        % 2
        == 1
}

/// `lines="12-14"`, or `lines="12"` for a single one.
fn lines_attr(tag: &str) -> Option<(usize, usize)> {
    let at = tag.find("lines")?;
    let value = tag[at..].split('"').nth(1)?;
    let value = value.trim();
    match value.split_once('-') {
        Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
        None => {
            let one = value.parse().ok()?;
            Some((one, one))
        }
    }
}

impl Chat {
    /// A new conversation about `note`.
    pub fn new(note: String) -> Self {
        Self {
            path: None,
            note,
            name: None,
            turns: Vec::new(),
            draft: String::new(),
            waiting: false,
            progress: crate::llm::Progress::default(),
            failed: None,
            notice: None,
            grab: true,
            retype: false,
            scroll: ScrollState::default(),
            follow: true,
            overhead: 0,
            applied: std::collections::HashMap::new(),
        }
    }

    /// A conversation read back off disk.
    pub fn open(path: &Path, note: String) -> Self {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        // The heading is taken as the name rather than re-derived: a chat that
        // was renamed keeps the name it was given, and one that never was gets
        // back the same first-question title it was filed under.
        let name = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("# "))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        Self {
            path: Some(path.to_path_buf()),
            turns: parse(&text),
            name,
            ..Self::new(note)
        }
    }

    /// What it is called: the first thing that was asked, shortened.
    pub fn title(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        self.turns
            .iter()
            .find(|t| t.mine)
            .map(|t| one_line(&t.text, 46))
            .unwrap_or_else(|| "NEW CHAT".to_string())
    }

    /// Run a `/` command, and say whether it was one.
    ///
    /// Commands are typed into the same field as questions, told apart by the
    /// leading slash - the same bargain the editor's own `:` line makes, and
    /// one less thing on screen than a second box for them would be.
    pub fn command(&mut self, line: &str) -> bool {
        let Some(rest) = line.trim().strip_prefix('/') else {
            return false;
        };
        let (word, arg) = rest.split_once(' ').unwrap_or((rest, ""));
        let arg = arg.trim();
        match word {
            "rename" if !arg.is_empty() => {
                self.name = Some(one_line(arg, 60));
                self.notice = Some(format!("renamed to \"{}\"", self.title()));
            }
            "rename" => self.notice = Some("rename to what? /rename <name>".into()),
            "help" => self.notice = Some(manual()),
            other => self.notice = Some(format!("no command called /{other} - /help lists them")),
        }
        self.draft.clear();
        true
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

    /// Remember what happened to a proposal, so it is not offered again.
    pub fn settled(&mut self, edit: &Edit, taken: bool) {
        for (i, turn) in self.turns.iter().enumerate() {
            if turn.mine {
                continue;
            }
            for (j, other) in proposals(&turn.text).1.iter().enumerate() {
                if other == edit {
                    self.applied.insert((i, j), taken);
                }
            }
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

/// A count, shortened once it stops being worth reading digit by digit.
fn round(n: usize) -> String {
    if n < 1000 {
        return n.to_string();
    }
    format!("{:.1}K", n as f32 / 1000.0)
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
    pub fn show(&mut self, ui: &mut Ui, note: &str, chats: &[Filed]) -> Picked {
        let th = *ui.theme;
        let line_h = font::line_h();
        let screen = ui.canvas.bounds();
        ui.canvas
            .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

        let rows = (chats.len() + 1).min(ROWS);
        let rect = screen.centered(440, rows as i32 * line_h + 4 * line_h + 26);
        let inner = ui.panel(rect, &format!("CHATS ABOUT {}", note.to_uppercase()));
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
                let bin = Rect::new(row.right() - 11, y + 1, 9, 7);
                let over = resp.hovered || picked_row;
                let id = ui.id(&format!("bin{i}"));
                let hit = ui.interact(id, Rect::new(bin.x - 2, y, 13, line_h));
                if over || hit.hovered {
                    let tint = if hit.hovered { th.danger.hi } else { dim };
                    pixui::icon::draw(ui.canvas, bin.x, bin.y, pixui::icon::BIN, tint);
                }
                if hit.clicked {
                    asking = Some(i);
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

/// How much room the field and its hint take at the foot of the panel.
const FOOT: i32 = 34;

impl Chat {
    /// Draw the conversation and take its keys.
    ///
    /// Call this with the rest of the frame wrapped in [`Ui::input_blocked`],
    /// the way the dialogs are: a conversation has the keyboard while it is up.
    pub fn show(&mut self, ui: &mut Ui, note: &[String]) -> Outcome {
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
        let inner = ui.panel_badged(rect, &self.title().to_uppercase(), &self.weight(note));
        ui.capture_keyboard();

        let mut outcome = Outcome::None;
        if ui.input.key_pressed(Key::Escape) {
            return Outcome::Close;
        }
        // Enter sends. A conversation is one question at a time, so the key
        // that ends a sentence is the key that asks it; a newline inside one
        // question is a thing to want later and not today.
        if ui.input.key_pressed(Key::Enter) && !self.draft.trim().is_empty() {
            let typed = self.draft.clone();
            if self.command(&typed) {
                // A rename is worth writing down straight away: it is the kind
                // of thing you do and then close the panel.
                outcome = Outcome::Save;
            } else if !self.waiting {
                self.notice = None;
                self.commit();
                outcome = Outcome::Ask;
            }
        }

        // What a half-typed command could still become, offered above the
        // field: near what is being typed, and out of the transcript's way.
        let hints = completions(self.draft.trim_end());
        if ui.input.key_pressed(Key::Tab) {
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
        let width = body.w - Ui::SCROLL_GUTTER - 8;
        let mut state = self.scroll;
        let mut answered = None;
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
                // A rule from the end of the name to the edge, which is what
                // separates two turns without a box around each of them.
                let from = head.x + 6 + font::advance_width(who);
                ui.canvas.hline(
                    from,
                    head.y + line_h / 2,
                    head.right() - from,
                    th.well_border,
                );
                // What it said, and separately what it offered to do. The
                // blocks are lifted out so the reply reads as a sentence
                // rather than as a sentence with machinery in the middle.
                let (prose, edits) = if turn.mine {
                    (turn.text.clone(), Vec::new())
                } else {
                    proposals(&turn.text)
                };
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
                for (j, edit) in edits.iter().enumerate() {
                    if let Some(taken) = self.offer(ui, width, note, edit, (i, j)) {
                        answered = Some((edit.clone(), taken));
                    }
                }
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

        // ---- what to say next ----------------------------------------------
        let field = Rect::new(foot.x, foot.y + 2, foot.w, 15);
        let hint = if self.waiting {
            "WAITING FOR AN ANSWER"
        } else {
            "ASK SOMETHING"
        };
        let mut draft = std::mem::take(&mut self.draft);
        // `grab` also puts the caret after the text, which is what a field
        // whose contents were just completed for it needs.
        let take = self.grab || std::mem::take(&mut self.retype);
        ui.text_field_grab_at(field, "chat-field", &mut draft, hint, take);
        self.draft = draft;
        self.grab = false;

        let legend = Rect::new(foot.x, field.bottom() + 2, foot.w, line_h);
        ui.draw_text_in(
            legend,
            &format!(
                "ABOUT {} - /HELP LISTS THE COMMANDS",
                self.note.to_uppercase()
            ),
            th.ink_soft,
            Align::Left,
        );
        ui.draw_text_in(
            legend,
            "ESC LEAVES - IT IS SAVED",
            th.ink_soft,
            Align::Right,
        );
        match answered {
            Some((edit, true)) => Outcome::Apply(edit),
            Some((edit, false)) => {
                self.settled(&edit, false);
                Outcome::Save
            }
            None => outcome,
        }
    }

    /// How much context this conversation is carrying, said in tokens.
    ///
    /// Measured once there is something to measure: the worker counts the
    /// prompt on its way in and says so, and that number is exact. Before the
    /// first question there is nothing to have counted, so what is shown is an
    /// estimate off the characters, marked as one.
    fn weight(&self, note: &[String]) -> String {
        if self.progress.prompt > 0 {
            return format!("{} TOKENS", round(self.progress.prompt));
        }
        let said: usize = self.turns.iter().map(|t| t.text.len()).sum();
        let here: usize = note.iter().map(|l| l.len() + 1).sum();
        format!("~{} TOKENS", round(self.overhead + (said + here) / 4))
    }

    /// One proposed change, with what it would do to the note as it is now.
    ///
    /// The diff is against the *current* note rather than against the note the
    /// model was shown, which is the whole safety of this: if the lines have
    /// moved since it answered, what is drawn is the nonsense that would
    /// actually happen, and nobody presses accept on nonsense.
    fn offer(
        &self,
        ui: &mut Ui,
        width: i32,
        note: &[String],
        edit: &Edit,
        at: (usize, usize),
    ) -> Option<bool> {
        let th = *ui.theme;
        let line_h = font::line_h();
        ui.space(3);
        let head = ui.alloc(line_h);
        let settled = self.applied.get(&at).copied();
        let Some(before) = edit.replacing(note) else {
            ui.draw_text_in(
                head,
                &format!(
                    "A CHANGE TO LINES {}-{}, WHICH ARE NOT THERE",
                    edit.from, edit.to
                ),
                th.danger.face,
                Align::Left,
            );
            return None;
        };
        let span = if edit.from == edit.to {
            format!("LINE {}", edit.from)
        } else {
            format!("LINES {}-{}", edit.from, edit.to)
        };
        font::draw_text_styled(ui.canvas, head.x + 4, head.y, &span, th.info.hi, true);
        // Both answers, side by side and equally reachable. A change offered
        // with only a way to take it is a change you have to take.
        let mut answer = None;
        match settled {
            Some(true) => ui.draw_text_in(head, "APPLIED", th.positive.face, Align::Right),
            Some(false) => ui.draw_text_in(head, "REJECTED", th.ink_soft, Align::Right),
            None => {
                let no = Rect::new(head.right() - 42, head.y - 2, 42, 13);
                let yes = Rect::new(no.x - 46, head.y - 2, 44, 13);
                if ui.button_at(yes, "ACCEPT", pixui::Tone::Positive).clicked {
                    answer = Some(true);
                }
                if ui.button_at(no, "REJECT", pixui::Tone::Neutral).clicked {
                    answer = Some(false);
                }
            }
        }

        // The diff, drawn the way the assistant's own block draws one: this is
        // the same question - what would this do to what is there - and two
        // answers to it that looked different would be two things to learn.
        let cols = ((width - 12) / font::advance()).max(8) as usize;
        let pieces = crate::diff::words(&before, &edit.text);
        for row in crate::assist::rows(&pieces, cols) {
            let at = ui.alloc(line_h);
            crate::assist::draw_row(ui, Rect::new(at.x + 8, at.y, at.w - 8, at.h), &row);
        }
        answer
    }
}
