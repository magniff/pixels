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
            false => format!("{}\n\n{}", declare(&self.tools), prompt),
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
    // The call format is the one baked into the weights, word for word: the
    // model obeys this shape and argues with any other. The reminders that
    // followed it there are not - four paragraphs of them, saying twice over
    // what the example already shows.
    out.push_str(
        "\n</tools>\n\nIf you choose to call a function ONLY reply in the following format \
         with NO suffix:\n\n<tool_call>\n<function=example_function_name>\n         <parameter=example_parameter_1>\nvalue_1\n</parameter>\n</function>\n</tool_call>\n\n         Several calls at once means several such blocks. Reasoning may come before a call, \
         never after. If no function fits, answer normally and do not mention functions.",
    );
    out
}

/// What the model is told it is doing when it is being talked to rather than
/// asked to rewrite something.
///
/// Not a setting, unlike the editing prompt. That one is worth exposing because
/// how you want your prose handled is personal; this one only has to describe
/// the situation, and a situation is not a preference.
pub const CHAT_PROMPT: &str = "You are talking with somebody about their own \
notes, in the editor they keep them in. Notes are organised into projects; a project \
is a folder of files. You can see a line about every note in the vault, and the whole \
of every file in the project they are looking at, with the lines numbered.

Be direct and brief. This is a side panel, not an essay. No preamble.

To change the project - and only when asked to - write one block, at the top level, \
outside any code fence:

<edit file=\"{note}\" lines=\"12-14\">the text those lines become</edit>
<edit file=\"{note}\" after=\"14\">new lines to go in below line 14</edit>
<write file=\"{note}\">everything the file says from now on</write>
<delete file=\"old.md\"></delete>
<merge into=\"kept.md\" from=\"one.md, two.md\">what the one file says</merge>

Rules for them:

- The file is named without its folder. Leave the name off an edit to mean the file \
they are looking at.
- Only the project they are looking at can be changed. Any note in the vault can be \
read with the read tool; to change one in another project, say they should open it.
- Lines are inclusive and count from one, as in the margin. Write the replacement \
without the numbers.
- Change as few lines as the request needs. To add something, use after: the new \
lines go in below that line and nothing else moves, and after=\"0\" puts them at \
the top. Not the whole file: rewriting a file to make one addition means copying out \
every line you were not asked to touch, and what gets copied wrong is the part nobody \
was looking at.
- Use write to lay down a whole file, whether or not it is there yet. Use merge to \
fold files into one and take the rest away, in a single step - never a write plus \
deletes, because those are accepted separately and half a merge loses a note.
- One block per reply, with a sentence outside it saying what you changed.
- Nothing happens until they accept it. Propose the change; do not announce you have \
made it.
- Asked to change nothing, write no block.";

