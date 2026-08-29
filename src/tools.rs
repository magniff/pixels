//! What a conversation can reach for, and what happens when it does.
//!
//! Two kinds, and the difference matters more than the list does. Working a sum
//! out happens here, on this machine, and is offered always. Looking something
//! up sends a place name or a search term to somebody else's server, and is
//! offered only when that has been switched on. Keeping them in one list would
//! make the switch mean something it does not.
//!
//! The wording of each description is the whole of whether the model ever
//! reaches for it. Measured on the model this ships against: a tool described
//! as *what it does* - "search the web and return a list of results" - was
//! reached for once in four questions that needed it; the same tool described
//! in terms of *when it is needed*, and saying plainly that the model's own
//! memory of such things is wrong, was reached for four times in four, with no
//! false alarms on the two questions that did not need it.

use std::path::Path;

use crate::llm::Tool;
use crate::{calc, clock, web};

/// Working something out, which needs nothing outside this machine.
fn arithmetic() -> Tool {
    Tool {
        name: "calc",
        about: "Work out a sum exactly. Use this for any arithmetic at all, however easy it \
                looks: you do sums the way you do everything else, by what the answer ought to \
                look like, and you will be confidently a few thousand out. Takes the usual \
                signs and brackets, powers with ^, and sqrt, abs, round, floor, ceil, min, max, \
                log, ln, exp, sin, cos, tan, and pi. It knows nothing about dates: it \
                cannot subtract one from another, and a year multiplied by 365.25 is not \
                how long somebody has been alive. Anything with a date in it goes to the \
                date tool, which answers in days and in years and is never a day out.",
        takes: (
            "expression",
            "The sum on its own, as you would write it: 384 * 517, or (12.5 + 3) / 4, or sqrt(2).",
        ),
    }
}

/// What day it is, which is on the machine and not on the network.
fn calendar() -> Tool {
    Tool {
        name: "date",
        about: "Call this before you say what the time is, what the date is, what year it is, \
                what day of the week it is, whether it is morning or evening, or how long until \
                anything. It gives the clock time and the timezone as well as the day. You do \
                not know any of it and you cannot work it out: asked the date you gave one three \
                weeks out, asked the time you said you had no way of telling, and asked how long \
                until Christmas you used a year that had already gone. It also says how far off \
                another day is, so do not count days yourself - give it the date and read the \
                answer. A day with no year, like 12-25, means the next one there is.",
        takes: (
            "when",
            "Leave it as today for the date now. For a particular day, 2026-12-25, or the \
             month written out in either order - 31 July 1989, July 31 1989. For a day that \
             comes round every year, leave the year off: 12-25, or 25 December. Pass the day \
             somebody gave you rather than working anything out from it.",
        ),
    }
}

/// Read a note as it is on disk now.
///
/// The one tool that reaches into the vault rather than out of the machine.
/// It exists because of what a model does when it is told a file has changed:
/// it tries to go and look, and until this it had nothing to look with, so it
/// answered from the last thing it had been told - which was often its own.
///
/// `here` is the note the conversation is about, folder and all, and decides
/// between two notes of the same name: a `notes.md` in the project being
/// looked at is the one meant, and the one in some other project is not. It
/// used to be whichever the vault listed first.
fn look_at(named: &str, here: &str) -> Result<String, String> {
    let wanted = named
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(named)
        .trim()
        .to_lowercase();
    if wanted.is_empty() {
        return Err("no note was named".into());
    }
    let project = here.rsplit_once(['/', '\\']).map(|(p, _)| p).unwrap_or("");
    let dir = crate::notes_dir();
    let mut same: Vec<_> = crate::read_vault(&dir)
        .into_iter()
        .filter(|note| note.filename().to_lowercase() == wanted)
        .collect();
    same.sort_by_key(|note| note.project != project);
    let found = same.into_iter().next();
    match found {
        Some(note) => Ok(format!(
            "`{}` says, as of now:\n\n{}",
            note.filename(),
            crate::digest::numbered(&note.buffer.to_text())
        )),
        None => Err(format!(
            "there is no note called {named} in the vault. The list of notes is above"
        )),
    }
}

/// Looking at a note, which is the only tool that is about the vault itself.
fn reading() -> Tool {
    Tool {
        name: "read",
        about: "Read a note and get back what it says now, with its lines numbered. Use it \
                whenever you are told a file has changed, whenever you are asked to read one, \
                and before answering about a note you have not been shown recently. What you \
                were told earlier - including anything you wrote yourself - may be out of date; \
                this is not.",
        takes: ("file", "The note's name, without its folder: notes.md"),
    }
}

/// Finding notes by their names.
fn finding() -> Tool {
    Tool {
        name: "find",
        about: "Find notes by name. Give part of a file or folder name - stock, or aquarium/ - \
                and get every note whose path has it in, with its title and how long it is. \
                Use it when a question names a note you cannot see in full, or asks what notes \
                there are about something, before guessing at a name.",
        takes: (
            "name",
            "Part of a note's name or folder, without .md: stock, or aquarium/",
        ),
    }
}

/// Finding which notes say a thing.
fn searching() -> Tool {
    Tool {
        name: "grep",
        about: "Find which notes say something. Give a word or a phrase and get every line in \
                the vault that has it, with the note and the line number, up to forty. Use it \
                when asked where something is mentioned, which notes are about a thing, or \
                before saying that nothing mentions it: the list at the top shows one line of \
                each note, and a word that is not on that line is not in front of you.",
        takes: (
            "text",
            "The word or phrase to look for, as written: ember tetras",
        ),
    }
}

