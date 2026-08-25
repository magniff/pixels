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
                let reply = backend.edit(&ask);
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
