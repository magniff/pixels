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
const MINE: &str = "## you";
const THEIRS: &str = "## assistant";

/// How many chats the picker shows before it scrolls.
const ROWS: usize = 10;

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

/// Something the conversation looked up, and what came back.
pub struct Lookup {
    pub tool: String,
    pub arg: String,
    pub result: String,
}

/// A tool's argument, at a length that fits on a line.
fn shortened(arg: &str) -> String {
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

/// The files a conversation is about, as they are now.
///
/// Borrowed for the frame rather than copied: a project is every note in a
/// folder, and copying all of them to draw one diff would be a strange price.
pub struct Folder<'a> {
    /// The project these files are, by its folder name - empty for the notes
    /// that sit loose at the top of the vault.
    pub project: String,
    /// The note in front of you, which is what an unqualified change means.
    pub here: String,
    pub files: Vec<(String, &'a [String])>,
}

/// The project a name points at, when it is not this one.
///
/// A change may name a file with its folder in front - the list of notes shows
/// every note that way, so it is the shape a model has in front of it. When
/// the folder is the project being looked at, or there is no folder, the name
/// means a file here. When it is some other project, the change is for a file
/// this conversation cannot reach: only the project on screen can be changed,
/// and the way to change another is to open it. Reading is a different matter
/// and the read tool reaches the whole vault.
///
/// This used to fall through. The folder was dropped, the bare name matched
/// nothing here, and - because a bare name that matches nothing means the note
/// in front of you - an edit meant for `aquarium/stock.md` was offered against
/// whatever was open, line for line.
pub fn elsewhere(named: &str, project: &str) -> Option<String> {
    let named = named.trim().trim_start_matches(['/', '\\']);
    let (folder, _) = named.rsplit_once(['/', '\\'])?;
    let folder = folder.trim().trim_end_matches(['/', '\\']);
    (!folder.is_empty() && folder != project).then(|| folder.to_string())
}

impl Folder<'_> {
    /// The file a change is about, if it is there at all.
    ///
    /// By its own name, with any folder in front of it dropped. Models write
    /// the folder in - the list of notes shows every note with one, so it is
    /// the shape they have in front of them - and they write the wrong one:
    /// asked for a note while reading a project, one wrote
    /// `typography/bikes.md`, which is a real folder and not that one.
    ///
    /// The application has always dropped it before applying a change. This
    /// did not, so the panel looked for a file under a name nothing is filed
    /// under, found nothing, and said the change was not there to make - about
    /// a file that was there, and a change that would have applied cleanly.
    /// Two places deciding what a name means, and only one of them right.
    pub fn lines(&self, named: Option<&String>) -> Option<&[String]> {
        let want = named
            .map(|n| own_name(n))
            .unwrap_or_else(|| self.here.clone());
        self.files
            .iter()
            .find(|(name, _)| own_name(name) == want)
            .map(|(_, lines)| *lines)
    }
}

/// A file's own name, without whatever folder was written in front of it.
pub fn own_name(named: &str) -> String {
    named
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(named)
        .trim()
        .to_string()
}

/// Something the model has offered to do to the project.
///
/// Line numbers rather than text to find: the files are shown numbered, and a
/// number cannot be misquoted. Both are one-based and inclusive, the way they
/// are written in the margin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Which file, by name within the project. None means the note in front of
    /// you, which is what an unqualified change is about.
    pub file: Option<String>,
    pub what: What,
    /// What was decided about it, once something was. Kept in the transcript
    /// rather than in memory: a conversation reopened tomorrow should not offer
    /// again a change you took this morning, and the only place tomorrow can
    /// learn that is the file.
    pub state: Option<bool>,
}

