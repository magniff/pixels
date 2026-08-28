//! What the model is told about the rest of your notes.
//!
//! A model asked to tighten a sentence sees the sentence, and that is all it
//! has ever seen. It does not know the note the sentence is in, which is how
//! you get a rewrite that repeats the heading two lines above it, or renames a
//! thing the next paragraph goes on to use. And it does not know the other
//! notes, which is how you get one that quietly contradicts them.
//!
//! Two answers here, of different weights. Every note gets a line — its name,
//! what it calls itself, and the first thing it says — which is enough for the
//! model to know a subject exists and which file it is in. The note actually
//! being edited is given whole, with the passage marked in place.
//!
//! Both are built from the text's own structure rather than written by the
//! model. That is deliberate: a summary the model wrote needs a cache, an
//! invalidation rule, and a background pass to keep it current, and for notes
//! that are themselves a page long it is not clear it beats the first line.
//!
//! The digest is the *same for every request* — every note, always, in
//! filename order, whichever one you happen to be editing. That costs a few
//! redundant tokens for the note that is also given in full, and buys the one
//! property worth having: it is a stable prefix. Everything volatile comes
//! after it, so it can eventually be decoded once and kept rather than re-read
//! on every question.

use crate::text::{Buffer, Cursor};
use crate::Note;

/// How much of a note's first line to quote.
const GIST: usize = 140;

/// How much of a note's title to quote.
const TITLE: usize = 60;

/// How many of a note's headings to name.
const SECTIONS: usize = 6;

/// How much of the surrounding note to send, in characters.
///
/// A window rather than the whole thing once a note gets long: the point is to
/// know what the passage is part of, and four thousand characters either side
/// of it is more than enough for that. Prefill is the cost that matters here -
/// it grows faster than linearly - so this is the difference between an answer
/// that arrives and one you wait out.
const ROOM: usize = 8000;

/// Where the passage under discussion is marked in the note around it.
pub const OPEN: &str = "<selection>";
pub const CLOSE: &str = "</selection>";

/// One line per note: where it sits, what it calls itself, and its gist.
///
/// Ordered by path rather than by the order the vault happened to be read in,
/// so the same vault always produces the same text - and sorting by path is
/// what puts a project's notes together under each other.
pub fn vault(notes: &[Note]) -> String {
    let mut lines: Vec<String> = notes.iter().map(entry).collect();
    lines.sort();
    lines.join("\n")
}

fn entry(note: &Note) -> String {
    let lines = note.buffer.lines();
    // The note's own title rather than the sidebar's: that one is cut to
    // twenty-four characters and shouted, because it lives in a narrow column.
    // "FIBONACCI NUMBERS: A SI~" tells the model less than nothing.
    let title = crate::markdown::derive_title(lines, TITLE);
    // Named by where it sits rather than by what it is called, so a vault of
    // projects reads as one: two of them may each have a `todo.md`, and a list
    // that says `todo.md` twice has told the model nothing about either.
    let mut out = format!("- `{}` \"{}\"", note.slug(), title);
    if let Some(gist) = gist(lines) {
        out.push_str(": ");
        out.push_str(&gist);
    }
    let sections = sections(lines);
    if !sections.is_empty() {
        out.push_str(" (sections: ");
        out.push_str(&sections.join(", "));
        out.push(')');
    }
    out
}

/// The first thing a note actually says, headings and markup aside.
fn gist(lines: &[String]) -> Option<String> {
    let mut in_code = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        // A table row is markup standing in for prose: "| key | moves |" says
        // nothing about the note that its section headings do not say better.
        if in_code || t.is_empty() || t.starts_with('#') || t.starts_with('|') || is_rule(t) {
            continue;
        }
        // A heading is not always spelled with a hash. The other form is a
        // line of prose with a rule drawn under it, and it is the form nearly
        // every note that has one uses at the very top - so without this the
        // gist of half the vault is its own title again.
        if lines.get(i + 1).is_some_and(|n| is_rule(n.trim())) {
            continue;
        }
        return Some(clip(t, GIST));
    }
    None
}

/// The headings under the title, which are the shape of the note.
fn sections(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    for line in lines {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if let Some(rest) = t.strip_prefix('#') {
            let text = rest.trim_start_matches('#').trim();
            if !text.is_empty() {
                out.push(clip(text, 40));
            }
        }
        if out.len() > SECTIONS {
            break;
        }
    }
    // The first heading is the title, which has already been named.
    if !out.is_empty() {
        out.remove(0);
    }
    out.truncate(SECTIONS);
    out
}

/// A line that is a horizontal rule, or the underline of a setext heading.
fn is_rule(t: &str) -> bool {
    t.len() >= 3
        && t.chars()
            .all(|c| c == '-' || c == '=' || c == '*' || c == '_' || c == ' ')
}

