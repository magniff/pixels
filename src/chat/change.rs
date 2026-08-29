//! What a model offers to do to a project, read out of what it wrote.
//!
//! A change is a block in the reply - an edit, a write, a delete, a merge -
//! and reading one is more than finding its tags. Blocks come with their
//! decision written into them, drafts of one block supersede each other, a
//! write with deletes beside it is a merge, and a delete a merge covers is
//! nothing. All of that is decided here, once, so that the panel that offers
//! a change and the application that applies it never disagree about what
//! was offered.

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
    /// What a file that is not there yet should say: an `edit` with a file
    /// named and no lines. Models reach for `edit` as the general word for
    /// changing something, and a new file has no lines to name - asked to
    /// make a note of four birthdays, one wrote `<edit file="ages.md">` with
    /// the whole note in it. For a file that is there it means nothing, and
    /// is refused: read as a write it laid one line over a nine-line budget,
    /// when all that was asked was one word in it changed.
    Lay { text: String },
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
            What::Write { text } | What::Lay { text } => (count(text), target),
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
            // "Below line 9" of an eight-line file is below the line that is
            // not there yet, which is the end: the same tolerance an edit one
            // past the end gets, for the same reason. Asked for a row in an
            // eight-line table a model wrote after="9", meaning the row would
            // be the ninth line, and was refused for it.
            What::Insert { after, .. } => {
                let lines = lines?;
                (*after <= lines.len() + 1).then(String::new)
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
            // Only where there is nothing yet. See `What::Lay`.
            What::Lay { .. } => lines.is_none().then(String::new),
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
            What::Edit { text, .. }
            | What::Insert { text, .. }
            | What::Write { text }
            | What::Lay { text } => text.clone(),
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
            What::Write { .. } | What::Lay { .. } => format!("WRITE  {named}"),
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
pub(super) fn undecided(reply: &str) -> String {
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
                // An edit of no particular lines, of a file it has named, is
                // what a file not there yet should say. See `What::Lay`.
                //
                // Only when a file is named. A bare `<edit>` means the note in
                // front of you, and reading that as "replace all of it" is too
                // much to infer from something left out.
                .or_else(|| named.is_some().then(|| What::Lay { text: body.clone() })),
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
pub(super) fn blocks(text: &str) -> Vec<(&'static str, usize, usize, usize)> {
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
pub(super) fn block_end(text: &str, kind: &str, close: usize) -> usize {
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
                    What::Write { .. } | What::Lay { .. } | What::Merge { .. } => {
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
pub(super) fn strip_state(tag: &str) -> String {
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
pub(super) fn state_attr(tag: &str) -> Option<bool> {
    match attr(tag, "state")?.as_str() {
        "applied" => Some(true),
        "rejected" => Some(false),
        _ => None,
    }
}

/// Whether an odd number of code fences opened in this text.
pub(super) fn fences(text: &str) -> bool {
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
pub(super) fn lines_attr(tag: &str) -> Option<(usize, usize)> {
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
pub(super) fn names(tag: &str) -> Vec<String> {
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
pub(super) fn attr(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(name)?;
    Some(tag[at..].split('"').nth(1)?.trim().to_string())
}