/// The three things it can offer to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    /// Replace these lines with this text. Empty text takes them out.
    Edit {
        from: usize,
        to: usize,
        text: String,
    },
    /// Put this text in below this line, moving nothing. Zero is the top.
    ///
    /// Adding a line used to be an edit: replace the line it goes after with
    /// that line and the new one. Which is the instruction models get wrong
    /// most - asked to add eggs to a list, one rewrote the tail from the wrong
    /// line and had the milk twice; told to edit the line it goes after,
    /// another edited it and left the old line out. Both are the same
    /// difficulty: saying "add" as "replace with more". This says add.
    Insert { after: usize, text: String },
    /// What the file should contain from now on, whether or not it is there
    /// yet. One verb rather than create-and-append-and-replace: "here is what
    /// this file says now" covers all three, and a model that has been handed
    /// the whole file reaches for it naturally.
    Write { text: String },
    /// A file that should not be there any more.
    Delete,
    /// Several files folded into one, and the ones folded in taken away.
    ///
    /// Three blocks would do this - write the one, delete the others - but
    /// they would be accepted one at a time, and a merge half accepted is a
    /// note duplicated and a note lost. It is one thing, so it is one answer.
    Merge {
        /// What is being folded in. The target may be one of them, and then it
        /// is written rather than removed.
        from: Vec<String>,
        /// What the result should say. Empty means the parts, end to end, in
        /// the order they were named.
        text: String,
    },
}

impl Change {
    /// The other project this change reaches for, if it does.
    ///
    /// Every name it carries is looked at, because a merge names several and
    /// one of them pointing out of the project is enough to make it a change
    /// this conversation cannot make.
    pub fn misplaced(&self, folder: &Folder) -> Option<String> {
        let mut names: Vec<&String> = self.file.iter().collect();
        if let What::Merge { from, .. } = &self.what {
            names.extend(from.iter());
        }
        names
            .into_iter()
            .find_map(|name| elsewhere(name, &folder.project))
    }

    /// Lines gone and lines arrived, the way a diff counts them.
    pub fn tally(&self, folder: &Folder) -> (usize, usize) {
        let count = |t: &str| if t.is_empty() { 0 } else { t.lines().count() };
        let target = folder
            .lines(self.file.as_ref())
            .map(|l| l.len())
            .unwrap_or(0);
        match &self.what {
            What::Edit { from, to, text } => (count(text), to.saturating_sub(*from) + 1),
            What::Insert { text, .. } => (count(text), 0),
            What::Write { text } => (count(text), target),
            What::Delete => (0, target),
            // Everything folded in goes away as well as the target being
            // rewritten, so the count says what the project loses, not what
            // one file does.
            What::Merge { from, .. } => {
                let gone: usize = from
                    .iter()
                    .filter(|name| Some(*name) != self.file.as_ref())
                    .filter_map(|name| folder.lines(Some(name)))
                    .map(|l| l.len())
                    .sum();
                (count(&self.becoming(folder)), target + gone)
            }
        }
    }

    /// What this would replace, given the file it is about as it is now.
    ///
    /// None when there is nothing there to replace - lines past the end, or a
    /// file that is not there - so the panel can say so instead of guessing.
    pub fn replacing(&self, folder: &Folder) -> Option<String> {
        if self.misplaced(folder).is_some() {
            return None;
        }
        let lines = folder.lines(self.file.as_ref());
        match &self.what {
            // Below a line that is there, or at the top, and nothing is
            // replaced. Where the file is looked for is as for an edit.
            What::Insert { after, .. } => {
                let lines = lines?;
                (*after <= lines.len()).then(String::new)
            }
            What::Edit { from, to, .. } => {
                // A name that matches nothing is a file that is not there,
                // and the change is refused - not offered against the note in
                // front of you, which it was for a while. That was for a model
                // copying `notes.md` out of the example; the example names the
                // open note now, so there is nothing to copy wrong. And it was
                // dangerous: asked to make bike.md, a model wrote an edit to
                // line 1 of a bike.md that did not exist, and line 1 of
                // whatever was open became "RED".
                let lines = lines?;
                let first = from.checked_sub(1)?;
                // One past the end is the line that is not there yet, and an
                // edit to it means "after the last one". The instructions say
                // to add by editing the line it goes after; a model asked to
                // add bread to a three-line list wrote lines="4-4" instead,
                // which is what anybody would write, and was refused for it.
                if first > lines.len() || to < from {
                    return None;
                }
                Some(lines[first..(*to).min(lines.len())].join("\n"))
            }
            // A file being written replaces whatever it said before, which is
            // nothing at all when it is not there yet.
            What::Write { .. } => Some(lines.map(|l| l.join("\n")).unwrap_or_default()),
            What::Delete => Some(lines?.join("\n")),
            // A merge that names a file which is not there has nothing to fold
            // in, and is a mistake rather than a change.
            What::Merge { from, .. } => from
                .iter()
                .all(|name| folder.lines(Some(name)).is_some())
                .then(|| lines.map(|l| l.join("\n")).unwrap_or_default()),
        }
    }