/// Comparing two notes.
fn comparing() -> Tool {
    Tool {
        name: "diff",
        about: "Compare two notes. Give both names - draft.md final.md - and get the lines that \
                differ, as a diff: what the first has that the second does not marked with -, \
                and the other way round with +. Use it when asked what changed between two \
                notes, or which of two versions says what.",
        takes: (
            "files",
            "The two notes to compare, first then second: draft.md final.md",
        ),
    }
}

/// Every note in the vault, read from disk now.
fn vault(dir: &Path) -> Vec<crate::Note> {
    crate::read_vault(dir)
}

/// Notes whose path has this in it, one line each.
pub fn find_in(dir: &Path, named: &str) -> Result<String, String> {
    let wanted = named.trim().trim_end_matches(".md").to_lowercase();
    if wanted.is_empty() {
        return Err("nothing to look for was given".into());
    }
    let found: Vec<String> = vault(dir)
        .iter()
        .filter(|n| n.slug().to_lowercase().contains(&wanted))
        .map(|n| {
            format!(
                "- `{}` \"{}\" ({} lines)",
                n.slug(),
                crate::markdown::derive_title(n.buffer.lines(), 60),
                n.buffer.line_count()
            )
        })
        .collect();
    if found.is_empty() {
        return Ok(format!("no note has {named} in its name."));
    }
    Ok(format!(
        "{} note{} named like that:\n{}",
        found.len(),
        if found.len() == 1 { "" } else { "s" },
        found.join("\n")
    ))
}

/// How many lines a search will show before saying there are more.
const HITS: usize = 40;

/// Every line in the vault with this in it, with the note and the line number.
pub fn grep_in(dir: &Path, text: &str) -> Result<String, String> {
    let wanted = text.trim().to_lowercase();
    if wanted.is_empty() {
        return Err("nothing to look for was given".into());
    }
    let mut hits = Vec::new();
    for note in vault(dir) {
        let slug = note.slug();
        for (i, line) in note.buffer.to_text().lines().enumerate() {
            if line.to_lowercase().contains(&wanted) {
                hits.push(format!("{slug}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    if hits.is_empty() {
        return Ok(format!("nothing in the vault says {text}."));
    }
    let total = hits.len();
    let more = total.saturating_sub(HITS);
    hits.truncate(HITS);
    let mut out = format!(
        "{total} line{} say{} that:\n{}",
        if total == 1 { "" } else { "s" },
        if total == 1 { "s" } else { "" },
        hits.join("\n")
    );
    if more > 0 {
        out.push_str(&format!(
            "\n... and {more} more. Ask for something rarer to see fewer."
        ));
    }
    Ok(out)
}

/// The lines that differ between two notes.
pub fn diff_in(dir: &Path, files: &str) -> Result<String, String> {
    let names: Vec<&str> = files
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .collect();
    let [first, second] = names[..] else {
        return Err("two notes are needed, first then second: draft.md final.md".into());
    };
    let notes = vault(dir);
    let pick = |named: &str| -> Result<(String, String), String> {
        let want = named
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(named)
            .to_lowercase();
        let by_slug = notes
            .iter()
            .find(|n| n.slug().to_lowercase() == named.to_lowercase());
        by_slug
            .or_else(|| notes.iter().find(|n| n.filename().to_lowercase() == want))
            .map(|n| (n.slug(), n.buffer.to_text()))
            .ok_or_else(|| format!("there is no note called {named} in the vault"))
    };
    let (a, before) = pick(first)?;
    let (b, after) = pick(second)?;
    if before == after {
        return Ok(format!("`{a}` and `{b}` say the same thing."));
    }
    Ok(format!(
        "From `{a}` to `{b}`, {}",
        crate::digest::changed(&before, &after)
    ))
}

/// Everything on offer, given whether the network has been allowed.
pub fn available(web_allowed: bool) -> Vec<Tool> {
    let mut out = vec![
        arithmetic(),
        calendar(),
        reading(),
        finding(),
        searching(),
        comparing(),
    ];
    if web_allowed {
        out.extend(web::tools());
    }
    out
}

/// Run one call, and say what came back.
///
/// A tool that fails says so in a sentence rather than returning nothing.
/// Nothing at all is the one answer that reliably makes a model invent: handed
/// an empty result it fills the gap, and that is where a llama.cpp version that
/// has never existed came from.
pub fn run(name: &str, arg: &str, here: &str) -> String {
    // Changing a file is not a tool, and is the one thing models most often
    // try to call as though it were: `<function=write>` with the name and the
    // contents as parameters. Answering "there is no tool called write" is
    // true and useless - it tried again, and again, and the conversation ended
    // with nothing said. So it is told the shape that does work.
    if let "edit" | "write" | "create" | "delete" | "merge" = name {
        return format!(
            "{name} is not a tool. Changing a file is not something you call: write a \
             <{name}> block in your reply instead, at the top level and outside any code \
             fence, exactly as the instructions above describe. Nothing happens to the file \
             until they accept it."
        );
    }
    let done = match name {
        "calc" => calc::evaluate(arg),
        "date" => clock::about(arg),
        "read" => look_at(arg, here),
        "find" => find_in(&crate::notes_dir(), arg),
        "grep" => grep_in(&crate::notes_dir(), arg),
        "diff" => diff_in(&crate::notes_dir(), arg),
        "weather" => web::weather(arg),
        "wikipedia" => web::wikipedia(arg),
        "release" => web::release(arg),
        "fetch" => web::fetch(arg),
        other => Err(format!("there is no tool called {other}")),
    };
    match done {
        Ok(text) => text,
        Err(why) => format!("That did not work: {why}."),
    }
}
