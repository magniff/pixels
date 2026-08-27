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
/// It was five, which is fewer than some perfectly ordinary questions need: a
/// table of the last ten Christmases is ten calls to the calendar before a word
/// can be written, and asking for one got "I looked several things up and did
/// not get to an answer" - having thrown away all five lookups on the way out.
/// Twelve covers that, and what happens at the ceiling matters more than where
/// the ceiling is: see `finish`.
///
/// A cap rather than a hope. A model that cannot find what it wants will search
/// for it again, and again, and there is somebody waiting at the end of it.
const STEPS: u8 = 12;

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

/// A reply with the calling-out taken out of it.
///
/// What the model writes when it reaches for a tool is a block of tags, and
/// nobody asked to see that. It matters most while the answer is being watched
/// as it is written: the first thing to arrive is the machinery, and a panel
/// that shows it is a panel showing somebody the inside of the thing they asked
/// a question of.
///
/// Anything said *before* the call is kept - "let me check" is worth reading -
/// and an unterminated block takes the rest with it, since a block half written
/// is a block still being written.
pub fn without_machinery(said: &str) -> String {
    let mut out = String::with_capacity(said.len());
    let mut rest = said;
    loop {
        // The earliest thing that starts a call, whichever spelling it is in.
        let Some((at, opener)) = OPENERS
            .iter()
            .filter_map(|o| rest.find(o).map(|i| (i, *o)))
            .min_by_key(|(i, _)| *i)
        else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at]);
        let after = &rest[at + opener.len()..];
        // Past the end of this call, if it has one. A block half written is a
        // block still being written, and what follows it is nothing: that is
        // the streaming case, where the tags arrive before the answer does.
        let Some(shut) = CLOSERS
            .iter()
            .filter_map(|c| after.find(c).map(|i| (i + c.len(), *c)))
            .min_by_key(|(i, _)| *i)
            .map(|(i, _)| i)
        else {
            break;
        };
        rest = &after[shut..];
    }
    // Whatever is left of the wrapping, for the spellings that arrive without
    // their opening or in an order nobody predicted. None of it is language.
    for stray in STRAYS {
        out = out.replace(stray, "");
    }
    out.trim().to_string()
}

/// What a model writes to begin reaching for a tool.
///
/// More than one spelling, because more than one family is spoken to here and
/// each was trained on its own. The list is what has actually been seen coming
/// out of a model in this application, not everything that exists.
const OPENERS: &[&str] = &["<tool_call", "<function=", "<|tool_call_start|>", "[TOOL_CALL]"];

/// And what ends one.
const CLOSERS: &[&str] = &["</tool_call>", "</function>", "<|tool_call_end|>", "[/TOOL_CALL]"];

/// Leftovers: a closing tag whose opening never came, and the parameter
/// wrapping that sits inside a call and sometimes outlives it.
const STRAYS: &[&str] = &[
    "</tool_call>",
    "</function>",
    "</parameter>",
    "<|tool_call_end|>",
    "<|tool_call_start|>",
    "[/TOOL_CALL]",
    "[TOOL_CALL]",
];

/// The tool call a reply is making, if it is making one.
///
/// The model's own format, which is why the parsing is this short: the tag and
/// one parameter, exactly as the chat template told it to write them.
pub fn called(reply: &str) -> Option<(String, String)> {
    calls(reply).into_iter().next()
}

/// Every call a reply is making, in the order it made them.
///
/// More than one, because models ask for more than one at a time: given three
/// things to find out, they write all three blocks in a single reply. Reading
/// only the first meant the rest were dropped on the floor - and since the
/// reply was then all machinery with the first call consumed, what the person
/// waiting saw was the raw tags of the calls that never ran.
pub fn calls(reply: &str) -> Vec<(String, String)> {
    reply
        .split("<function=")
        .skip(1)
        .filter_map(one_call)
        .collect()
}

