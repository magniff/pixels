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
fn look_at(named: &str) -> Result<String, String> {
    let wanted = named
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(named)
        .trim()
        .to_lowercase();
    if wanted.is_empty() {
        return Err("no note was named".into());
    }
    let dir = crate::notes_dir();
    let found = crate::read_vault(&dir)
        .into_iter()
        .find(|note| note.filename().to_lowercase() == wanted);
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

/// Everything on offer, given whether the network has been allowed.
pub fn available(web_allowed: bool) -> Vec<Tool> {
    let mut out = vec![arithmetic(), calendar(), reading()];
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
pub fn run(name: &str, arg: &str) -> String {
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
        "read" => look_at(arg),
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
