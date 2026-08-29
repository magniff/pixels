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

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};

mod answer;
mod dialect;
mod reply;

pub use answer::fitted;
pub use dialect::{declare, Dialect, CHAT_PROMPT, THINK_FIRST};
pub use reply::{
    called, calls, clean_reply, shown, to_ascii, unfused, without_machinery, without_thinking,
    without_thoughts, THOUGHT_CLOSE, THOUGHT_OPEN,
};

use answer::answer;

/// One side of a conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// Whose turn it was. The person asking, or the model answering.
    pub mine: bool,
    pub text: String,
}

/// A request: the text to change, what to do to it, and what it is part of.
///
/// The last two are context rather than instruction. They are what makes the
/// difference between rewriting a sentence and rewriting a sentence *in a
/// note* - the model can see the heading above it, the paragraph after it, and
/// that a note about the same thing exists two files away.
///
/// A conversation is the same request with `turns` filled in instead of a
/// passage: one path through the backend, because the part that assembles the
/// context is the part worth having once.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub source: String,
    pub request: String,
    /// The conversation so far, ending with what was just asked. Empty for an
    /// edit, which is a question about a passage rather than a conversation.
    pub turns: Vec<Turn>,
    /// The note the passage was taken from, with the passage marked in place.
    /// None when the passage is the whole note and there is nothing around it.
    pub within: Option<String>,
    /// Which file that note is, so the vault line and the note agree.
    pub file: String,
    /// One line per note in the vault, the same for every request.
    pub vault: String,
    /// What the model may reach for. Empty means it is told about none, which
    /// is not the same as being told it has none: a tool it was never offered
    /// is one it cannot mention.
    pub tools: Vec<Tool>,
    /// What has moved in the project since it was written out in `within`.
    ///
    /// Goes at the end, just before the newest question, rather than being
    /// folded into the project at the front - which is the whole reason it
    /// exists. See [`crate::digest::since`]. Folded into that question once,
    /// by the answering loop, before any backend sees it; a backend is handed
    /// this as `None` and the question already carrying it.
    pub since: Option<String>,
    /// Whether looking things up is switched off rather than absent.
    ///
    /// The difference is worth telling the model, because the two produce the
    /// same answer otherwise - "I do not have access to that" - and one of them
    /// has a switch. A refusal that does not mention the switch is
    /// indistinguishable from a broken feature.
    pub web_off: bool,
}

impl Ask {
    /// Whether this is a conversation rather than an edit.
    pub fn talking(&self) -> bool {
        !self.turns.is_empty()
    }

    /// What the model is told it is doing.
    ///
    /// Assembled rather than looked up, because a conversation's system message
    /// has more than one thing in it: what the situation is, and - when there
    /// are any - what it can reach for. The tools go first, because that is
    /// where the model's own chat template puts them, and a prompt in the shape
    /// the model was trained on is obeyed and one in another shape is argued
    /// with.
    pub fn system(&self, editing: &str) -> String {
        self.system_in(editing, Dialect::Qwen)
    }

    /// The same, in the words this family of models uses for its tools.
    pub fn system_in(&self, editing: &str, dialect: Dialect) -> String {
        if !self.talking() {
            return editing.to_string();
        }
        // The examples are written about the note actually open. They used to
        // name `notes.md`, which was a fine name for an example right up until
        // a model copied it: asked to change a line of the note in front of
        // it, one wrote `<edit file="notes.md" lines="7">` for a project with
        // no notes.md in it, and nothing happened. It had every other part of
        // the answer right. A name in an example is a name a model will write,
        // so the one in the example is the one that works.
        let shown = self
            .file
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or("notes.md");
        let prompt = CHAT_PROMPT.replace("{note}", shown);
        let mut out = match self.tools.is_empty() {
            true => prompt,
            false => format!("{}\n\n{}", declare(&self.tools, dialect), prompt),
        };
        if self.web_off {
            out.push_str(
                "\n\nLooking things up on the web is switched off in this app's settings, and \
                 can be switched back on there or by typing /web. If a question needs the web - \
                 the weather, the news, what is on a page - say that it is switched off and that \
                 they can turn it on, rather than only that you cannot help.",
            );
        }
        out
    }
}

/// One thing the model can reach for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub name: &'static str,
    /// What it does and *when to use it*, which is the part that decides
    /// whether the model reaches for it at all: the same model asked the same
    /// four questions called a tool described as "search the web and return a
    /// list of results" once, and one described in terms of when it is needed
    /// four times out of four.
    pub about: &'static str,
    /// The one argument it takes: its name, and what to put in it.
    pub takes: (&'static str, &'static str),
}

