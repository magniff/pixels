//! Asking a language model to edit a piece of text, off the frame loop.
//!
//! The model lives on a worker thread and is spoken to through two channels.
//! Nothing here blocks: a request is posted, the frame carries on drawing at
//! sixty a second, and the reply is collected whenever it turns up. A model
//! that takes two seconds to answer must not cost two seconds of frames.
//!
//! The backend is a trait so the interface can be built and tested against a
//! stub. Which model answers is a detail; that the editor can ask at all is
//! the part worth getting right.

#[cfg(feature = "llm")]
pub mod local;

use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// A request: the text to change, and what to do to it.
#[derive(Clone, Debug)]
pub struct Ask {
    pub source: String,
    pub request: String,
}

/// What came back, or why nothing did.
pub type Reply = Result<String, String>;

/// Something that can rewrite a piece of text on request.
pub trait Backend: Send + 'static {
    /// Shown in the status bar, so it is never a mystery which one answered.
    fn name(&self) -> String;
    fn edit(&mut self, ask: &Ask) -> Reply;
}

/// A backend on a thread, with one question outstanding at a time.
pub struct Assistant {
    tx: Sender<Ask>,
    rx: Receiver<Reply>,
    name: String,
    busy: bool,
}

impl Assistant {
    pub fn spawn(mut backend: Box<dyn Backend>) -> Self {
        let name = backend.name();
        let (tx, asks) = std::sync::mpsc::channel::<Ask>();
        let (replies, rx) = std::sync::mpsc::channel::<Reply>();
        std::thread::spawn(move || {
            // Ends when the application drops its end of the channel.
            while let Ok(ask) = asks.recv() {
                let reply = backend.edit(&ask).map(|text| to_ascii(&text));
                if replies.send(reply).is_err() {
                    break;
                }
            }
        });
        Self {
            tx,
            rx,
            name,
            busy: false,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Post a question. Refused while one is already outstanding, so a second
    /// click cannot leave two answers racing for the same selection.
    pub fn ask(&mut self, ask: Ask) -> bool {
        if self.busy {
            return false;
        }
        self.busy = self.tx.send(ask).is_ok();
        self.busy
    }

    /// Collect an answer if one has arrived. Never blocks.
    pub fn poll(&mut self) -> Option<Reply> {
        match self.rx.try_recv() {
            Ok(reply) => {
                self.busy = false;
                Some(reply)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.busy = false;
                Some(Err("the assistant went away".into()))
            }
        }
    }
}

/// Take the answer out of whatever a model wrapped it in.
///
/// Small models announce themselves — "Here is the proofread version:" — and
/// fence things they were not asked to fence. The prompt asks for the passage
/// between `<text>` and `</text>` precisely so this can be exact rather than a
/// guess about which opening sentence is a preamble and which is the text.
/// Everything else here is a fallback for a model that ignored that.
pub fn clean_reply(text: &str) -> String {
    let mut out = text.trim();

    // The delimiters, when they are there. A missing closing tag still tells
    // us where the answer began, which is the half that matters.
    if let Some(start) = out.find("<text>") {
        out = &out[start + "<text>".len()..];
        if let Some(end) = out.find("</text>") {
            out = &out[..end];
        }
    }
    out = out.trim();

    // A code fence around prose, which no instruction ever quite stops.
    if let Some(rest) = out.strip_prefix("```") {
        let rest = rest.split_once('\n').map_or(rest, |(_, r)| r);
        out = rest.strip_suffix("```").unwrap_or(rest).trim();
    }

    // And a whole answer in quotes, which is not the same as an answer that
    // happens to contain them.
    if out.len() > 1
        && out.starts_with('"')
        && out.ends_with('"')
        && !out[1..out.len() - 1].contains('"')
    {
        out = &out[1..out.len() - 1];
    }
    out.trim().to_string()
}

/// Fold a reply into the alphabet the editor can actually draw.
///
/// The font is 5x7 and ASCII. A model that answers with em dashes, curly
/// quotes and a couple of party emoji is not wrong — it is writing for a
/// different screen — but every one of those lands in a note as a missing-glyph
/// box. The common punctuation has an obvious ASCII spelling and gets it;
/// anything left over is dropped rather than drawn as a box.
///
/// Applied to every backend's answer, because the limit belongs to the editor
/// rather than to whichever model happened to answer.
pub fn to_ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            c if c.is_ascii() => out.push(c),
            // An em dash is two hyphens in ASCII prose; a single one reads as a
            // hyphenated word and joins the two clauses it was separating.
            '\u{2014}' | '\u{2015}' => out.push_str("--"),
            '\u{2013}' | '\u{2012}' | '\u{2212}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{2018}' | '\u{2019}' | '\u{201b}' => out.push('\''),
            '\u{201c}' | '\u{201d}' | '\u{201f}' => out.push('"'),
            '\u{00a0}' | '\u{2007}' | '\u{202f}' => out.push(' '),
            '\u{2022}' | '\u{00b7}' => out.push('-'),
            '\u{00d7}' => out.push('x'),
            _ => {}
        }
    }
    // Dropping a character can leave the space that was holding it up. Tidied
    // line by line, and past the indent: the lines are the passage's shape and
    // the indent is a list's nesting, and neither is whitespace to tidy away.
    out.lines()
        .map(|line| {
            let indent = &line[..line.len() - line.trim_start().len()];
            let words: Vec<&str> = line.split_whitespace().collect();
            format!("{indent}{}", words.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A stand-in that does a few mechanical fixes without a model.
///
/// It exists so the whole interaction — the mark on the selection, the chat,
/// the diff, applying it — can be built, tested and screenshotted before a
/// gigabyte of weights is involved, and so the app still has an assistant when
/// built without one. It is not pretending to be intelligent: it says so in
/// its name, and the status bar prints the name.
#[derive(Default)]
pub struct Rehearsal;

impl Backend for Rehearsal {
    fn name(&self) -> String {
        "REHEARSAL".into()
    }

    fn edit(&mut self, ask: &Ask) -> Reply {
        // Line by line, so a selection keeps its shape, and indent by indent,
        // so a list item keeps its nesting.
        let out = ask
            .source
            .lines()
            .map(|line| {
                let indent = &line[..line.len() - line.trim_start().len()];
                let words: Vec<&str> = line
                    .split_whitespace()
                    .map(|w| match w {
                        "teh" => "the",
                        "adn" => "and",
                        "recieve" => "receive",
                        other => other,
                    })
                    .collect();
                format!("{indent}{}", words.join(" "))
            })
            .collect::<Vec<_>>()
            .join("\n");
        if out == ask.source {
            // Something has to differ or there is no diff to review, and a
            // rehearsal that silently does nothing teaches nothing.
            return Ok(format!("{out} ({})", ask.request.trim().to_lowercase()));
        }
        Ok(out)
    }
}
