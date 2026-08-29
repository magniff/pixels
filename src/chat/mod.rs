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

mod change;
mod history;
mod panel;

use change::{attr, undecided};
pub use change::{elsewhere, own_name, proposals, settle, Change, Folder, What};
pub use history::{as_sent, without_bodies};
pub use panel::{doing, Picked, Picker};

use pixui::ScrollState;

use crate::llm::Turn;
use crate::markdown;

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
        name: "web",
        takes: "",
        what: "let it look things up, or stop it",
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
pub(super) const MINE: &str = "## you";

pub(super) const THEIRS: &str = "## assistant";

/// How many chats the picker shows before it scrolls.
pub(super) const ROWS: usize = 10;

/// One conversation.
pub struct Chat {
    /// Where it is filed, once it has been saved. A conversation with nothing
    /// said in it is not written to disk.
    pub path: Option<PathBuf>,
    /// The project it belongs to. A conversation is about the work, and the
    /// work is a folder of notes rather than one of them: opened from any file
    /// in a project you get the same conversations back.
    pub project: String,
    /// The file it was opened from, which is the one it is looking at. Not
    /// kept in the transcript - it follows wherever you open the chat from,
    /// which is the point of it.
    pub focus: String,
    /// What it has been called, if it has been given a name. Otherwise it is
    /// known by the first thing that was asked in it.
    pub name: Option<String>,
    pub turns: Vec<Turn>,
    /// Each turn as the panel draws it, worked out once per text rather than
    /// once per frame. See `Drawn`.
    pub(crate) drawn: Vec<Drawn>,
    /// What is being typed.
    pub draft: String,
    /// True while an answer is on its way.
    pub waiting: bool,
    /// What the worker last said about it.
    pub progress: crate::llm::Progress,
    /// The answer as far as it has got, while it is still arriving.
    pub partial: String,
    /// Why the last question failed, if it did.
    pub failed: Option<String>,
    /// Something worth saying that is not a failure - what a command did.
    pub notice: Option<String>,
    /// True on the frame it opens, so the field takes the keyboard.
    grab: bool,
    /// Whether there was a field to type in on the last frame.
    ///
    /// While a question is being answered, and while a change is waiting to be
    /// accepted, there is no field - and the toolkit hands the keyboard back
    /// when a field it was pointing at stops existing, which is right. What was
    /// missing is the other half: taking it again when the field returns.
    /// Without it, every answered question left the conversation unable to be
    /// typed into until it was clicked.
    had_field: bool,
    /// True on the frame a completion rewrote the draft, so the field picks up
    /// the new text and puts the caret after it.
    retype: bool,
    /// Set by `/web`, and spent by the application on the same frame.
    flip_web: bool,
    scroll: ScrollState,
    /// True when the view should be pinned to the newest turn.
    follow: bool,
    /// Roughly how many tokens the context around the conversation comes to -
    /// the vault list and the note. Told to it once by the application, which
    /// is the thing that assembles them, rather than worked out every frame.
    pub overhead: usize,
    /// The one-line-per-note index as the model was last shown it.
    ///
    /// Held with the project and for the same reason. It is derived from what
    /// the notes say - a title, a first line, the headings - so editing a note
    /// moves it, and it sits in front of the project, so moving it threw the
    /// project away too. The delta below kept the project still while the line
    /// above it shifted, and the reading happened anyway.
    shown_index: String,
    /// The index as the model was last *told* it, wherever it was told.
    ///
    /// The same pairing as `front` and `known` below, and needed for the same
    /// reason. `shown_index` is what sits at the top of the conversation and
    /// must not move. This is what the model has actually been told, whether
    /// at the top or in a correction since - and without it a list that had
    /// been corrected once was corrected again every turn afterwards, because
    /// the thing it was being compared against was the copy at the front that
    /// is never going to change.
    known_index: String,
    /// The project as the model was last shown it, whole.
    ///
    /// Not saved with the conversation, and deliberately: a chat opened
    /// tomorrow has shown the model nothing, so it is written out afresh. That
    /// is also what makes a file edited in another window - or in this one,
    /// with the panel closed - come out right, because what is compared
    /// against is what was *sent*, not what the notes were doing at the time.
    front: Vec<(String, String)>,
    /// Everything the model has been told about the project, however it was
    /// told: written out at the front, or corrected at the end afterwards.
    ///
    /// Kept apart from `front` because they answer different questions. The
    /// front is what must not move, or the reading starts again. This is what
    /// the model actually knows, and what a correction has to be measured
    /// against - otherwise a file first mentioned in a correction is new every
    /// time, and a one-line change to a long note re-sends the whole note for
    /// the rest of the conversation.
    known: Vec<(String, String)>,
    /// What the model's own edits did, waiting to be said with the next
    /// question and then let go of. See `did`.
    done: Vec<String>,
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
    /// Give up on the answer that is on its way.
    Stop,
    /// Turn looking things up on, or off.
    Web,
    /// Something changed that should be written down.
    Save,
    /// Put this change into the project.
    Apply(Change),
    /// Take it away.
    Close,
}