    /// What it would leave behind in place of that.
    pub fn becoming(&self, folder: &Folder) -> String {
        match &self.what {
            What::Edit { text, .. } | What::Insert { text, .. } | What::Write { text } => {
                text.clone()
            }
            What::Delete => String::new(),
            What::Merge { from, text } if text.is_empty() => from
                .iter()
                .filter_map(|name| folder.lines(Some(name)))
                .map(|l| l.join("\n"))
                .collect::<Vec<_>>()
                .join("\n\n"),
            What::Merge { text, .. } => text.clone(),
        }
    }

    /// How to say what it is, in a few words.
    pub fn headline(&self, whose: &str) -> String {
        let named = self.file.clone().unwrap_or_else(|| whose.to_string());
        match &self.what {
            What::Edit { from, to, .. } if from == to => format!("{named}  LINE {from}"),
            What::Edit { from, to, .. } => format!("{named}  LINES {from}-{to}"),
            What::Insert { after: 0, .. } => format!("{named}  AT THE TOP"),
            What::Insert { after, .. } => format!("{named}  AFTER LINE {after}"),
            What::Write { .. } => format!("WRITE  {named}"),
            What::Delete => format!("DELETE  {named}"),
            What::Merge { from, .. } => format!("MERGE  {}  INTO  {named}", from.join(", ")),
        }
    }
}

/// Take back any decision the model wrote into its own change.
///
/// Whether a change was accepted is recorded in the block itself, as
/// `state="applied"`, which is how a conversation still shows tomorrow what
/// was done with it. The model sees those blocks in its own history, and
/// copies the shape: asked to write a file, Qwen3.5 produced
/// `<write file="facts.md" state="applied">` - already decided, by the one
/// party that does not get a say.
///
/// The application believed it. The change was not pending, so no buttons were
/// offered; it was not applied either, because nobody had applied it. The file
/// was silently not written, and the conversation looked like it had been.
///
/// So a decision only ever gets into the text from this side of it.
fn undecided(reply: &str) -> String {
    let mut out = String::new();
    let mut at = 0;
    for (_, tag, open, _) in blocks(reply) {
        out.push_str(&reply[at..tag]);
        let bare = strip_state(&reply[tag..open]);
        let bare = bare.trim_end().trim_end_matches('>').trim_end();
        out.push_str(bare);
        out.push('>');
        at = open;
    }
    out.push_str(&reply[at..]);
    out
}

/// A past turn as the model should see it, with the bodies of changes taken
/// out of it.
///
/// A change block is a copy of a file, and a copy of a file goes stale. Left
/// in the conversation it is worse than stale: it is a copy the model wrote
/// itself, so when the file later says something else, the model has its own
/// word against a correction, and takes its own. Reported exactly that way -
/// "if the model set the text it won't accept the change of it at all, if the
/// text was there already the change is accepted just fine", and a conversation
/// started fresh gets it right, having nothing of its own to disagree with.
///
/// So what it proposed is still there - it should know what it did - but the
/// text of it is not, because the text of it is in the project, once, and
/// current. The stored transcript keeps the whole thing: this is only what is
/// sent, and the panel still draws the diff.
/// What the conversation looked up, said rather than tagged.
///
/// Two reasons, and the second one is the reason.
///
/// A note read comes back whole, with its lines numbered, and it stayed in the
/// conversation once it was there - a note read on the first question still
/// being sent on the tenth, in the state it was in on the first. The largest
/// thing in the prompt, and a copy of a file that has had ten turns to change,
/// arguing with the current one. The fact that it looked survives; what it saw
/// does not, because it can look again.
///
/// And a lookup written as `<used tool="date" arg="...">` is a shape, and a
/// shape in an assistant's own turn is a shape an assistant writes. One did:
/// asked how old somebody was, it wrote the block itself and filled it in -
/// Tuesday for a Monday, five hundred and ninety-five days for six hundred and
/// thirteen, and a span written "1 year, 8 months, and 5 days" where this
/// application writes "1 year and 8 months". Nothing was asked and nothing
/// answered. It had simply learnt, from its own transcript, that this is
/// something it may write, and what it writes it may invent.
///
/// So none of it goes back as a tag. The answers still do - a sum and a date
/// are a line each and cannot go stale - but written as something that was
/// told to it, which is what it was.
fn without_lookups(text: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut notes = Vec::new();
    let mut at = 0usize;
    const SHUT: &str = "</used>";
    while let Some(i) = text[at..].find("<used tool=") {
        let from = at + i;
        let Some(end) = text[from..].find(SHUT).map(|j| from + j + SHUT.len()) else {
            break;
        };
        let span = &text[from..end];
        let mut quoted = span.split('"');
        let tool = quoted.nth(1).unwrap_or("something").to_string();
        let arg = quoted.nth(1).unwrap_or("").to_string();
        let body = span
            .split_once('>')
            .map(|(_, rest)| rest.trim_end_matches(SHUT).trim())
            .unwrap_or("");
        out.push_str(&text[at..from]);
        notes.push(if tool == "read" {
            format!("You read `{arg}` at that point.")
        } else {
            format!("The {tool} tool was asked about {arg}, and answered: {body}")
        });
        at = end;
    }
    out.push_str(&text[at..]);
    (out, notes)
}