/// As much of a conversation as fits, counted from the newest turn back.
///
/// `measure` says how long a set of turns comes to, in whatever unit `limit`
/// is in; it is asked as few times as the answer allows. The oldest turns go
/// first, two at a time - a question and its answer - so what is left still
/// begins with somebody asking something, which is the only shape the chat
/// templates take. The newest turn is never let go of: a question that does
/// not fit on its own is a different problem, and the caller's.
///
/// Whatever is left is told that it is not the whole story, in a line on the
/// front of its first turn, so the model does not answer "you never asked
/// about that" about something it was asked three hours ago.
pub fn fitted(turns: &[Turn], limit: usize, measure: impl Fn(&[Turn]) -> usize) -> Vec<Turn> {
    if turns.is_empty() || measure(turns) <= limit {
        return turns.to_vec();
    }
    const LEFT_OUT: &str = "[Earlier turns of this conversation have been left out to make room.]";
    // The first turn that could open the kept part: a question, at the
    // earliest, and past the pairs let go of so far.
    let mut from = 0;
    loop {
        // Two at a time, then forward to the next thing somebody asked.
        from = (from + 2).min(turns.len() - 1);
        while from < turns.len() - 1 && !turns[from].mine {
            from += 1;
        }
        let mut kept = turns[from..].to_vec();
        if let Some(first) = kept.first_mut() {
            first.text = format!("{LEFT_OUT}\n\n{}", first.text);
        }
        if from >= turns.len() - 1 || measure(&kept) <= limit {
            return kept;
        }
    }
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
    // A change block dressed as a call is put right before anything is taken
    // away, or it would be taken away with the dressing. See `unfused`.
    let mended = unfused(said);
    let said: &str = &mended;
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
const OPENERS: &[&str] = &[
    "<tool_call",
    "<function=",
    "<|tool_call_start|>",
    "[TOOL_CALL]",
    "<read",
];

/// And what ends one.
const CLOSERS: &[&str] = &[
    "</tool_call>",
    "</function>",
    "<|tool_call_end|>",
    "[/TOOL_CALL]",
    "</read>",
];

/// Leftovers: a closing tag whose opening never came, and the parameter
/// wrapping that sits inside a call and sometimes outlives it.
const STRAYS: &[&str] = &[
    "</read>",
    "</tool_call>",
    "</function>",
    "</parameter>",
    "<|tool_call_end|>",
    "<|tool_call_start|>",
    "[/TOOL_CALL]",
    "[TOOL_CALL]",
];

/// A block whose attributes arrived as a call's parameters.
///
/// The third shape of the same confusion, and the one that turns up once the
/// model has both blocks and tools well in mind:
///
/// ```text
/// <write>
/// <parameter=file>
/// facts.md
/// </write>
/// <parameter=content>
/// what the file should say
/// ```
///
/// The name and the body are both there, wearing the wrong tags, and the
/// closing one has wandered into the middle. Read for what it plainly means -
/// a write of `facts.md` with that text - rather than thrown away, which is
/// what happened: the change was never offered and nothing was written.
fn with_parameters(reply: &str) -> String {
    let mut out = reply.to_string();
    for kind in ["edit", "write", "create", "delete", "merge"] {
        let open = format!("<{kind}>");
        if !out.contains(&open) || !out.contains("<parameter=file>") {
            continue;
        }
        let Some(named) = out
            .split("<parameter=file>")
            .nth(1)
            .and_then(|rest| rest.split('<').next())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let body = out
            .split("<parameter=content>")
            .nth(1)
            .map(|rest| rest.split("</parameter>").next().unwrap_or(rest).trim())
            .unwrap_or("")
            .to_string();
        // Everything from the opening tag onwards was the block; what came
        // before it is whatever the model said first.
        let before = out.split(&open).next().unwrap_or("").to_string();
        out = format!("{before}<{kind} file=\"{named}\">\n{body}\n</{kind}>");
    }
    out
}

/// Put right a change block written as if it were a tool call.
///
/// Applied before the machinery is taken off, and again as a reply is stored.
/// Before, because the scrub removes a whole call from opening tag to closing
/// one - and a change block wearing a call's tags is inside that span. Asked
/// to add two children to a note, the model looked both their birthdays up,
/// wrote the edit, wrapped it in a call, and the whole thing was deleted as
/// wiring: two correct lookups followed by "it looked that up but did not say
/// anything about it".
///
/// The two are told apart by nothing but their tags, and a model handed both in
/// one system prompt sometimes fuses them. Asked to make a file, Qwen3.5 wrote
///
/// ```text
/// <tool_call>
/// <function=write file="kettle.md">
/// the kettle is broken.
/// </write>
/// </tool_call>
/// ```
///
/// which is this app's own write block wearing a call's opening tag. The intent
/// is not in doubt - the closing tag says which block was meant - so it is read
/// as the block it plainly is rather than thrown away, which is what happened
/// before: the reply came out empty and the conversation showed a blank turn.
///
/// Only the four kinds that exist, and only when the matching close is there,
/// so a genuine call to a tool that happens to share a name is left alone.
pub fn unfused(reply: &str) -> std::borrow::Cow<'_, str> {
    if !reply.contains("<function=") {
        // The bare shape needs no unfusing, only its parameters read.
        if reply.contains("<parameter=file>") {
            return std::borrow::Cow::Owned(with_parameters(reply));
        }
        return std::borrow::Cow::Borrowed(reply);
    }
    let mut out = reply.to_string();
    let mut mended = false;
    for kind in ["edit", "write", "create", "delete", "merge"] {
        let opened = format!("<function={kind}");
        if !out.contains(&opened) {
            continue;
        }
        out = out.replace(&opened, &format!("<{kind}"));
        mended = true;
        // The close is fused in more than one way - `</write>` one time,
        // `</parameter></function>` the next - so whichever wrapper closes
        // first stands in for the one that was meant.
        if !out.contains(&format!("</{kind}>")) {
            for wrapper in ["</parameter>", "</function>", "</tool_call>"] {
                if out.contains(wrapper) {
                    out = out.replacen(wrapper, &format!("</{kind}>"), 1);
                    break;
                }
            }
        }
    }
    if !mended {
        return std::borrow::Cow::Borrowed(reply);
    }
    out = with_parameters(&out);
    for wrapper in ["<tool_call>", "</tool_call>", "</function>", "</parameter>"] {
        out = out.replace(wrapper, "");
    }
    std::borrow::Cow::Owned(out)
}

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
    let mut out: Vec<(String, String)> = reply
        .split("<function=")
        .skip(1)
        .filter_map(one_call)
        .collect();
    out.extend(asked_to_read(reply));
    out
}