/// One call, from the text just after its `<function=`.
fn one_call(after: &str) -> Option<(String, String)> {
    let name = after.split('>').next()?.trim().to_string();
    // Only up to the end of this call: a reply making three of them must not
    // read the second one's argument as the first one's.
    let after = after.split("</function>").next().unwrap_or(after);
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
        let said = without_machinery(said);
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
    // What it has said so far, across every pass.
    //
    // A question in three parts is answered in three passes, and each pass
    // writes its part and then reaches for the next tool. Only the last pass
    // used to survive: asked for a product, a weekday and a day count, the
    // conversation showed "3. 120 days until the next 25 December" and
    // nothing else. The first two answers had been written and thrown away.
    let mut so_far: Vec<String> = Vec::new();
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
            return Ok(assembled(&used, &so_far, &without_machinery(&said)));
        }

        let asked_for = if ask.tools.is_empty() {
            Vec::new()
        } else {
            calls(&said)
        };
        if asked_for.is_empty() {
            // Whatever it looked up goes in front of what it said, so the
            // answer arrives with its working. Anything that looked like a
            // call and was not one comes out here rather than being read as
            // prose: a half-written block is not something to show anybody.
            return Ok(assembled(&used, &so_far, &without_machinery(&said)));
        }
        // The ones worth running: not already answered, and not asked twice in
        // the same breath. Both happen, and the second one badly - asked for a
        // percentage of a lifetime, a model wrote the same sum forty-four
        // times in one reply, and every one of them ran, because the check for
        // going round in circles only ever looked at the first.
        let mut fresh: Vec<(String, String)> = Vec::new();
        for call in &asked_for {
            let seen = used.iter().any(|u| (&u.tool, &u.arg) == (&call.0, &call.1));
            if !seen && !fresh.contains(call) {
                fresh.push(call.clone());
            }
        }
        // Out of steps, or nothing left to ask that has not been asked. Either
        // way the thing to do is not to give up: it has been looking things up
        // all this time and the answers are sitting in the turns behind it.
        let looping = fresh.is_empty();
        if step >= STEPS || looping {
            keep(&mut so_far, &said);
            turns.push(Turn {
                mine: false,
                text: said,
            });
            return finish(backend, &ask, turns, used, so_far, beat, words, stop, looping);
        }
        // What it wrote before reaching for the tool is part of the answer.
        keep(&mut so_far, &said);
        let _ = beat.send(Progress {
            looking: true,
            steps: step.saturating_add(1),
            ..at
        });
        // Every call it made, not only the first, and no more of them than
        // there are steps left. The calls and their answers become two more
        // turns, in the shapes the model's own template expects: what it said,
        // then a user turn holding the responses, which the template reads as
        // tool results rather than as somebody asking something new.
        let mut answers = String::new();
        for (tool, arg) in fresh {
            if step >= STEPS {
                break;
            }
            let result = crate::tools::run(&tool, &arg);
            answers.push_str(&format!("<tool_response>\n{result}\n</tool_response>\n"));
            used.push(Used { tool, arg, result });
            step = step.saturating_add(1);
        }
        turns.push(Turn {
            mine: false,
            text: said,
        });
        turns.push(Turn {
            mine: true,
            text: answers.trim_end().to_string(),
        });
    }
}

/// Keep the part of a pass that is prose rather than machinery.
fn keep(so_far: &mut Vec<String>, said: &str) {
    let plain = without_machinery(said);
    // The same sentence twice is one sentence: a model that repeats its
    // working before each call would otherwise say everything n times.
    if !plain.is_empty() && !so_far.contains(&plain) {
        so_far.push(plain);
    }
}

/// Everything looked up, then everything said, in the order it was said.
///
/// `last` has already had the machinery taken off it - the caller does that,
/// because only the caller knows whether what looked like machinery was any.
fn assembled(used: &[Used], so_far: &[String], last: &str) -> String {
    let mut out: String = used.iter().map(Used::written).collect();
    let mut parts: Vec<&str> = so_far.iter().map(String::as_str).collect();
    let last = last.trim();
    if !last.is_empty() && !parts.contains(&last) {
        parts.push(last);
    }
    // A reply that was nothing but a call it got wrong leaves nothing behind
    // once the tags are off it, and a turn with nothing in it reads as the
    // application having lost the answer. Say what happened instead: the
    // lookups are shown above it either way, so this is the only line missing.
    if parts.is_empty() {
        parts.push(if used.is_empty() {
            "It did not answer."
        } else {
            "It looked that up but did not say anything about it."
        });
    }
    out.push_str(&parts.join("\n\n"));
    out
}

/// One more pass, with the tools taken away.
///
/// What used to happen at the ceiling was a sentence saying it had looked
/// several things up and got nowhere, with the lookups discarded - the worst of
/// both, since the answers it needed were already in front of it and the person
/// waiting got neither them nor a reply.
///
/// So it is asked once more without any tools to reach for, which leaves it
/// nothing to do but write. The results are still in the turns, so a table of
/// ten Christmases is written from the ten answers rather than abandoned on the
/// eleventh call.
#[allow(clippy::too_many_arguments)]
fn finish(
    backend: &mut dyn Backend,
    ask: &Ask,
    mut turns: Vec<Turn>,
    used: Vec<Used>,
    so_far: Vec<String>,
    beat: &Sender<Progress>,
    words: &Sender<String>,
    stop: &std::sync::atomic::AtomicBool,
    looping: bool,
) -> Reply {
    turns.push(Turn {
        mine: true,
        text: format!(
            "{} Answer now, from what you already have above, and do not look \
             anything else up - there is nothing left to look up with. If you were \
             asked to write or change a file, write the block for it now, with the \
             values you have. Plain words and the block, nothing else.",
            if looping {
                "You have already looked that up, and the answer is above."
            } else {
                "That is as much looking up as there is time for."
            }
        ),
    });
    let mut watch = Watching {
        beat,
        words,
        stop,
        step: STEPS,
        sent: 0,
        at: Progress::default(),
    };
    let said = to_ascii(&backend.edit(
        &Ask {
            turns,
            // Nothing to reach for, so the only thing left is the answer.
            tools: Vec::new(),
            ..ask.clone()
        },
        &mut watch,
    )?);
    // There were no tools to call on this pass, so anything shaped like a call
    // was not one - it was the model carrying on out of habit. What it wrote
    // is gone with the tags either way, and handing those back instead was
    // showing somebody the inside of the thing they asked a question of.
    // Better to say plainly that there is no answer, when there is none.
    let plain = without_machinery(&said);
    let ending = if plain.is_empty() && so_far.is_empty() {
        "I looked those up but could not put an answer together."
    } else {
        &plain
    };
    Ok(assembled(&used, &so_far, ending))
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