pub fn without_bodies(text: &str) -> String {
    let (said, notes) = bodies_but(text, &[], &[]);
    // On its own this is one turn with nowhere to carry to, so what would have
    // gone into the next question is put on the end of this one. `as_sent` is
    // what the conversation actually goes through, and it has somewhere.
    if notes.is_empty() {
        said
    } else {
        format!("{said}\n\n{}", notes.join("\n")).trim().to_string()
    }
}

/// The conversation as the model should be shown it.
///
/// Every change block becomes a label, except the newest accepted one for each
/// file, which keeps what it said. That body is not a duplicate of anything -
/// it is the only copy of what the file holds, because a file the model has
/// just made is not in the project written out at the top of the conversation,
/// which was written before it existed.
///
/// Stripping it and then sending the same text back at the end as a file that
/// had "changed on disk since anything you have been told" was two wrongs: it
/// cost the text twice over the two turns it took to do it, and it announced a
/// change nobody had made, in the strongest words this application has, about
/// a file the model had written itself and been told was accepted.
pub fn as_sent(turns: &[Turn], now: &[(String, String)]) -> Vec<String> {
    let mut newest: Vec<(String, usize, usize)> = Vec::new();
    // Blocks that were accepted and whose text the file no longer holds.
    //
    // A conversation opened again is shown the project afresh, so the front
    // said the door was blue - and the model's own accepted edit, still in
    // the history with its body, said green. Asked, it said green. It trusts
    // its own words over the page; that is the whole reason a superseded
    // block loses its body, and a block the file has moved on from is
    // superseded by the file. It keeps its body only while the file still
    // says what it says; after that it is a note that the file has changed
    // since, which is the one thing it needs telling.
    let mut stale: Vec<(usize, usize)> = Vec::new();
    for (t, turn) in turns.iter().enumerate() {
        if turn.mine {
            continue;
        }
        for (b, (kind, tag, open, close)) in blocks(&turn.text).into_iter().enumerate() {
            if kind == "delete" {
                continue;
            }
            let head = &turn.text[tag..open];
            let named = attr(head, "into").or_else(|| attr(head, "file"));
            if let (Some(named), Some(true)) = (named, state_attr(head)) {
                let body = turn.text[open..close].trim_matches('\n');
                let holds = now
                    .iter()
                    .find(|(n, _)| own_name(n) == own_name(&named))
                    .map(|(_, text)| match kind {
                        "write" | "create" | "merge" => {
                            text.trim_end_matches('\n') == body.trim_end_matches('\n')
                                || (body.trim().is_empty() && kind == "merge")
                        }
                        _ => body.trim().is_empty() || text.contains(body.trim()),
                    });
                if holds == Some(false) {
                    stale.push((t, b));
                    continue;
                }
                newest.retain(|(n, _, _)| *n != named);
                newest.push((named, t, b));
            }
        }
    }
    // Whatever a turn of theirs cannot keep travels forward and is said in the
    // next turn of ours. Which is where it belongs and, more to the point, is
    // somewhere the model does not write.
    //
    // It used to be said in their own turn - a bracket where the block or the
    // lookup had been. A bracket in an assistant's turn is a shape an
    // assistant writes, and this one wrote it: asked four times over to put a
    // name back, it answered "[you read `family.md` here]" and "[edit to
    // `family.md`: accepted]" and did nothing at all, four times, because
    // those were the words that went in that place. Nothing was read and
    // nothing was edited. The same fault as the tool tag it had been forging
    // the hour before, in the shape that replaced it.
    let mut out: Vec<String> = Vec::with_capacity(turns.len());
    let mut carry: Vec<String> = Vec::new();
    for (t, turn) in turns.iter().enumerate() {
        if turn.mine {
            let mut text = String::new();
            if !carry.is_empty() {
                text.push_str(&carry.join("\n"));
                text.push_str("\n\n");
                carry.clear();
            }
            text.push_str(&turn.text);
            out.push(text);
            continue;
        }
        let keep: Vec<usize> = newest
            .iter()
            .filter(|(_, at, _)| *at == t)
            .map(|(_, _, b)| *b)
            .collect();
        let moved_on: Vec<usize> = stale
            .iter()
            .filter(|(at, _)| *at == t)
            .map(|(_, b)| *b)
            .collect();
        let (said, notes) = bodies_but(&turn.text, &keep, &moved_on);
        carry.extend(notes);
        out.push(said);
    }
    // Nowhere left to put them, which happens only when the last word was
    // theirs. Then they go on the end of it, and there is no next question for
    // them to be copied into.
    if let (Some(last), false) = (out.last_mut(), carry.is_empty()) {
        last.push_str("\n\n");
        last.push_str(&carry.join("\n"));
    }
    out
}