/// Cut to `room` characters on a word boundary, with an ellipsis if it bit.
fn clip(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_string();
    }
    let head: String = text.chars().take(room).collect();
    let cut = head.rfind(' ').unwrap_or(head.len());
    format!("{}...", head[..cut].trim_end())
}

/// The note with the passage between `from` and `to` marked in place.
///
/// Returns `None` when there is nothing around the passage worth sending -
/// when the selection *is* the note - because repeating it under a second
/// heading only invites the model to answer about the wrong copy.
pub fn around(buf: &Buffer, from: Cursor, to: Cursor) -> Option<String> {
    let whole = buf.to_text();
    let before = offset_of(&whole, from);
    let after = offset_of(&whole, to);
    marked(&whole, before, after)
}

/// The same, given the passage's byte range rather than its cursors.
pub fn marked(whole: &str, before: usize, after: usize) -> Option<String> {
    let (head, rest) = whole.split_at(before.min(whole.len()));
    let (mid, tail) = rest.split_at(after.saturating_sub(before).min(rest.len()));
    if head.trim().is_empty() && tail.trim().is_empty() {
        return None;
    }
    // Kept to whole lines: a window that starts mid-sentence reads as a note
    // that starts mid-sentence, and the model will try to finish it.
    let head = keep_last(head, ROOM / 2);
    let tail = keep_first(tail, ROOM / 2);
    Some(format!("{head}{OPEN}{mid}{CLOSE}{tail}"))
}

/// The byte offset a cursor sits at in the whole text.
fn offset_of(text: &str, at: Cursor) -> usize {
    let mut offset = 0;
    for (i, line) in text.split('\n').enumerate() {
        if i == at.line {
            return offset
                + line
                    .char_indices()
                    .nth(at.col)
                    .map_or(line.len(), |(b, _)| b);
        }
        offset += line.len() + 1;
    }
    text.len()
}

/// The last `room` characters, from a line boundary.
fn keep_last(text: &str, room: usize) -> String {
    if text.len() <= room {
        return text.to_string();
    }
    let cut = text.len() - room;
    let cut = text[cut..].find('\n').map_or(text.len(), |i| cut + i + 1);
    format!("[...earlier lines not shown...]\n{}", &text[cut..])
}

/// The first `room` characters, to a line boundary.
fn keep_first(text: &str, room: usize) -> String {
    if text.len() <= room {
        return text.to_string();
    }
    let cut = text[..room].rfind('\n').unwrap_or(room);
    format!("{}\n[...later lines not shown...]", &text[..cut])
}

/// Every file in a project, numbered, for a conversation that may be asked to
/// change one of them.
///
/// All of them rather than the one being read: a project is the unit somebody
/// thinks in, and half the questions worth asking about a note are about the
/// note next to it. Whole rather than summarised, because a change has to name
/// lines and lines are only real in the whole file.
pub fn project(files: &[(String, String)]) -> String {
    let mut out = String::new();
    for (name, text) in files {
        out.push_str(&format!("=== {name} ===\n"));
        out.push_str(&numbered(text));
        out.push('\n');
    }
    out
}

/// A note with its lines numbered, for a conversation that may be asked to
/// change one of them.
///
/// Numbers rather than quoting the text back: a model asked to reproduce the
/// exact line it means will get a space or a dash wrong eventually, and then
/// the edit lands nowhere. A number is a number. It also gives the two sides
/// something to point at - "line 14" is a thing both can say.
/// A file's lines, counted the way the editor counts them.
///
/// On `\n` and not by `str::lines`, because the buffer in the editor splits on
/// `\n` - so a file ending in a newline has an empty last line, the margin
/// numbers it, and a line number the model is given has to mean the same thing
/// here as it does there. Getting this wrong showed the model lines numbered
/// one to six and then told it the file was five lines long.
fn rows(text: &str) -> Vec<&str> {
    text.split('\n').collect()
}

pub fn numbered(text: &str) -> String {
    let mut out = String::new();
    for (i, line) in rows(text).into_iter().enumerate() {
        out.push_str(&format!("{:>4} | {line}\n", i + 1));
    }
    out
}