/// Reading a note, asked for the way this application's own blocks are written.
///
/// Told a file had changed and given a tool to read it with, the model wrote
/// `<read file="bike.md"></read>` - not a tool call, but the shape of the edit
/// and write blocks it had just been taught three paragraphs earlier. Which is
/// a reasonable thing to conclude from that prompt. Nothing understood it, so
/// it went unanswered and the model fell back on what it already believed.
///
/// Answered in the shape it asked, it gets it right. So both shapes are heard.
///
/// And a fourth shape, once the model has blocks and tools both well in mind:
/// `<read>` with no name on it, the name as a call's parameter underneath,
/// and a call's closing tags. Asked to add a line to a note it had just made,
/// it wrote that to look at the note first - which was the right thing to
/// want - and nothing understood it, so the reply came out as nothing.
fn asked_to_read(reply: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = reply;
    while let Some(at) = rest.find("<read") {
        let after = &rest[at + 5..];
        let Some((head, tail)) = after.split_once('>') else {
            break;
        };
        // Only the tag itself: `<reader>` or `<ready` is a word, not a call.
        if !head.is_empty() && !head.starts_with(char::is_whitespace) {
            rest = tail;
            continue;
        }
        let quoted = head
            .split('"')
            .nth(1)
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let named = quoted.map(str::to_string).or_else(|| {
            // The name written as a parameter, up to the parameter's closing
            // tag or the next tag of any kind, whichever comes first.
            tail.split("<parameter=")
                .nth(1)
                .and_then(|p| p.split_once('>'))
                .map(|(_, v)| v.split('<').next().unwrap_or("").trim().to_string())
                .filter(|n| !n.is_empty())
        });
        if let Some(named) = named {
            out.push(("read".to_string(), named));
        }
        rest = tail;
    }
    out
}