/// Every block replaced by a label, save the ones named by position.
fn bodies_but(text: &str, keep: &[usize], moved_on: &[usize]) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut notes = Vec::new();
    let mut at = 0;
    for (nth, (kind, tag, open, close)) in blocks(text).into_iter().enumerate() {
        let head = &text[tag..open];
        let named = attr(head, "into")
            .or_else(|| attr(head, "file"))
            .unwrap_or_else(|| "the note".into());
        let done = match state_attr(head) {
            // Only that it was taken, not what the file says now. A block
            // that has lost its body is one a later block for the same file
            // has superseded, and three notes in a row each saying the file
            // was "as it left it" were three contradictions in front of a
            // model about to rewrite that file - which it then did from a
            // picture that matched none of them.
            Some(true) if moved_on.contains(&nth) => {
                "accepted, but the file has been changed by hand since and no longer says that"
            }
            Some(true) => "accepted",
            Some(false) => "turned down, and the file is as it was",
            None => "not answered either way yet",
        };
        out.push_str(&text[at..tag]);
        if keep.contains(&nth) {
            out.push_str(&text[tag..block_end(text, kind, close)]);
            at = block_end(text, kind, close);
            continue;
        }
        // Taken out of their turn and said in ours. What it said is gone
        // because a newer block for that file has it; what is left to say is
        // that it was proposed and how it went, and that is our news, not
        // theirs.
        notes.push(format!("Your {kind} to `{named}` was {done}."));
        at = block_end(text, kind, close);
    }
    out.push_str(&text[at..]);
    // Blocks first and lookups after, in that order and not the other way. A
    // tool's answer may quote the shape of a change - the one telling a model
    // that a write is not something to call quotes it on purpose - and the
    // scan for blocks knows to leave a quoted answer alone. Unwrap the answer
    // first and that protection is gone: the quote is loose in the turn, and
    // it comes back as a change nobody proposed.
    let (said, looked) = without_lookups(&out);
    notes.extend(looked);
    (said.trim().to_string(), notes)
}