/// A file's contents in eight bytes, for telling whether it is the one we saw.
///
/// Not a secure hash and not trying to be: the two things being compared are
/// two versions of a note on one person's disk, minutes apart. What it buys is
/// that "has this changed" is one number against one number, whatever the file
/// weighs - so a project of long notes is walked without comparing a word of
/// them, and a file that has not moved is not looked at twice.
pub fn fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// What has changed in a project since the model was shown it.
///
/// `None` when nothing has. Otherwise a note saying which files moved and what
/// their lines say now, meant to go at the *end* of a question rather than at
/// the front with the rest of the project.
///
/// That placement is the whole point. Everything before the newest question is
/// read once and kept; changing a character of the project at the front throws
/// all of it away and reads it again - measured, a one-line edit to a
/// five-thousand-token project cost 8.7 seconds on the next question, against
/// 0.4 for the same question with nothing moved. Left alone at the front and
/// corrected at the end, that becomes 0.4 as well, and the cost is the size of
/// the edit rather than the size of the project.
///
/// The correction has to be believed over the copy above it, and it is: shown
/// a file saying the bicycle is red and then told the line now says green, the
/// model says green, and still says green a turn later.
pub fn since(shown: &[(String, String)], now: &[(String, String)]) -> Option<String> {
    let mut out = String::new();
    for (name, text) in now {
        match shown.iter().find(|(n, _)| n == name) {
            // Told about it already and it weighs the same to the byte: there
            // is nothing to say, and nothing was read to find that out beyond
            // the two numbers. This is the common case by a long way - most
            // files in a project are untouched by any one edit - and it is the
            // case that has to cost nothing.
            Some((_, before)) if fingerprint(before) == fingerprint(text) => {}
            None => {
                out.push_str(&format!(
                    "`{name}` now contains, in full:\n\n{}\n\n",
                    numbered(text)
                ));
            }
            Some((_, before)) => {
                out.push_str(&format!("In `{name}`, {}\n\n", replaced(before, text)));
            }
        }
    }
    for (name, _) in shown {
        if !now.iter().any(|(n, _)| n == name) {
            out.push_str(&format!("`{name}` is gone.\n\n"));
        }
    }
    (!out.is_empty()).then(|| {
        format!(
            "STOP. The files below have changed on disk since anything you have been \
             told about them - including the copy written out at the top of this \
             conversation, and including anything you or the user said about them in \
             earlier turns. All of that is out of date and must be ignored. What \
             follows is the only current text. Answer from this and nothing else.\n\n{}",
            out.trim_end()
        )
    })
}

/// What has changed about the list of notes, rather than the list again.
///
/// `None` when nothing has. The list is one line per note in the whole vault,
/// derived from what each note says (its title, its first line, its headings),
/// so writing a single word into a single note moves it. Correcting it by
/// sending the whole thing again meant every note in the vault appearing twice
/// in the same prompt, once at the front and once at the end, with the one
/// line that actually differed buried among them.
///
/// The same trade the project text makes: say what moved. A vault of ten notes
/// costs about a thousand characters to list and about eighty to correct.
pub fn relisted(before: &str, after: &str) -> Option<String> {
    // The path in backticks at the head of the line is what names the note;
    // everything after it is derived and is exactly what moves.
    let key = |line: &str| -> Option<String> { line.split('`').nth(1).map(|k| k.to_string()) };
    let seen: Vec<(Option<String>, &str)> = before.lines().map(|l| (key(l), l)).collect();
    let mut changed = String::new();
    for line in after.lines() {
        let named = key(line);
        let known = named
            .as_ref()
            .and_then(|k| seen.iter().find(|(n, _)| n.as_ref() == Some(k)));
        match known {
            Some((_, was)) if *was == line => {}
            _ => {
                changed.push_str(line);
                changed.push('\n');
            }
        }
    }
    let gone: Vec<&str> = seen
        .iter()
        .filter_map(|(n, _)| n.as_deref())
        .filter(|k| !after.lines().any(|l| key(l).as_deref() == Some(k)))
        .collect();
    if changed.is_empty() && gone.is_empty() {
        return None;
    }
    // Past the point where naming the differences is smaller than naming them
    // all, which is where a project rewritten wholesale ends up.
    if changed.len() + gone.len() * 16 >= after.len() {
        return Some(format!(
            "The list of notes at the top is out of date. It is now:\n\n{after}"
        ));
    }
    let mut out = String::from("The list of notes at the top is out of date.");
    if !changed.is_empty() {
        out.push_str(&format!(
            " These lines of it are different now:\n\n{}",
            changed.trim_end()
        ));
    }
    if !gone.is_empty() {
        let named: Vec<String> = gone.iter().map(|k| format!("`{k}`")).collect();
        out.push_str(&format!(
            "\n\nThese notes are no longer in the vault: {}",
            named.join(", ")
        ));
    }
    Some(out)
}

/// How many unchanged lines to show either side of a change.
///
/// Two rather than the three a unified diff usually carries. A note is not
/// source code and the surrounding lines are usually prose, which is long: the
/// third line of context costs more here than it does anywhere else, and two is
/// enough to say where in the file this is.
const CONTEXT: usize = 2;

/// The most cells of comparison to do before giving up and showing the lot.
///
/// The table is one cell per pair of lines that might match, so it grows with
/// the product of the two sides. Trimming the common ends first means the two
/// sides are usually tiny, and this only ever comes up when a file has been
/// rewritten wholesale - at which point showing what changed is showing the
/// file anyway.
const CELLS: usize = 4_000_000;