/// Write something down for somebody watching, if anybody is.
///
/// `PIXUI_PROMPT=<file>` turns it on. Both halves go in: what the model was
/// given and what it wrote back. A file of questions without the answers is
/// half a transcript, and the half that was missing is the one that matters
/// when a question comes back empty - there was no way to tell whether the
/// model had written nothing or had written something nothing understood.
pub(crate) fn jotted(head: &str, body: &str) {
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

/// One call, from the text just after its `<function=`.
fn one_call(after: &str) -> Option<(String, String)> {
    let name = after.split('>').next()?.trim().to_string();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    // Only up to the end of this call: a reply making three of them must not
    // read the second one's argument as the first one's.
    let after = after.split("</function>").next().unwrap_or(after);
    // Everything past the parameter's own `>`, up to its closing tag. Splitting
    // on `>` first would eat the `>` of `</parameter>` and leave the tag in the
    // value - which fed the tools an argument with a tag on the end, got an
    // empty result back, and had the model inventing to fill the gap.
    //
    // A call with the argument left out is still a call. It used to be
    // nothing: not a call, so stripped off as machinery, so a reply that was
    // only `<function=calc></function>` came out as "it did not answer". Run
    // with nothing, the tool says what it wanted, and the model gets to write
    // the call again with it in - which is what it does.
    let value = after
        .split("<parameter=")
        .nth(1)
        .and_then(|p| p.split_once('>'))
        .and_then(|(_, v)| v.split("</parameter>").next())
        .map(str::trim)
        .unwrap_or("");
    Some((name, value.to_string()))
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
    // What moved since the project was written out goes in front of the
    // question, once, here - and not in the backend, where it went in front
    // of whichever turn happened to be last. A question that reaches for a
    // tool is asked again with the tool's answer as the last turn, and the
    // correction was moving with it: the model was shown "STOP, the files
    // below have changed" stapled to a tool response, and the question it had
    // been in front of a moment earlier had lost it. Every question of that
    // kind was also read from the cache twice over, because the turn it moved
    // out of no longer matched the turn that had been read.
    if let (Some(moved), Some(last)) = (&ask.since, turns.last_mut()) {
        if last.mine {
            last.text = format!("{moved}\n\n---\n\n{}", last.text);
        }
    }
    let ask = Ask { since: None, ..ask };
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
        jotted(&format!("REPLY ({} chars)", said.len()), &said);
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
            return finish(
                backend, &ask, turns, used, so_far, beat, words, stop, looping,
            );
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
            let result = crate::tools::run(&tool, &arg, &ask.file);
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

/// A reply with the deliberation taken off, and nothing else.
///
/// Reasoning models answer in channels: the deliberation first, the answer
/// last, both in the same stream. Only the last one was asked for. The thinking
/// is dropped rather than shown — an editor that pauses to explain itself for
/// four hundred words before touching your sentence is a worse editor, however
/// interesting the four hundred words are.
///
/// This is the whole of what a *conversation* gets tidied by. The rest of
/// [`clean_reply`] is for a passage handed back - fences and quotes it was not
/// asked to wrap the passage in - and applied to a reply in a conversation it
/// took a code block off the front of an answer that was code because somebody
/// had asked for code.
pub fn without_thinking(text: &str) -> String {
    let mut out = text.trim();
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
    out.trim().to_string()
}

/// Take the answer out of whatever a model wrapped it in.
///
/// Small models announce themselves — "Here is the proofread version:" — and
/// fence things they were not asked to fence. The prompt asks for the passage
/// between `<text>` and `</text>` precisely so this can be exact rather than a
/// guess about which opening sentence is a preamble and which is the text.
/// Everything else here is a fallback for a model that ignored that.
pub fn clean_reply(text: &str) -> String {
    let out = without_thinking(text);
    let mut out = out.as_str();

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
/// box. The common punctuation has an obvious ASCII spelling and gets it; the
/// decoration - emoji, arrows, the symbols - is dropped rather than drawn as a
/// box.
///
/// Letters are kept, whatever alphabet they are in. This used to drop them
/// with the emoji, and it is applied to the whole reply, change blocks and
/// all: a note with a name like Müller or a line of Cyrillic in it, copied by
/// the model into an edit, came back with those characters gone and was then
/// written to disk that way. The editor draws a letter it has no glyph for as
/// a question mark, which is what a person's own typing gets, and a question
/// mark on screen is a different thing from a letter missing from the file.
///
/// Applied to every backend's answer, because the limit belongs to the editor
/// rather than to whichever model happened to answer.
pub fn to_ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            c if c.is_ascii() || c.is_alphanumeric() => out.push(c),
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
