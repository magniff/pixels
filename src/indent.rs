//! What the editor types for you when you open a line or press Tab.
//!
//! This is markdown's grammar and a code block's convention, not the toolkit's
//! business: a list carries on because a list is a thing markdown has, and an
//! indent grows after a colon because that is what Python means by one. The
//! rules are pure functions over the lines, so they can be asserted on rather
//! than eyeballed — the same reason the vim grammar lives apart from drawing.

use crate::markdown::{is_fence, item_marker, Marker};

/// Spaces per level of nesting. Two in prose, because that is what the list
/// parser reads a level as; four in code, because that is what code means.
const LIST_STEP: usize = 2;
const CODE_STEP: usize = 4;

/// What pressing Enter should do with the line it is splitting.
#[derive(Debug, PartialEq, Eq)]
pub enum Opened {
    /// Nothing to carry down: an ordinary new line.
    Plain,
    /// Begin the new line with this text.
    With(String),
    /// The marker was all the line held. Empty it instead of carrying the
    /// marker down — that is how a list is ended, and an editor that continues
    /// lists without this one traps you in a list you cannot leave.
    Ending,
}

/// Whether the line at `at` is inside a fenced code block.
///
/// Counted from the top of the file rather than searched for nearby, because
/// "inside a fence" is a fact about everything above you.
pub fn in_code(lines: &[String], at: usize) -> bool {
    lines
        .iter()
        .take(at)
        .filter(|l| is_fence(l))
        .count()
        .is_multiple_of(2)
        .eq(&false)
}

/// One level of indent where the caret is standing.
pub fn step(lines: &[String], at: usize) -> usize {
    if in_code(lines, at) {
        CODE_STEP
    } else {
        LIST_STEP
    }
}

/// What a new line opened at `col` on line `at` should begin with.
pub fn opened(lines: &[String], at: usize, col: usize) -> Opened {
    let Some(line) = lines.get(at) else {
        return Opened::Plain;
    };
    if in_code(lines, at) {
        return in_code_block(line, col);
    }
    in_prose(line, col)
}

/// The leading whitespace of a line.
fn leading(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Inside a fence: keep the indent, and take one more level after a line that
/// opens a block. A colon is Python's way of saying so and a bracket is
/// everyone else's; both are only ever a guess, which is why Tab and Shift-Tab
/// exist to correct it.
fn in_code_block(line: &str, col: usize) -> Opened {
    let indent = leading(line);
    if col < indent.chars().count() {
        return Opened::Plain;
    }
    let mut out = indent.to_string();
    if line.trim_end().ends_with([':', '{', '(', '[']) {
        out.push_str(&" ".repeat(CODE_STEP));
    }
    if out.is_empty() {
        Opened::Plain
    } else {
        Opened::With(out)
    }
}

/// The leading run of quote markers, if the line is quoted at all.
fn quote_bar(line: &str) -> &str {
    let n = line
        .char_indices()
        .find(|(_, c)| *c != '>' && *c != ' ')
        .map_or(line.len(), |(i, _)| i);
    let bar = &line[..n];
    if bar.contains('>') {
        bar
    } else {
        ""
    }
}

fn in_prose(line: &str, col: usize) -> Opened {
    // A quote carries its markers down, and whatever is inside one continues
    // on its own terms — a list in a quote is still a list.
    let bar = quote_bar(line);
    if !bar.is_empty() {
        let taken = bar.chars().count();
        return match in_prose(&line[bar.len()..], col.saturating_sub(taken)) {
            Opened::With(p) => Opened::With(format!("{bar}{p}")),
            // The list inside ends, but the quote around it does not.
            Opened::Plain | Opened::Ending => Opened::With(bar.to_string()),
        };
    }

    let indent = leading(line);
    let Some((marker, used)) = item_marker(line) else {
        return if indent.is_empty() || col < indent.chars().count() {
            Opened::Plain
        } else {
            Opened::With(indent.to_string())
        };
    };
    // Splitting inside the marker is not continuing the list; it is cutting the
    // marker in half, and the half that moves down needs no second one.
    if col < line[..used].chars().count() {
        return Opened::Plain;
    }
    if line[used..].trim().is_empty() {
        return Opened::Ending;
    }
    let body = line.trim_start();
    let bullet = body.chars().next().unwrap_or('-');
    Opened::With(match marker {
        Marker::Bullet => format!("{indent}{bullet} "),
        // Always unchecked: the new item is a task you have not done.
        Marker::Task(_) => format!("{indent}{bullet} [ ] "),
        Marker::Number(n) => {
            // `1.` and `1)` are both ordered lists, and a list that changes
            // punctuation halfway down is a list that stops being one.
            let dot = body.chars().find(|c| *c == '.' || *c == ')').unwrap_or('.');
            format!("{indent}{}{dot} ", n + 1)
        }
    })
}

/// Move a line one level in or out, and say how far its text moved.
///
/// Outdenting takes back whatever indent is actually there, up to a level: a
/// line indented by three spaces should come back to the margin rather than
/// keeping one space nobody meant to type.
pub fn shifted(line: &str, out: bool, step: usize) -> (String, i32) {
    if !out {
        return (format!("{}{line}", " ".repeat(step)), step as i32);
    }
    if let Some(rest) = line.strip_prefix('\t') {
        return (rest.to_string(), -1);
    }
    let spaces = line.chars().take_while(|c| *c == ' ').count().min(step);
    (line[spaces..].to_string(), -(spaces as i32))
}
