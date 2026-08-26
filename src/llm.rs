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
        if !self.talking() {
            return editing.to_string();
        }
        let mut out = match self.tools.is_empty() {
            true => CHAT_PROMPT.to_string(),
            false => format!("{}\n\n{}", declare(&self.tools), CHAT_PROMPT),
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
    /// Whether the wait is a tool being run rather than the model writing.
    /// Somebody watching a panel should be told which of the two it is: one is
    /// the machine working and the other is the network.
    pub looking: bool,
    /// How many tools have been run for this one question.
    pub steps: u8,
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

/// The most tools one question may run before it has to answer.
///
/// A cap rather than a hope. A model that cannot find what it wants will search
/// for it again, and again, and there is somebody waiting at the end of it.
const STEPS: u8 = 5;

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

/// The tool call a reply is making, if it is making one.
///
/// The model's own format, which is why the parsing is this short: the tag and
/// one parameter, exactly as the chat template told it to write them.
pub fn called(reply: &str) -> Option<(String, String)> {
    let after = reply.split("<function=").nth(1)?;
    let name = after.split('>').next()?.trim().to_string();
    let param = after.split("<parameter=").nth(1)?;
    // Everything past the parameter's own `>`, up to its closing tag. Splitting
    // on `>` first would eat the `>` of `</parameter>` and leave the tag in the
    // value - which fed the tools an argument with a tag on the end, got an
    // empty result back, and had the model inventing to fill the gap.
    let value = param
        .split_once('>')?
        .1
        .split("</parameter>")
        .next()?
        .trim();
    (!name.is_empty() && !value.is_empty()).then(|| (name, value.to_string()))
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

/// Answer one question, running whatever tools it asks for on the way.
///
/// The loop lives here rather than in a backend, because it is not about
/// language models: it is about a question that took a few steps. The stub
/// answers in one and never notices there is a loop around it.
/// The worker's end of a question in flight: it reports, and it listens.
struct Watching<'a> {
    beat: &'a Sender<Progress>,
    words: &'a Sender<String>,
    stop: &'a std::sync::atomic::AtomicBool,
    /// Which tool call this is, which the backend knows nothing about.
    step: u8,
    /// Words sent so far, so a stream is not the same sentence over and over.
    sent: usize,
    at: Progress,
}

impl Watcher for Watching<'_> {
    fn tick(&mut self, at: Progress, said: &str) {
        self.at = Progress {
            steps: self.step,
            ..at
        };
        // A closed channel means the application has gone; the answer itself
        // will notice on the way out.
        let _ = self.beat.send(self.at);
        // Whole words only. A stream that redraws on every token spends more
        // of the machine on drawing half a word than on writing the next one,
        // and a word appearing is what reads as writing anyway.
        //
        // Counted rather than looked for at the end: what arrives here has
        // been through the tidying that takes the model's markers off, and
        // that trims, so the text never ends in the space that would have said
        // a word had finished.
        let words = said.split_whitespace().count();
        if words > self.sent {
            self.sent = words;
            let _ = self.words.send(said.to_string());
        }
    }

    fn carry_on(&self) -> bool {
        !self.stop.load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn answer(
    backend: &mut dyn Backend,
    ask: Ask,
    beat: &Sender<Progress>,
    words: &Sender<String>,
    stop: &std::sync::atomic::AtomicBool,
) -> Reply {
    let mut turns = ask.turns.clone();
    let mut used: Vec<Used> = Vec::new();
    let mut step = 0u8;
    loop {
        let mut watch = Watching {
            beat,
            words,
            stop,
            step,
            sent: 0,
            at: Progress {
                steps: step,
                ..Progress::default()
            },
        };
        let asked = Ask {
            turns: turns.clone(),
            ..ask.clone()
        };
        let said = to_ascii(&backend.edit(&asked, &mut watch)?);
        let at = watch.at;
        // Stopped: what it had got to is the answer. Half a paragraph you
        // asked it to stop writing is more use than nothing, and it is what
        // you were looking at when you pressed the button.
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            let mut out: String = used.iter().map(Used::written).collect();
            out.push_str(said.trim());
            return Ok(out);
        }

        let Some((tool, arg)) = called(&said).filter(|_| !ask.tools.is_empty()) else {
            // Whatever it looked up goes in front of what it said, so the
            // answer arrives with its working.
            let mut out: String = used.iter().map(Used::written).collect();
            out.push_str(&said);
            return Ok(out);
        };
        if step >= STEPS {
            return Ok(format!(
                "{}I looked several things up and did not get to an answer.",
                used.iter().map(Used::written).collect::<String>()
            ));
        }
        step += 1;
        let _ = beat.send(Progress {
            looking: true,
            steps: step,
            ..at
        });
        let result = crate::tools::run(&tool, &arg);
        // The call and its answer become two more turns, in the shapes the
        // model's own template expects: what it said, then a user turn holding
        // the response, which the template reads as a tool result rather than
        // as somebody asking something new.
        turns.push(Turn {
            mine: false,
            text: said,
        });
        turns.push(Turn {
            mine: true,
            text: format!("<tool_response>\n{result}\n</tool_response>"),
        });
        used.push(Used { tool, arg, result });
    }
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
                        let reply = answer(&mut *backend, ask, &beat, &said, &flag);
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
