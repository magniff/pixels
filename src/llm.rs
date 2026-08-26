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
        if !self.talking() {
            return editing.to_string();
        }
        match self.tools.is_empty() {
            true => CHAT_PROMPT.to_string(),
            false => format!("{}\n\n{}", declare(&self.tools), CHAT_PROMPT),
        }
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

/// The tools, written out the way the model's own chat template writes them.
///
/// Copied from the template baked into the weights rather than invented. The
/// template cannot be handed a tool list through the interface this app has -
/// that lives in a part of llama.cpp the bindings do not expose - but it merges
/// the tool block and the system message into one turn, so writing the block
/// out by hand renders exactly what passing tools would have.
pub fn declare(tools: &[Tool]) -> String {
    let mut out = String::from("# Tools\n\nYou have access to the following functions:\n\n<tools>");
    for tool in tools {
        out.push_str(&format!(
            "\n{{\"type\": \"function\", \"function\": {{\"name\": \"{}\", \"description\": \"{}\", \
             \"parameters\": {{\"type\": \"object\", \"properties\": {{\"{}\": {{\"type\": \"string\", \
             \"description\": \"{}\"}}}}, \"required\": [\"{}\"]}}}}}}",
            tool.name, tool.about, tool.takes.0, tool.takes.1, tool.takes.0
        ));
    }
    out.push_str(
        "\n</tools>\n\nIf you choose to call a function ONLY reply in the following format with NO \
         suffix:\n\n<tool_call>\n<function=example_function_name>\n<parameter=example_parameter_1>\n\
         value_1\n</parameter>\n</function>\n</tool_call>\n\n<IMPORTANT>\nReminder:\n- Function calls \
         MUST follow the specified format: an inner <function=...></function> block must be nested \
         within <tool_call></tool_call> XML tags\n- Required parameters MUST be specified\n- You may \
         provide optional reasoning for your function call in natural language BEFORE the function \
         call, but NOT after\n- If there is no function call available, answer the question like \
         normal with your current knowledge and do not tell the user about function calls\n</IMPORTANT>",
    );
    out
}

/// What the model is told it is doing when it is being talked to rather than
/// asked to rewrite something.
///
/// Not a setting, unlike the editing prompt. That one is worth exposing because
/// how you want your prose handled is personal; this one only has to describe
/// the situation, and a situation is not a preference.
pub const CHAT_PROMPT: &str = "You are talking with somebody about their own notes, \
inside the markdown editor they keep them in. Notes are organised into projects, and a \
project is a folder of files. You can see a one-line summary of every note in the vault, \
and the whole of every file in the project they are looking at, with the lines numbered. \
Answer about those when the question is about those, and answer normally when it is not. \
Be direct and brief - this is a conversation in a side panel, not an essay. Markdown \
where it helps, plain sentences where it does not, and no preamble.

When you are asked to change the project, and only then, propose the change by writing \
one block, at the top level and outside any code fence. There are three:

<edit file=\"notes.md\" lines=\"12-14\">
the text those lines should become
</edit>

<write file=\"notes.md\">
everything the file should contain from now on
</write>

<delete file=\"old-file.md\"></delete>

<merge into=\"kept.md\" from=\"one.md, two.md\">
what the one file should say, having read all of them
</merge>

Use edit to change a few lines of a file and write to lay down a whole one, whether or \
not it is there yet. Merge folds several files into one and takes the others away, in a \
single step - do not write one file and delete the others by hand, because those would \
be accepted separately and a merge half accepted loses a note. The file merged into may \
be one of the files merged; leave the body of a merge empty to have them joined end to \
end unchanged, and fill it in when you have something better to say than that. The file is named without its folder; leave the attribute off an \
edit to mean the file they are looking at, and name a file that was not in the list only \
when you mean to make it. Line numbers are \
inclusive, count from one, and are the numbers in the margin - write the replacement \
without them. An empty edit takes the lines out. One block per reply, and a sentence \
outside it saying what you changed. Nothing happens to any file until they accept it, so \
propose the change rather than announcing that you have made it. If you were not asked \
to change anything, do not write a block at all.";

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

/// Something that can rewrite a piece of text on request.
pub trait Backend: Send + 'static {
    /// Shown in the status bar, so it is never a mystery which one answered.
    fn name(&self) -> String;
    /// Answer, calling `tick` as often as there is something new to report.
    fn edit(&mut self, ask: &Ask, tick: &mut dyn FnMut(Progress)) -> Reply;
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
    name: String,
    busy: bool,
    /// The last thing the worker said about the question in flight.
    progress: Progress,
}

impl Assistant {
    pub fn spawn(mut backend: Box<dyn Backend>) -> Self {
        let name = backend.name();
        let (tx, asks) = std::sync::mpsc::channel::<Ask>();
        let (replies, rx) = std::sync::mpsc::channel::<Reply>();
        let (beat, ticks) = std::sync::mpsc::channel::<Progress>();
        std::thread::spawn(move || {
            // Ends when the application drops its end of the channel.
            let mut warm = false;
            loop {
                match asks.recv_timeout(IDLE) {
                    Ok(ask) => {
                        let mut tick = |p: Progress| {
                            // A closed channel means the application has gone;
                            // the answer itself will notice on the way out.
                            let _ = beat.send(p);
                        };
                        let reply = backend.edit(&ask, &mut tick).map(|text| to_ascii(&text));
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
            name,
            busy: false,
            progress: Progress::default(),
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
        self.busy
    }

    /// Where the question in flight has got to. Drained to the latest, because
    /// the ones behind it are already out of date.
    pub fn progress(&mut self) -> Progress {
        while let Ok(p) = self.ticks.try_recv() {
            self.progress = p;
        }
        self.progress
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

    // Reasoning models answer in channels: the deliberation first, the answer
    // last, both in the same stream. Only the last one was asked for. The
    // thinking is dropped rather than shown — an editor that pauses to explain
    // itself for four hundred words before touching your sentence is a worse
    // editor, however interesting the four hundred words are.
    if let Some(start) = out.rfind("<|channel|>final<|message|>") {
        out = &out[start + "<|channel|>final<|message|>".len()..];
    }
    for tail in ["<|return|>", "<|end|>", "<|call|>"] {
        if let Some(cut) = out.find(tail) {
            out = &out[..cut];
        }
    }
    if let Some(start) = out.rfind("</think>") {
        out = &out[start + "</think>".len()..];
    }
    out = out.trim();

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

    fn edit(&mut self, ask: &Ask, tick: &mut dyn FnMut(Progress)) -> Reply {
        // It answers instantly, so there is one report and it is the finished
        // one — enough for the block to have something true to draw.
        tick(Progress {
            prompt: ask.source.split_whitespace().count(),
            ..Progress::default()
        });
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
