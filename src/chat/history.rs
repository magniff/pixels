//! The conversation as the model is shown it, which is not as it was said.
//!
//! What goes back is not the transcript. A block already accepted has its
//! body taken out unless it is the only copy of what a file says; a lookup
//! goes back as something the model was told rather than as a tag it could
//! copy; and whatever a turn of theirs cannot keep travels forward and is
//! said in the next turn of ours, which is somewhere the model does not
//! write. Every rule here came from a model writing something it had seen.

use super::change::{attr, block_end, blocks, own_name, state_attr};
use crate::llm::Turn;

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
                "accepted, but the file has changed since and no longer says that"
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