/// Split a reply into what it said and what it proposed.
///
/// The blocks are lifted out of the prose rather than left in it: a reply is
/// read as a sentence and a change, and showing the raw block would be showing
/// somebody the machinery instead of the change.
pub fn proposals(reply: &str) -> (String, Vec<Change>) {
    let (prose, changes) = every_proposal(reply);
    // A delete of a file a merge in the same reply folds in is the merge's
    // own work said twice, and dangerous: accepted first, the file is gone
    // before the merge can fold it, and the merge then has nothing to fold -
    // which is the note lost that the verb exists to prevent. A model wrote
    // exactly that, a merge and two deletes, and the week's note was never
    // made while both days were.
    let folded: Vec<String> = changes
        .iter()
        .filter_map(|c| match &c.what {
            What::Merge { from, .. } => Some(from.iter().map(|f| own_name(f)).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect();
    let changes: Vec<Change> = changes
        .into_iter()
        .filter(|c| {
            !(matches!(c.what, What::Delete)
                && c.file
                    .as_deref()
                    .is_some_and(|f| folded.contains(&own_name(f))))
        })
        .collect();
    // Several blocks aimed at the same place in one reply are drafts, and
    // the last is the one meant. A model thinking as it wrote put down a
    // pair of edits, said "wait, I need to recalculate", put down another
    // pair, and again - six blocks, three for one line of one file, and only
    // the last pair right. Offered all six, nothing was taken. The last block
    // for each place stands for the rest.
    let places: Vec<(Option<String>, String)> = changes
        .iter()
        .map(|c| (c.file.as_deref().map(own_name), c.headline("")))
        .collect();
    let changes: Vec<Change> = changes
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !places[i + 1..].contains(&places[*i]))
        .map(|(_, c)| c)
        .collect();
    // A write of one file and deletes of others in the same reply is a merge
    // said the way the instructions say not to say it - and models say it
    // that way anyway. Taken one at a time, in whichever order the buttons
    // are pressed, a delete can go first and a day's note is gone before the
    // week's is made. Read as the merge it is, the parts are gathered before
    // anything moves, and it is one answer instead of three.
    let writes: Vec<usize> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.what, What::Write { .. }) && c.file.is_some())
        .map(|(i, _)| i)
        .collect();
    let deletes: Vec<usize> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c.what, What::Delete) && c.file.is_some())
        .map(|(i, _)| i)
        .collect();
    let changes: Vec<Change> = if writes.len() == 1 && !deletes.is_empty() {
        let w = writes[0];
        let into = changes[w].file.clone();
        let target = own_name(into.as_deref().unwrap_or(""));
        let from: Vec<String> = deletes
            .iter()
            .filter_map(|&d| changes[d].file.clone())
            .filter(|f| own_name(f) != target)
            .collect();
        let text = match &changes[w].what {
            What::Write { text } => text.clone(),
            _ => String::new(),
        };
        let merged = Change {
            file: into,
            what: What::Merge { from, text },
            state: changes[w].state,
        };
        changes
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i != w && !deletes.contains(i))
            .map(|(_, c)| c)
            .chain(std::iter::once(merged))
            .collect()
    } else {
        changes
    };
    (prose, changes)
}

fn every_proposal(reply: &str) -> (String, Vec<Change>) {
    let mut prose = String::new();
    let mut changes = Vec::new();
    let mut at = 0;
    for (kind, tag, open, close) in blocks(reply) {
        let head = &reply[tag..open];
        let body = reply[open..close].trim_matches('\n').to_string();
        // `into` for a merge, which reads as what it is, and `file` for the
        // rest. Either spelling is taken for either, so a model reaching for
        // the wrong one is still understood.
        let named = attr(head, "into").or_else(|| attr(head, "file"));
        let what = match kind {
            "edit" => attr(head, "after")
                .and_then(|a| a.trim().parse().ok())
                .map(|after| What::Insert {
                    after,
                    text: body.clone(),
                })
                .or_else(|| {
                    lines_attr(head).map(|(from, to)| What::Edit {
                        from,
                        to,
                        text: body.clone(),
                    })
                })
                // An edit of no particular lines, of a file it has named, is a
                // write: here is what the file should say. Models reach for
                // `edit` as the general word for changing something, and a new
                // file has no lines to name - asked to make a note of four
                // birthdays, one wrote `<edit file="ages.md">` with the whole
                // note in it, and the block was dropped on the floor for want
                // of a `lines`. Nothing was offered and nothing was written.
                //
                // Only when a file is named. A bare `<edit>` means the note in
                // front of you, and reading that as "replace all of it" is too
                // much to infer from something left out.
                .or_else(|| named.is_some().then(|| What::Write { text: body.clone() })),
            // The rest are about a file by name and none of them means
            // anything without one. `create` as well as `write`, because it is
            // the word a model reaches for and refusing it would be pedantry
            // with a cost.
            "write" | "create" if named.is_some() => Some(What::Write { text: body }),
            "delete" if named.is_some() => Some(What::Delete),
            "merge" if named.is_some() => {
                let from = names(head);
                (!from.is_empty()).then_some(What::Merge { from, text: body })
            }
            _ => None,
        };
        let shut = block_end(reply, kind, close);
        match what {
            Some(what) => {
                prose.push_str(&reply[at..tag]);
                changes.push(Change {
                    file: named,
                    what,
                    state: state_attr(head),
                });
            }
            // Not a block this understands. Left in the prose rather than
            // swallowed, so a malformed one is visible instead of missing.
            None => prose.push_str(&reply[at..shut]),
        }
        at = shut;
    }
    prose.push_str(&reply[at..]);
    (prose.trim().to_string(), changes)
}