/// The lines of a file that are not the same any more, as a diff.
///
/// The shape every diff has had since 1975, because it is the shape these
/// models have read more of than any other: `@@` to say where, `-` for what
/// went, `+` for what came. Line numbers count from one, as in the margin, so
/// a model reading this can turn round and write an `<edit lines="...">`
/// against the numbers it just read.
///
/// It says the *lines*, not the stretch containing them. What was here before
/// matched the two files from their ends and sent everything in between, which
/// is right when an edit is one contiguous thing and is what an edit usually
/// is. When it is not, it was hopeless: two words changed a hundred and ninety
/// lines apart in a two-hundred line note sent a hundred and ninety-one lines,
/// to say two. That is the case this exists for - a one-line change in a large
/// file - and it now sends two lines and their context.
fn replaced(before: &str, after: &str) -> String {
    let (old, new) = (rows(before), rows(after));
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    // Give back a little of each end, so a change at the very start or end of
    // what moved still has a line or two either side of it saying where it is.
    // Those lines are the same in both files, so they walk through as context
    // and cost only themselves.
    let room = old.len().min(new.len());
    let head = head.saturating_sub(CONTEXT);
    let tail = tail.saturating_sub(CONTEXT).min(room - head);
    let (o, n) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
    let ends = format!("The file is {} lines long.", new.len());
    if o.is_empty() && n.is_empty() {
        return format!("nothing changed after all. {ends}");
    }
    let steps = if o.len().saturating_mul(n.len()) > CELLS {
        // Too much to line up, which means almost none of it lines up. One
        // hunk saying "all of this, for all of that" is the honest answer.
        o.iter()
            .map(|line| ('-', *line))
            .chain(n.iter().map(|line| ('+', *line)))
            .collect()
    } else {
        walked(o, n)
    };
    let text = written(&steps, head);
    // A diff longer than the thing it describes is not worth reading.
    if text.len() >= after.len() {
        return format!("it now reads, in full:\n\n{}\n\n{ends}", numbered(after));
    }
    format!("these lines changed:\n\n{text}\n{ends}")
}

/// The two sides walked together into keeps, removals and additions.
fn walked<'a>(o: &[&'a str], n: &[&'a str]) -> Vec<(char, &'a str)> {
    let (rows, cols) = (o.len(), n.len());
    // Longest common subsequence, from the far corner back, so that walking
    // forward through it takes the longest run of lines the two share.
    let mut table = vec![0u32; (rows + 1) * (cols + 1)];
    let at = |i: usize, j: usize| i * (cols + 1) + j;
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            table[at(i, j)] = if o[i] == n[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < rows && j < cols {
        if o[i] == n[j] {
            out.push((' ', o[i]));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            out.push(('-', o[i]));
            i += 1;
        } else {
            out.push(('+', n[j]));
            j += 1;
        }
    }
    out.extend(o[i..].iter().map(|line| ('-', *line)));
    out.extend(n[j..].iter().map(|line| ('+', *line)));
    out
}

/// The steps written out as hunks, with their line numbers.
///
/// `skip` is how many lines of the file were the same at the front and so were
/// never walked; every number here is counted from one in the whole file.
fn written(steps: &[(char, &str)], skip: usize) -> String {
    // Where each step sits in the two files.
    let mut placed: Vec<(char, &str, usize, usize)> = Vec::new();
    let (mut o, mut n) = (skip, skip);
    for (mark, line) in steps {
        placed.push((*mark, line, o, n));
        match mark {
            '-' => o += 1,
            '+' => n += 1,
            _ => {
                o += 1;
                n += 1;
            }
        }
    }
    // Changed steps, grouped while they are near enough to share context.
    let moved: Vec<usize> = placed
        .iter()
        .enumerate()
        .filter(|(_, (m, _, _, _))| *m != ' ')
        .map(|(i, _)| i)
        .collect();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for i in moved {
        match groups.last_mut() {
            Some((_, last)) if i <= *last + CONTEXT * 2 + 1 => *last = i,
            _ => groups.push((i, i)),
        }
    }
    let mut out = String::new();
    for (first, last) in groups {
        let from = first.saturating_sub(CONTEXT);
        let to = (last + CONTEXT).min(placed.len() - 1);
        let span = &placed[from..=to];
        let count = |want: char| {
            span.iter()
                .filter(|(m, _, _, _)| *m == want || *m == ' ')
                .count()
        };
        let start = |pick: fn(&(char, &str, usize, usize)) -> usize| {
            span.first().map(pick).unwrap_or(0) + 1
        };
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            start(|s| s.2),
            count('-'),
            start(|s| s.3),
            count('+')
        ));
        for (mark, line, _, _) in span {
            out.push_str(&format!("{mark}{line}\n"));
        }
    }
    out
}