/// What came back, or why nothing did.
pub type Reply = Result<String, String>;

/// How far along a question is, sent while it is being answered.
///
/// A model that takes twenty seconds and says nothing for nineteen of them is
/// indistinguishable from a model that has hung. These are the numbers the
/// worker already has — it is counting tokens anyway — passed up so the block
/// can show that something is happening and roughly how fast.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    /// Tokens the question came to, known once it has been read.
    pub prompt: usize,
    /// How many of those have been read so far. Reading a long question is the
    /// slowest part of answering it and the part with nothing to show, so it
    /// reports its own progress rather than leaving a panel to say "busy" for
    /// eight seconds and hope.
    pub read: usize,
    /// Tokens written back so far.
    pub written: usize,
    /// Since the question was sent, which is what somebody waiting is counting.
    pub elapsed: std::time::Duration,
    /// Of that, the part spent writing the tokens counted above. The rate is
    /// this rather than the whole wait: loading twelve gigabytes of weights is
    /// not slow generation, and averaging the two together tells you neither.
    pub generating: std::time::Duration,
    /// Whether those tokens are the model thinking rather than answering.
    pub deliberating: bool,
    /// Tokens spent thinking, kept once the thinking is over: what the
    /// deliberation cost is worth seeing beside what it produced.
    pub thought: usize,
    /// Whether the wait is a tool being run rather than the model writing.
    /// Somebody watching a panel should be told which of the two it is: one is
    /// the machine working and the other is the network.
    pub looking: bool,
    /// How many tools have been run for this one question.
    pub steps: u8,
    /// Whether the wait is the weights going in, which is not reading anything.
    pub loading: bool,
    /// Of `prompt`, how many tokens are actually being read this time. The
    /// rest were read for the last question and are still in the cache -
    /// which for a turn of a conversation is nearly all of it, so a panel that
    /// says "reading the notes" over a two-hundred-token tail is telling
    /// somebody their notes are being read when what is being read is their
    /// question. Zero until the read has started.
    pub fresh: usize,
}

impl Progress {
    /// Tokens a second, over the whole answer so far.
    pub fn rate(&self) -> f32 {
        let secs = self.generating.as_secs_f32();
        if secs <= 0.0 || self.written == 0 {
            return 0.0;
        }
        self.written as f32 / secs
    }
}

/// A call the model made and what came back.
///
/// Kept with the answer rather than reported separately: it goes into the
/// transcript, so what a conversation looked up is still visible tomorrow, and
/// an answer that came from somewhere can be told from one that did not.
pub struct Used {
    pub tool: String,
    pub arg: String,
    pub result: String,
}

impl Used {
    /// The block that carries it in a reply.
    pub fn written(&self) -> String {
        format!(
            "<used tool=\"{}\" arg=\"{}\">\n{}\n</used>\n\n",
            self.tool.replace('"', "'"),
            self.arg.replace('"', "'"),
            self.result
        )
    }
}

/// Write something down for somebody watching, if anybody is.
///
/// `PIXUI_PROMPT=<file>` turns it on. Both halves go in: what the model was
/// given and what it wrote back. A file of questions without the answers is
/// half a transcript, and the half that was missing is the one that matters
/// when a question comes back empty - there was no way to tell whether the
/// model had written nothing or had written something nothing understood.
pub fn jotted(head: &str, body: &str) {
    let Some(where_to) = std::env::var_os("PIXUI_PROMPT") else {
        return;
    };
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(where_to)
    {
        let _ = writeln!(f, "\n===== {head} =====\n{body}");
    }
}

/// Something that can rewrite a piece of text on request.
/// What a backend calls as it goes, and what it hears back.
///
/// The words as well as the numbers, because a counter that says four hundred
/// tokens is telling you the machine is busy and not what it is saying - and
/// twenty seconds of that is twenty seconds of having to trust it. The answer
/// back is whether to carry on: a question you no longer want the answer to
/// should stop costing you, and the only place that can be noticed is between
/// two tokens.
pub trait Watcher {
    /// How far along, and what has been said so far.
    fn tick(&mut self, at: Progress, said: &str);
    /// False once somebody has asked for this to stop.
    fn carry_on(&self) -> bool;
}

/// A watcher for a caller that is not watching.
///
/// A test, or anything that wants the answer and not the writing of it.
pub struct Quiet;

impl Watcher for Quiet {
    fn tick(&mut self, _at: Progress, _said: &str) {}
    fn carry_on(&self) -> bool {
        true
    }
}