/// Where the blocks are: which kind, the tag's start, where its body starts,
/// and where its body ends.
///
/// A block inside a fence is a block being talked about rather than one being
/// made, and counting the fences passed on the way to it is enough to tell
/// which. One walk, so the reader and the writer below cannot disagree about
/// which block is the second one.
fn blocks(text: &str) -> Vec<(&'static str, usize, usize, usize)> {
    const KINDS: &[&str] = &["edit", "write", "create", "delete", "merge"];
    // What a tool answered is quoted whole, and quoting is not proposing. A
    // note read back that has `<write ...>` written in it, or an answer that
    // shows the shape of a block to a model that tried to call one, is text
    // *about* a change: taken for a change it puts a file in front of somebody
    // that nobody asked for, and it took the closing tag of the real block
    // with it, so the reply came out as two changes and a mangled sentence.
    let mut quoted: Vec<(usize, usize)> = Vec::new();
    let mut scan = 0usize;
    while let Some(i) = text[scan..].find("<used") {
        let from = scan + i;
        let end = match text[from..].find("</used>") {
            Some(j) => from + j + "</used>".len(),
            None => text.len(),
        };
        quoted.push((from, end));
        scan = end;
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut in_code = false;
    loop {
        let next = KINDS
            .iter()
            .filter_map(|k| text[at..].find(&format!("<{k}")).map(|i| (at + i, *k)))
            .min_by_key(|(i, _)| *i);
        let Some((start, kind)) = next else { break };
        in_code ^= fences(&text[at..start]);
        if in_code {
            at = start + 1;
            continue;
        }
        if let Some(&(_, end)) = quoted.iter().find(|(a, b)| start >= *a && start < *b) {
            at = end;
            continue;
        }
        // The tag ends at its `>`, or at the end of its line when the `>`
        // was left off: `<write file="shop.md"` and the body on the next
        // line is what one model wrote, and the first `>` after that was
        // the one in `</write>`, which made the block its own closing tag.
        let Some(open) = text[start..]
            .find(['>', '\n'])
            .map(|i| start + i + usize::from(text.as_bytes()[start + i] == b'>'))
        else {
            break;
        };
        let shut = format!("</{kind}>");
        // A delete has nothing inside it, and a model writes the tag on its
        // own - `<delete file="scratch.md">`, or with a slash - as often as it
        // writes the pair. Nothing was proposed and the note stayed. The tag
        // alone is the block.
        if kind == "delete" && !text[open..].contains(&shut) {
            out.push((kind, start, open, open));
            at = open;
            continue;
        }
        let Some(close) = text[open..].find(&shut).map(|i| open + i) else {
            // An opener with nothing closing it is not the end of the reply.
            //
            // Measured: asked for a note of birthdays, the model copied the
            // example tag out of its own instructions - `<edit file="notes.md"
            // lines="12-14">` and all - never closed it, and wrote the real
            // block inside. Stopping here threw away a write that was correct,
            // right down to the day count, and the answer came out empty. Step
            // over the opener and keep reading: what is nested inside it is
            // still a change somebody asked for.
            at = open;
            continue;
        };
        out.push((kind, start, open, close));
        at = close + shut.len();
    }
    out
}

/// Where a block ends, given where its body ends: after the closing tag,
/// or - for a delete written as a lone tag - right where the body would be.
fn block_end(text: &str, kind: &str, close: usize) -> usize {
    let shut = format!("</{kind}>");
    if text[close..].starts_with(&shut) {
        close + shut.len()
    } else {
        close
    }
}

/// Write down what was decided about the `nth` change in a reply.
///
/// Into the tag, so it is carried by the transcript and is still true when the
/// conversation is opened again.
///
/// Every block the change came from, not one block by its number. A change
/// offered is not always one block: drafts of the same edit are one change,
/// and a write with deletes beside it is one merge. Marking the nth block for
/// the nth change marked the wrong draft applied and left the right one
/// waiting to be answered, and a merge folded from three blocks marked one of
/// the three.
pub fn settle(text: &str, change: &Change, taken: bool) -> String {
    let word = if taken { "applied" } else { "rejected" };
    let place = |c: &Change| (c.file.as_deref().map(own_name), c.headline(""));
    let wanted = place(change);
    // For a merge, the write that became it and the deletes it folds in.
    let (target, from): (Option<String>, Vec<String>) = match &change.what {
        What::Merge { from, .. } => (
            change.file.as_deref().map(own_name),
            from.iter().map(|f| own_name(f)).collect(),
        ),
        _ => (None, Vec::new()),
    };
    let mut out = String::new();
    let mut at = 0;
    for (kind, tag, open, close) in blocks(text) {
        let end = block_end(text, kind, close);
        let own = every_proposal(&text[tag..end]).1.into_iter().next();
        let mine = own.as_ref().is_some_and(|c| {
            place(c) == wanted
                || match &c.what {
                    What::Write { .. } | What::Merge { .. } => {
                        c.file.as_deref().map(own_name) == target && target.is_some()
                    }
                    What::Delete => c
                        .file
                        .as_deref()
                        .is_some_and(|f| from.contains(&own_name(f))),
                    _ => false,
                }
        });
        out.push_str(&text[at..tag]);
        if mine {
            let bare = strip_state(&text[tag..open]);
            let bare = bare.trim_end().trim_end_matches('>').trim_end();
            out.push_str(&format!("{bare} state=\"{word}\">"));
            at = open;
        } else {
            at = tag;
        }
    }
    out.push_str(&text[at..]);
    out
}

/// The tag without any decision already written into it.
fn strip_state(tag: &str) -> String {
    let Some(at) = tag.find("state") else {
        return tag.to_string();
    };
    let end = tag[at..]
        .match_indices('"')
        .nth(1)
        .map(|(i, _)| at + i + 1)
        .unwrap_or(tag.len());
    format!("{}{}", &tag[..at], &tag[end..])
}

/// `state="applied"`, if a decision was written into the tag.
fn state_attr(tag: &str) -> Option<bool> {
    match attr(tag, "state")?.as_str() {
        "applied" => Some(true),
        "rejected" => Some(false),
        _ => None,
    }
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
    let value = attr(tag, "lines")?;
    match value.split_once('-') {
        Some((a, b)) => Some((a.trim().parse().ok()?, b.trim().parse().ok()?)),
        None => {
            let one = value.parse().ok()?;
            Some((one, one))
        }
    }
}

/// The files a merge is folding in: `from="a.md, b.md"`.
fn names(tag: &str) -> Vec<String> {
    attr(tag, "from")
        .into_iter()
        .flat_map(|list| {
            list.split([',', ' '])
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// One quoted attribute out of a tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(name)?;
    Some(tag[at..].split('"').nth(1)?.trim().to_string())
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
    pub fn did(&mut self, file: &str, before: &str, after: &str) {
        if before == after {
            return;
        }
        self.done.push(format!(
            "Your edit to `{file}` was applied. {}",
            crate::digest::changed(before, after)
        ));
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
    pub fn show(&mut self, ui: &mut Ui, project: &str, chats: &[Filed]) -> Picked {
        let th = *ui.theme;
        let line_h = font::line_h();
        let screen = ui.canvas.bounds();
        ui.canvas
            .fill_rect_blend(screen, pixui::palette::VOID, 0.55);

        let rows = (chats.len() + 1).min(ROWS);
        let rect = screen.centered(440, rows as i32 * line_h + 4 * line_h + 26);
        let picker = ui.id("chat-picker");
        let inner = ui
            .floating(picker, rect, &format!("CHATS IN {}", called(project)))
            .inner;
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
const WHOLE: usize = 14;

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
                let (said, looked) = if turn.mine {
                    (turn.text.clone(), Vec::new())
                } else {
                    lookups(&turn.text)
                };
                let (prose, edits) = if turn.mine {
                    (said, Vec::new())
                } else {
                    proposals(&said)
                };
                // What it went and found, before what it made of it.
                for look in &looked {
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
        self.turns.iter().filter(|t| !t.mine).any(|turn| {
            proposals(&turn.text)
                .1
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
                None => format!("{span} - WHICH IS NOT THERE TO CHANGE"),
            };
            ui.draw_text_in(head, &why, th.danger.face, Align::Left);
            return None;
        };
        let tint = match change.what {
            What::Delete => th.danger.face,
            What::Write { .. } | What::Merge { .. } => th.positive.face,
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