/// A turn as the panel draws it: the prose, what was looked up, what was
/// offered.
///
/// Reading a turn is not free - the blocks are found, drafts collapsed, a
/// write and its deletes folded into a merge - and the panel was doing all of
/// it for every turn on every frame, sixty times a second, for as long as a
/// conversation was open. Once per text is enough: the key is a hash of the
/// text, and a turn whose text has not changed is drawn from what was worked
/// out last time.
#[derive(Clone, Debug, Default)]
pub(crate) struct Drawn {
    pub(crate) key: u64,
    pub(crate) prose: String,
    pub(crate) looked: Vec<Lookup>,
    pub(crate) edits: Vec<Change>,
}

impl Drawn {
    /// The hash a turn's text is known by.
    pub(crate) fn key_of(turn: &Turn) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        turn.text.hash(&mut h);
        h.finish()
    }

    fn of(turn: &Turn) -> Self {
        let key = Self::key_of(turn);
        if turn.mine {
            return Self {
                key,
                prose: turn.text.clone(),
                ..Self::default()
            };
        }
        let (said, looked) = lookups(&turn.text);
        let (prose, edits) = proposals(&said);
        Self {
            key,
            prose,
            looked,
            edits,
        }
    }
}

impl Chat {
    /// Bring the drawn turns up to date with the turns, touching only the
    /// ones whose text has changed since they were last drawn.
    pub(crate) fn redraw_turns(&mut self) {
        self.drawn.truncate(self.turns.len());
        for (i, turn) in self.turns.iter().enumerate() {
            let key = Drawn::key_of(turn);
            match self.drawn.get(i) {
                Some(d) if d.key == key => {}
                Some(_) => self.drawn[i] = Drawn::of(turn),
                None => self.drawn.push(Drawn::of(turn)),
            }
        }
    }
}

/// Something the conversation looked up, and what came back.
#[derive(Clone, Debug)]
pub struct Lookup {
    pub tool: String,
    pub arg: String,
    pub result: String,
}

/// A tool's argument, at a length that fits on a line.
pub(super) fn shortened(arg: &str) -> String {
    const ROOM: usize = 48;
    let flat = arg.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(ROOM) {
        None => flat,
        Some((at, _)) => format!("{}...", flat[..at].trim_end()),
    }
}

impl Lookup {
    /// What it did, in words.
    ///
    /// `date` and `today` are what the machinery calls it, and printing them
    /// puts the wiring in front of somebody who asked what time it was. The
    /// call is worth showing - an answer that came from somewhere should say
    /// so - but showing it is not the same as showing the arguments it was
    /// made with.
    pub fn said(&self) -> String {
        // Cut, because an argument can be enormous. Asked how many days
        // somebody had been alive, one model reached for the calculator with
        // four hundred ones added together, and the whole of it would have
        // been drawn across the panel as a single line.
        let arg = &shortened(self.arg.trim());
        let arg: &str = arg;
        let now =
            arg.is_empty() || arg.eq_ignore_ascii_case("today") || arg.eq_ignore_ascii_case("now");
        match self.tool.as_str() {
            "date" if now => "CHECKED THE DATE AND TIME".to_string(),
            "date" => format!("CHECKED WHEN {arg} IS"),
            "calc" => format!("WORKED OUT {arg}"),
            "weather" => format!("CHECKED THE WEATHER IN {arg}"),
            "wikipedia" => format!("LOOKED UP {arg}"),
            "release" => format!("CHECKED THE LATEST RELEASE OF {arg}"),
            // The address rather than the whole query string, which is longer
            // than the row and says less.
            "fetch" => format!(
                "READ {}",
                arg.trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or(arg)
            ),
            // Something added since this was written. Better a plain sentence
            // about a tool this does not know than nothing at all.
            other => format!("USED {other} ON {arg}"),
        }
        .to_uppercase()
    }
}