pub trait Backend: Send + 'static {
    /// Shown in the status bar, so it is never a mystery which one answered.
    fn name(&self) -> String;
    /// Answer, telling `watch` as often as there is something new to report,
    /// and stopping when it says to.
    fn edit(&mut self, ask: &Ask, watch: &mut dyn Watcher) -> Reply;
    /// Let go of whatever was being held for speed. Called when nothing has
    /// been asked for a while; the next question loads it again.
    fn release(&mut self) {}
}

/// How long the weights stay resident after the last question.
///
/// They are held so that a second thought about the same paragraph answers
/// straight away. But a twenty-billion-parameter model is twelve gigabytes of
/// a machine that has twenty-four, and holding those for a whole afternoon
/// because somebody fixed a comma at eleven is not a trade anyone agreed to.
/// Long enough to keep a conversation warm; short enough that the machine gets
/// itself back.
const IDLE: std::time::Duration = std::time::Duration::from_secs(180);

/// A backend on a thread, with one question outstanding at a time.
pub struct Assistant {
    tx: Sender<Ask>,
    rx: Receiver<Reply>,
    ticks: Receiver<Progress>,
    /// What is being written, as it is written.
    words: Receiver<String>,
    /// Raised to ask the question in flight to give up. Shared with the worker
    /// rather than sent down the channel: a request to stop is no use behind a
    /// queue of the thing it is trying to stop.
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    name: String,
    busy: bool,
    gone: bool,
    /// The last thing the worker said about the question in flight.
    progress: Progress,
    /// The answer so far, while there is one.
    partial: String,
}

impl Assistant {
    pub fn spawn(mut backend: Box<dyn Backend>) -> Self {
        let name = backend.name();
        let (tx, asks) = std::sync::mpsc::channel::<Ask>();
        let (replies, rx) = std::sync::mpsc::channel::<Reply>();
        let (beat, ticks) = std::sync::mpsc::channel::<Progress>();
        let (said, words) = std::sync::mpsc::channel::<String>();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            // Ends when the application drops its end of the channel.
            let mut warm = false;
            loop {
                match asks.recv_timeout(IDLE) {
                    Ok(ask) => {
                        // One bad question must not take the assistant with
                        // it. This thread is the only one there is: when it
                        // died the channel went with it, every question after
                        // it was refused, and what the panel said about that
                        // was that it was still busy with the last one - which
                        // it was not, and could never become again.
                        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            answer(&mut *backend, ask, &beat, &said, &flag)
                        }))
                        .unwrap_or_else(|_| {
                            Err("the assistant fell over on that question. \
                                 It is still here - ask again."
                                .to_string())
                        });
                        warm = true;
                        if replies.send(reply).is_err() {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if warm {
                            backend.release();
                            warm = false;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self {
            tx,
            rx,
            ticks,
            words,
            stop,
            name,
            busy: false,
            gone: false,
            progress: Progress::default(),
            partial: String::new(),
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
        self.progress = Progress::default();
        self.busy = self.tx.send(ask).is_ok();
        self.gone = !self.busy;
        self.busy
    }

    /// Whether the thread that answers is no longer there.
    ///
    /// Only ever true after a question failed to reach it. The two ways a
    /// question can be turned away are not the same thing - one clears on its
    /// own and the other never does - and telling somebody the first when it
    /// is the second leaves them waiting for an answer that is not coming.
    pub fn gone(&self) -> bool {
        self.gone
    }

    /// Where the question in flight has got to. Drained to the latest, because
    /// the ones behind it are already out of date.
    pub fn progress(&mut self) -> Progress {
        while let Ok(p) = self.ticks.try_recv() {
            self.progress = p;
        }
        self.progress
    }

    /// The answer as far as it has got, for showing while it is being written.
    pub fn partial(&mut self) -> &str {
        while let Ok(said) = self.words.try_recv() {
            self.partial = said;
        }
        &self.partial
    }

    /// Ask the question in flight to give up.
    ///
    /// What it had got to comes back as the answer rather than nothing: half a
    /// paragraph you asked it to stop writing is more use than an empty panel,
    /// and it is what you were looking at when you pressed the button.
    pub fn stop(&mut self) {
        if self.busy {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Collect an answer if one has arrived. Never blocks.
    pub fn poll(&mut self) -> Option<Reply> {
        match self.rx.try_recv() {
            Ok(reply) => {
                self.busy = false;
                self.partial.clear();
                self.stop.store(false, std::sync::atomic::Ordering::Relaxed);
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

    fn edit(&mut self, ask: &Ask, watch: &mut dyn Watcher) -> Reply {
        // It answers instantly, so there is one report and it is the finished
        // one — enough for the block to have something true to draw.
        watch.tick(
            Progress {
                prompt: ask.source.split_whitespace().count(),
                ..Progress::default()
            },
            "",
        );
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
