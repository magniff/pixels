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

impl Lookup {
    /// What it did, in words.
    ///
    /// `date` and `today` are what the machinery calls it, and printing them
    /// puts the wiring in front of somebody who asked what time it was. The
    /// call is worth showing - an answer that came from somewhere should say
    /// so - but showing it is not the same as showing the arguments it was
    /// made with.
    pub fn said(&self) -> String {
        let arg = self.arg.trim();
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
    /// The note in front of you, which is what an unqualified change means.
    pub here: String,
    pub files: Vec<(String, &'a [String])>,
}

impl Folder<'_> {
    /// The file a change is about, if it is there at all.
    pub fn lines(&self, named: Option<&String>) -> Option<&[String]> {
        let want = named.cloned().unwrap_or_else(|| self.here.clone());
        self.files
            .iter()
            .find(|(name, _)| *name == want)
            .map(|(_, lines)| *lines)
    }
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
    /// Lines gone and lines arrived, the way a diff counts them.
    pub fn tally(&self, folder: &Folder) -> (usize, usize) {
        let count = |t: &str| if t.is_empty() { 0 } else { t.lines().count() };
        let target = folder
            .lines(self.file.as_ref())
            .map(|l| l.len())
            .unwrap_or(0);
        match &self.what {
            What::Edit { from, to, text } => (count(text), to.saturating_sub(*from) + 1),
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
        let lines = folder.lines(self.file.as_ref());
        match &self.what {
            What::Edit { from, to, .. } => {
                let lines = lines?;
                let first = from.checked_sub(1)?;
                if first >= lines.len() || to < from {
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
            What::Edit { text, .. } | What::Write { text } => text.clone(),
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
            What::Write { .. } => format!("WRITE  {named}"),
            What::Delete => format!("DELETE  {named}"),
            What::Merge { from, .. } => format!("MERGE  {}  INTO  {named}", from.join(", ")),
        }
    }
}

/// Split a reply into what it said and what it proposed.
///
/// The blocks are lifted out of the prose rather than left in it: a reply is
/// read as a sentence and a change, and showing the raw block would be showing
/// somebody the machinery instead of the change.
pub fn proposals(reply: &str) -> (String, Vec<Change>) {
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
            "edit" => lines_attr(head).map(|(from, to)| What::Edit {
                from,
                to,
                text: body,
            }),
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
        let shut = close + kind.len() + 3;
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
        let Some(open) = text[start..].find('>').map(|i| start + i + 1) else {
            break;
        };
        let shut = format!("</{kind}>");
        let Some(close) = text[open..].find(&shut).map(|i| open + i) else {
            break;
        };
        out.push((kind, start, open, close));
        at = close + shut.len();
    }
    out
}

/// Write down what was decided about the `nth` change in a reply.
///
/// Into the tag, so it is carried by the transcript and is still true when the
/// conversation is opened again.
pub fn settle(text: &str, nth: usize, taken: bool) -> String {
    for (i, (_, tag, open, _)) in blocks(text).into_iter().enumerate() {
        if i == nth {
            let word = if taken { "applied" } else { "rejected" };
            let bare = strip_state(&text[tag..open]);
            let bare = bare.trim_end().trim_end_matches('>').trim_end();
            return format!("{}{bare} state=\"{word}\">{}", &text[..tag], &text[open..]);
        }
    }
    text.to_string()
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
                let p = self.progress;
                let row = ui.alloc(line_h);
                let said = if p.looking {
                    format!("LOOKING SOMETHING UP... ({} SO FAR)", p.steps)
                } else if p.deliberating {
                    format!("THINKING... {} TOKENS", p.written)
                } else if p.written > 0 {
                    format!("WRITING... {} TOKENS, {:.0}/S", p.written, p.rate())
                } else if p.prompt > 0 {
                    // A share rather than a count: the number of tokens in a
                    // vault is not something anybody has a feel for, and how
                    // near the end it is, is the only part being watched.
                    format!("READING THE NOTES... {}%", 100 * p.read / p.prompt.max(1))
                } else {
                    "READING THE NOTES...".to_string()
                };
                font::draw_text_styled(ui.canvas, row.x + 4, row.y, &said, th.info.hi, true);
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
                self.turns[i].text = settle(&self.turns[i].text, j, taken);
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
            // or one being made that already is. Said rather than offered,
            // because there is no honest diff to draw for any of them.
            ui.draw_text_in(
                head,
                &format!("{span} - WHICH IS NOT THERE TO CHANGE"),
                th.danger.face,
                Align::Left,
            );
            return None;
        };
        let tint = match change.what {
            What::Delete => th.danger.face,
            What::Write { .. } | What::Merge { .. } => th.positive.face,
            What::Edit { .. } => th.info.hi,
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