/// What one turn comes to on the clipboard.
///
/// What is on the screen, not what is on disk. A reply carries the record of
/// what it looked up, and that is drawn as a sentence rather than as the block
/// it is written in - so pasting the block somewhere else would hand over
/// wiring nobody asked for. What a change it offered says is kept, because
/// that is content: it is the lines it wants to put in the file.
pub fn copyable(turn: &Turn) -> String {
    if turn.mine {
        return turn.text.trim().to_string();
    }
    lookups(&turn.text).0.trim().to_string()
}

/// Lift the record of what was looked up out of a reply.
///
/// Kept in the transcript rather than reported and forgotten: an answer that
/// came from somewhere should still say where tomorrow, and one that came from
/// nowhere should still be telling you that.
pub fn lookups(reply: &str) -> (String, Vec<Lookup>) {
    let mut prose = String::new();
    let mut used = Vec::new();
    let mut rest = reply;
    while let Some(at) = rest.find("<used ") {
        let Some(open) = rest[at..].find('>').map(|i| at + i + 1) else {
            break;
        };
        let Some(close) = rest[open..].find("</used>").map(|i| open + i) else {
            break;
        };
        let tag = &rest[at..open];
        prose.push_str(&rest[..at]);
        used.push(Lookup {
            tool: attr(tag, "tool").unwrap_or_default(),
            arg: attr(tag, "arg").unwrap_or_default(),
            result: rest[open..close].trim().to_string(),
        });
        rest = &rest[close + 7..];
    }
    prose.push_str(rest);
    (prose.trim().to_string(), used)
}

impl Chat {
    /// A new conversation in `project`, looking at `focus`.
    pub fn new(project: String, focus: String) -> Self {
        Self {
            path: None,
            project,
            focus,
            name: None,
            turns: Vec::new(),
            drawn: Vec::new(),
            draft: String::new(),
            waiting: false,
            progress: crate::llm::Progress::default(),
            partial: String::new(),
            failed: None,
            notice: None,
            grab: true,
            had_field: false,
            retype: false,
            flip_web: false,
            scroll: ScrollState::default(),
            follow: true,
            overhead: 0,
            shown_index: String::new(),
            known_index: String::new(),
            front: Vec::new(),
            known: Vec::new(),
            done: Vec::new(),
        }
    }

