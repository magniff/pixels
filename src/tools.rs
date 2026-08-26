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
                log, ln, exp, sin, cos, tan, and pi.",
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
        about: "Call this before you say what the date is, what year it is, what day of the week \
                it is, or how long until anything. You do not know any of those and you cannot \
                work them out: asked the date you gave one three weeks out, and asked how long \
                until Christmas you used a year that had already gone. It also says how far off \
                another day is, so do not count days yourself - give it the date and read the \
                answer. A day with no year, like 12-25, means the next one there is.",
        takes: (
            "when",
            "Leave it as today for the date now, 2026-12-25 for a particular day, or 12-25 for \
             the next time that day comes round.",
        ),
    }
}

/// Everything on offer, given whether the network has been allowed.
pub fn available(web_allowed: bool) -> Vec<Tool> {
    let mut out = vec![arithmetic(), calendar()];
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
    let done = match name {
        "calc" => calc::evaluate(arg),
        "date" => clock::about(arg),
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
