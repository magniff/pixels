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

/// One line per note: what it is called, what it is filed as, and its gist.
///
/// Ordered by filename rather than by the order the vault happened to be read
/// in, so the same vault always produces the same text.
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
    let mut out = format!("- `{}` \"{}\"", note.filename(), title);
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