    /// A conversation read back off disk.
    pub fn open(path: &Path, project: String, focus: String) -> Self {
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
            drawn: Vec::new(),
            name,
            ..Self::new(project, focus)
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

    /// Take note that a file now says this, because we just made it say it.
    ///
    /// A change the user accepted is not news to be broken to the model: it
    /// proposed it, it was told the answer, and the block it wrote is still in
    /// the conversation with what it said in it. Without this the next question
    /// carried a correction saying the file had changed on disk since anything
    /// the model had been told - which was untrue, and was the whole file.
    ///
    /// What comes after this is measured against it, so an edit made in another
    /// window still arrives, and arrives as the lines that moved.
    pub fn wrote(&mut self, file: &str, text: Option<&str>) {
        self.known.retain(|(n, _)| n != file);
        if let Some(text) = text {
            self.known.push((file.to_string(), text.to_string()));
        }
    }

    /// What the model has been told a file says, if it has been told.
    pub fn knows(&self, file: &str) -> Option<&str> {
        self.known
            .iter()
            .find(|(n, _)| n == file)
            .map(|(_, t)| t.as_str())
    }

    /// Say, once, what an edit of the model's actually did to a file.
    ///
    /// An edit is line numbers, and if the numbers were wrong the file is not
    /// what the model meant - and it is the only one who cannot tell. So the
    /// lines that changed are shown to it with the next question, as its own
    /// doing rather than as news from outside. Once: after that the file is
    /// known as it is, so that somebody undoing the change by hand is seen as
    /// a change, which for a while it was not - the file was still known as
    /// it was before the edit, and going back to that looked like nothing.
    ///
    /// A file that is not at the front - one the model made in this
    /// conversation - is shown whole instead, with its lines numbered. The
    /// only copy it has of such a file is its own write, which has no numbers
    /// in the margin, and a diff on top of that is a sum to do: having put a
    /// row below line 4 and another below line 7, it changed line 6 to correct
    /// the hotel, which had been line 6 when it wrote the file and was line 7
    /// now. The flights went instead. A file it made is small and new, and the
    /// whole of it, numbered, costs less than one wrong line.
    pub fn did(&mut self, file: &str, before: &str, after: &str) {
        if before == after {
            return;
        }
        let at_front = self.front.iter().any(|(n, _)| n == file);
        let what = if at_front {
            crate::digest::changed(before, after)
        } else {
            format!(
                "It now says, in full:\n\n{}",
                crate::digest::numbered(after)
            )
        };
        self.done
            .push(format!("Your edit to `{file}` was applied. {what}"));
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
            // Answered by the application, which owns the settings: this only
            // says that it was asked.
            "web" => self.flip_web = true,
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
                // Put right here, once, rather than wherever a block is read.
                // What is stored is what the rest of the application works
                // from: the panel reads the blocks out of it, and answering
                // one writes the decision back into the same string by the
                // same offsets. Mending it on the way in keeps those two
                // looking at the same text - mending it on the way out meant
                // a change could be offered and then never settle, because
                // the decision was written against a block the store did not
                // have.
                self.turns.push(Turn {
                    mine: false,
                    text: undecided(crate::llm::unfused(text.trim()).trim())
                        .trim()
                        .to_string(),
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

    /// The project to send, and what has moved since it was last sent.
    ///
    /// The first is byte for byte the same text every time until it is worth
    /// rewriting, so everything before the newest question stays in the
    /// model's cache. The second says what changed, and goes at the end.
    ///
    /// Rewritten rather than corrected when correcting stops being the cheaper
    /// of the two, which is a thing that can be worked out rather than guessed
    /// at. Correcting costs the length of the correction, because everything
    /// before it is still in the model's cache. Rewriting costs the length of
    /// the whole project, because changing the front of it empties that cache.
    /// So correct while the correction is the smaller number, and when a
    /// project has been told it is wrong in so many places that the list is
    /// longer than the project, show it the project.
    pub fn context(
        &mut self,
        index: &str,
        now: &[(String, String)],
    ) -> (String, String, Option<String>) {
        // Written out again, and still told at the end what moved.
        //
        // The rewrite is about what it costs to read; the note at the end is
        // about what gets noticed. A file rewritten at the front sits before
        // every turn of the conversation, and a conversation that has been
        // saying "the bike is red" for six turns drowns it: shown a project
        // that plainly said green, the model went on answering red, because
        // the last thing said about the bike was its own. What is put in front
        // of the question is not.
        let afresh = |chat: &mut Self, moved: Option<String>| {
            chat.shown_index = index.to_string();
            chat.known_index = index.to_string();
            chat.front = now.to_vec();
            chat.known = now.to_vec();
            (index.to_string(), crate::digest::project(now), moved)
        };
        if self.front.is_empty() {
            return afresh(self, None);
        }
        // Against what it has been told, not against what is at the front: a
        // note first mentioned in a correction is not new the second time.
        //
        // With one exception. A file that is not at the front - one the model
        // made mid-conversation - is a file it has only ever seen in pieces:
        // its own write, then its own edits, then whatever moved since. Asked
        // to sort a list it had built that way after somebody else changed
        // two lines of it, it sorted from memory: the old price of the milk,
        // and the cheese that had been added left out. When such a file moves,
        // it is shown whole, which is what a diff against the front would
        // have been anyway had the file been there.
        let shown: Vec<(String, String)> = self
            .known
            .iter()
            .filter(|(name, text)| {
                self.front.iter().any(|(f, _)| f == name)
                    || now.iter().any(|(n, t)| n == name && t == text)
            })
            .cloned()
            .collect();
        let moved = crate::digest::since(&shown, now);
        // What its own edits did, said once, in front of anything that moved
        // since - it is older news, and the newer is the one to act on.
        let done = std::mem::take(&mut self.done);
        let moved = match (done.is_empty(), moved) {
            (true, moved) => moved,
            (false, Some(moved)) => Some(format!("{}\n\n{moved}", done.join("\n\n"))),
            (false, None) => Some(done.join("\n\n")),
        };
        let listed = crate::digest::relisted(&self.known_index, index);
        // Both are corrections, and both are paid for the same way, so both go
        // through the same choice. What must not happen is the front being
        // rewritten and the correction being sent as well: the rewrite empties
        // the cache and the correction then says the front is wrong when it is
        // not. A list that has moved on its own is worth a few hundred
        // characters at the end; it is never worth the whole project again.
        let both = match (moved, listed) {
            (None, None) => {
                return (
                    self.shown_index.clone(),
                    crate::digest::project(&self.front),
                    None,
                );
            }
            (Some(moved), Some(listed)) => format!("{moved}\n\n{listed}"),
            (Some(moved), None) => moved,
            (None, Some(listed)) => listed,
        };
        if both.len() >= crate::digest::project(now).len() + index.len() {
            return afresh(self, Some(both));
        }
        // Told, so next time only what moves after this has to be said.
        self.known = now.to_vec();
        self.known_index = index.to_string();
        (
            self.shown_index.clone(),
            crate::digest::project(&self.front),
            Some(both),
        )
    }

    /// Write it out, choosing a name the first time.
    /// The whole conversation, in the shape somebody would want it pasted.
    ///
    /// The same headings the file on disk uses, so a conversation copied into
    /// a note and one saved beside it read alike - and the same turns the
    /// panel draws, so it holds no machinery.
    pub fn transcript(&self) -> String {
        let mut out = format!("# {}\n\n", self.title());
        for turn in &self.turns {
            out.push_str(if turn.mine { MINE } else { THEIRS });
            out.push_str("\n\n");
            out.push_str(&fence_markers(&copyable(turn)));
            out.push_str("\n\n");
        }
        out.trim_end().to_string()
    }

    pub fn save(&mut self, dir: &Path) -> std::io::Result<()> {
        if self.turns.is_empty() {
            return Ok(());
        }
        let home = folder(dir, &self.project);
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

/// Every conversation filed under a project, newest first.
pub fn filed(dir: &Path, project: &str) -> Vec<Filed> {
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(folder(dir, project)) else {
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

/// Where a project's conversations live.
///
/// Under the vault rather than beside it, and in a dot directory: the vault is
/// read as the `.md` files directly inside it and the folders beside them, so
/// everything in here is invisible to it without the loader having to know
/// this exists. The loose notes at the top of the vault keep their
/// conversations at the top of this, which is the same arrangement one level
/// down.
pub fn folder(dir: &Path, project: &str) -> PathBuf {
    let home = dir.join(".chats");
    if project.is_empty() {
        home
    } else {
        home.join(project)
    }
}

/// What to call a project out loud, including the one with no name.
pub fn called(project: &str) -> String {
    if project.is_empty() {
        "THE VAULT".to_string()
    } else {
        project.to_uppercase()
    }
}

/// A file name nothing else has taken.
pub(super) fn free_name(home: &Path, title: &str) -> PathBuf {
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
pub(super) fn fence_markers(text: &str) -> String {
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
pub(super) fn one_line(text: &str, room: usize) -> String {
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
pub(super) fn round(n: usize) -> String {
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
