//! A quantised model run on this machine, through llama.cpp.
//!
//! Loaded on the first question rather than at startup: opening a note should
//! not wait on a gigabyte of weights, and most sessions never ask anything.
//!
//! A fresh context per request, which costs a little memory churn and buys the
//! guarantee that one edit cannot see the last one's tokens. Editing is a
//! one-shot job — the chat above it is a conversation with the *editor*, not
//! with a model that needs to remember.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::context::session::{LlamaStateSeqFlags, SeqState};
use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

use super::{Ask, Backend, Reply};

/// Leave this much of the window for the answer, deliberation included.
const RESERVE: usize = 2048;

/// The most of the question to read in one go, in tokens.
///
/// Reading it is one call into llama.cpp that does not come back until it is
/// done, and on a Mac it is the GPU doing the reading - the same GPU the window
/// is drawn on. Metal runs a command buffer to completion, so however long one
/// of these takes is how long a frame can be made to wait.
///
/// Measured at the window rather than at the worker, which is where the
/// complaint is: a window drawing an animation at 120fps, with a real 4,835
/// token question read on a worker thread beside it.
///
/// | tokens per read | whole answer | frames a second while reading | worst frame |
/// | --- | --- | --- | --- |
/// | 2048 | 9.4 s | 13-32 | 417 ms |
/// | 256 | 9.4 s | 10-33 | 425 ms |
/// | 128 | 9.6 s | 50-87 | 200 ms |
/// | 64 | 14.4 s | 72-78 | 150 ms |
///
/// So 128: four times the frame rate for two percent of the time. 64 buys
/// another 50ms of worst case for half again as long to answer, which is the
/// wrong way round - and the worst frame stops improving there anyway, so
/// something other than this sets a floor around 150ms and going smaller only
/// pays the per-call overhead to find it.
///
/// Letting the GPU drain between mouthfuls was tried and does nothing: resting
/// 4ms and 10ms after each one left the worst frame at 187-205ms, unchanged,
/// for the time the resting cost. The stall is a frame arriving behind a
/// dispatch that has already started, and only a shorter dispatch helps.
const MOUTHFUL: usize = 128;

/// How far short of the end of a question to take a mark, in tokens.
///
/// Only for the models that need one. The end of a question is the chat
/// template's way of saying "your turn"; the next question says the same thing
/// with the answer written into it, and the two stop agreeing a few tokens
/// before the end. Measured on Qwen3.5: a 5,367-token question and the one
/// after it shared 5,363. So a mark at the very end is past the common part
/// and worth nothing, and this is the distance back from it - sixteen times
/// the gap that was actually there, because the cost of being wrong is reading
/// the whole thing again and the cost of being generous is 64 tokens.
const MARGIN: usize = 64;

/// How many marks to keep.
///
/// Each is fifty megabytes of host memory, so this is a small number by
/// necessity. Three is enough for the two cases that actually happen - the
/// next turn of this conversation, and a different question about the same
/// project - with one spare.
const MARKS: usize = 3;

/// The last prompt written to `PIXUI_PROMPT`, so the next one can be written as
/// what it added. Only ever touched when that variable is set.
static LAST_PROMPT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// How much of the front of two prompts is the same, in bytes, cut back to a
/// character boundary so what is written out is still a string.
fn shared_prefix_chars(before: &str, now: &str) -> usize {
    let mut same = before
        .as_bytes()
        .iter()
        .zip(now.as_bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while same > 0 && !now.is_char_boundary(same) {
        same -= 1;
    }
    same
}

/// Where the weights are, unless `PIXUI_MODEL` says otherwise.
pub fn default_path() -> PathBuf {
    std::env::var_os("PIXUI_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models/Qwen3.6-35B-A3B-UD-IQ3_XXS.gguf"))
}

/// The weights, and the working memory that borrows them.
///
/// Held together because the second is worth keeping. Reading a question is
/// most of what answering one costs, and almost all of any question is the same
/// as the last: the same system prompt, the same project, the same conversation
/// with one more turn on the end. What is already in the key/value cache does
/// not have to be read again, and the only way to have that is to keep the
/// context that owns it.
///
/// `new_context` borrows the model, which makes this a self-referential struct
/// and is why it needs a word of unsafe. What holds it up:
///
/// * the model and the backend are boxed, so their addresses do not change
///   when this struct moves;
/// * `ctx` is declared first, and Rust drops fields in declaration order, so
///   the borrow always ends before the thing it borrows;
/// * nothing hands `ctx` out - it is reached only through `&mut self` here.
///
/// The lifetime written down is a lie told once, in [`Self::open`], and the
/// three points above are what make it a safe one.
struct Held {
    ctx: LlamaContext<'static>,
    /// Exactly the tokens the cache holds, in the positions it holds them.
    ///
    /// Kept in step with the cache on every path, cancellation included: a
    /// question read against a cache that says it holds something it does not
    /// is answered from somebody else's sentence.
    seen: Vec<LlamaToken>,
    /// Points the sequence can be put back to, for a model that cannot be cut.
    ///
    /// Each is a position and llama.cpp's own snapshot of the running state
    /// there. More than one because a question can differ from the last in two
    /// quite different places. Another turn of the same conversation shares
    /// everything but its ending, and wants a mark near the end. A new question
    /// about the same project shares the project and nothing after it, and
    /// wants one much earlier - with only a late mark, that case measured 13.7
    /// seconds against 2.1, because the only mark it had sat past the point
    /// where the two questions parted.
    ///
    /// So they are kept spread out rather than clustered, and the one used is
    /// the furthest along that is still inside the shared part.
    marks: Vec<(usize, SeqState)>,
    /// Whether `trims` has been found out yet, rather than assumed.
    ///
    /// It is only learnt by trying, and trying means asking the cache to give
    /// up a tail it holds - which happens the first time a question shares
    /// less than everything with the last one, and not before. A conversation
    /// that only ever grows never asks. So until it is known a mark is taken
    /// on every question, a tenth of a second and fifty megabytes each, on a
    /// model that may turn out not to need one. The alternative is that the
    /// first question to part from the conversation is read from the start.
    probed: bool,
    /// Whether this model's cache can be cut back to a common prefix at all.
    ///
    /// Not every one can. A model that carries a recurrent state rather than a
    /// plain key/value cache has nothing to cut - the state after five
    /// thousand tokens does not contain the state after four thousand - and
    /// llama.cpp says so: "couldn't remove partial sequence". Qwen3.5 answers
    /// that way and Qwen3 does not. Asked once and then remembered, because it
    /// is a property of the architecture and will not have changed by the next
    /// turn.
    trims: bool,
    /// How much room the context was made with. A question that needs more
    /// gets a new one, and the cache starts again.
    room: u32,
    model: Box<LlamaModel>,
    backend: Box<LlamaBackend>,
}

/// The most room a context is ever given, in tokens.
///
/// The model's own ceiling is a quarter of a million tokens, and a context is
/// paid for in memory whether or not it is used - so growing towards that
/// number by doubling, which is what happens to a conversation that goes on,
/// ends at a context the machine cannot hold. Thirty-two thousand tokens is
/// a hundred kilobytes of conversation and about six hundred megabytes of
/// cache on the model this ships against, and a conversation longer than that
/// is not read whole: see `fitted`, which lets the oldest turns go.
const MOST: u32 = 32_768;

/// How much room the first context gets, before any question has been seen.
///
/// Big enough that an ordinary question does not immediately outgrow it and
/// small enough to be worth nothing if one does. The key/value cache is paid
/// for in memory whether or not it is used.
const START: u32 = 8192;

// SAFETY: llama.cpp's context is not safe to use from two threads at once, and
// this one never is. `Local` is moved to the worker thread when the assistant
// is spawned, at which point there is no context - it is built, used and
// dropped there and nowhere else. What the compiler cannot see is that the
// move happens before the thing that is not `Send` exists.
unsafe impl Send for Held {}

impl Held {
    fn params(room: u32) -> LlamaContextParams {
        LlamaContextParams::default().with_n_ctx(NonZeroU32::new(room))
    }

    fn open(backend: LlamaBackend, model: LlamaModel, room: u32) -> Result<Self, String> {
        let backend = Box::new(backend);
        let model = Box::new(model);
        let ctx = model
            .new_context(&backend, Self::params(room))
            .map_err(|e| format!("context: {e}"))?;
        // SAFETY: see the type's own comment. The borrow is of `*model`, which
        // is boxed and outlives `ctx` by declaration order.
        let ctx: LlamaContext<'static> = unsafe { std::mem::transmute(ctx) };
        Ok(Self {
            ctx,
            seen: Vec::new(),
            marks: Vec::new(),
            probed: false,
            trims: true,
            room,
            model,
            backend,
        })
    }

    /// How much of `wanted` the cache already holds, and drop the rest of it.
    ///
    /// One short of the whole thing at most. The last token of a question is
    /// the one its first word is sampled from, so it has to go through a decode
    /// to have logits - a question asked twice still costs one token, not none.
    fn keep_prefix(&mut self, wanted: &[LlamaToken]) -> usize {
        let common = shared_prefix(&self.seen, wanted);
        if self.trims {
            if common == self.seen.len() {
                // The cache is the front of this question already.
                return common;
            }
            // Everything past the common part is somebody else's
            // conversation. Trying to drop it is also the only way to find out
            // whether this model lets anybody do that.
            self.probed = true;
            if self
                .ctx
                .kv_cache_seq_rm(0, Some(common as u32), None)
                .is_ok()
            {
                self.seen.truncate(common);
                return common;
            }
            // It does not. Which is learnt here, on the second question a
            // conversation asks - and the mark taken during the first one is
            // exactly what is needed now, so this must fall through to it
            // rather than throw it away and start again.
            self.trims = false;
        }
        self.rewind(common)
    }

    /// Put the sequence back to the best mark inside what is shared.
    ///
    /// For the models that cannot be cut. Restoring the running state is what
    /// makes the cut legal: the same `kv_cache_seq_rm` that answers "couldn't
    /// remove partial sequence" against a live sequence answers `Ok` the
    /// moment the state behind it has been put back to where that position
    /// was. Measured exact - after a rewind and a re-read, the same top token
    /// and not one logit different.
    fn rewind(&mut self, common: usize) -> usize {
        let Some(best) = self.marks.iter().rposition(|(at, _)| *at <= common) else {
            self.forget();
            return 0;
        };
        // Everything past the one being used describes a continuation that is
        // about to stop being true.
        self.marks.truncate(best + 1);
        let (at, state) = self.marks.last().expect("found above");
        let at = *at;
        if self.ctx.state_seq_set(state, 0).is_err()
            || self.ctx.kv_cache_seq_rm(0, Some(at as u32), None).is_err()
        {
            self.forget();
            return 0;
        }
        self.seen.truncate(at);
        at
    }

    /// Give up on what is remembered, after anything that leaves the cache and
    /// `seen` possibly disagreeing.
    fn forget(&mut self) {
        self.ctx.clear_kv_cache();
        self.seen.clear();
        self.marks.clear();
    }

    /// The same weights with a bigger context around them.
    ///
    /// A context cannot be grown, so a question that needs more room than the
    /// last one gets a new one and the cache starts empty. What is *not* done
    /// again is loading the weights, which is the part that takes seconds and
    /// gigabytes. Taken by value so the old context is dropped, by name,
    /// before the model it borrows is looked at again.
    ///
    /// A bigger context that will not open is not the end of the weights. It
    /// used to be: the old context was gone by then, the failure took the
    /// model with it on the way out, and the next question loaded thirteen
    /// gigabytes again to find out that it would not fit either. The old size
    /// is reopened instead, and the caller sees the room did not grow.
    fn regrown(self, room: u32) -> Result<Self, String> {
        let Self {
            ctx,
            model,
            backend,
            trims,
            probed,
            room: had,
            ..
        } = self;
        drop(ctx);
        let (ctx, room) = match model.new_context(&backend, Self::params(room)) {
            Ok(ctx) => (ctx, room),
            Err(_) => (
                model
                    .new_context(&backend, Self::params(had))
                    .map_err(|e| format!("context: {e}"))?,
                had,
            ),
        };
        // SAFETY: as in `open` - same boxes, same drop order, same invariant.
        let ctx: LlamaContext<'static> = unsafe { std::mem::transmute(ctx) };
        Ok(Self {
            ctx,
            seen: Vec::new(),
            marks: Vec::new(),
            probed,
            trims,
            room,
            model,
            backend,
        })
    }
}

/// Keep this point, and keep the kept ones spread out.
///
/// A mark is fifty megabytes and a tenth of a second, so there is a small
/// number of them and the question is which to let go of. The one dropped is
/// whichever sits closest to its neighbour: two marks a few tokens apart
/// answer the same question twice and leave the rest of the sequence with
/// nothing. It is also what keeps the useful one. The first mark of a
/// conversation lands just before the project the question is about, which is
/// exactly where every later question about that project stops matching - and
/// because everything after it arrives in a cluster at the far end, the gap in
/// front of it is the biggest there is and it is the last thing dropped.
fn remember(marks: &mut Vec<(usize, SeqState)>, at: usize, state: SeqState) {
    marks.retain(|(p, _)| *p != at);
    marks.push((at, state));
    marks.sort_by_key(|(p, _)| *p);
    while marks.len() > MARKS {
        let mut crowded = 1;
        let mut closest = usize::MAX;
        for i in 1..marks.len() {
            let gap = marks[i].0 - marks[i - 1].0;
            if gap < closest {
                closest = gap;
                crowded = i;
            }
        }
        marks.remove(crowded);
    }
}

/// How much of the front of `wanted` is already `seen`, one short at most.
///
/// The last token of a question is the one its first word is sampled from, and
/// a token only has logits if it has been through a decode. So the same
/// question asked twice costs one token rather than none, and - the case that
/// matters - a question that is entirely a prefix of what is remembered still
/// ends on a token that has been decoded this time round.
fn shared_prefix<T: PartialEq>(seen: &[T], wanted: &[T]) -> usize {
    seen.iter()
        .zip(wanted)
        .take(wanted.len().saturating_sub(1))
        .take_while(|(a, b)| a == b)
        .count()
}

pub struct Local {
    path: PathBuf,
    /// What the model is told it is doing, from the settings.
    system: String,
    loaded: Option<Held>,
    /// Whether this model reasons out loud unless told the thinking is done.
    thinks: bool,
}

impl Local {
    pub fn new(path: impl AsRef<Path>, system: String) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            system,
            loaded: None,
            thinks: false,
        }
    }

    /// llama.cpp reports every refusal the same way — a null model and no
    /// reason — so the obvious causes are checked here rather than passed on in
    /// a vocabulary nobody outside llama.cpp shares.
    fn why_it_would_not_load(&self, e: impl std::fmt::Display) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.display().to_string());
        match std::fs::metadata(&self.path) {
            Err(_) => format!("{name} is not there"),
            // A GGUF file says so in its first four bytes. Anything else is an
            // HTML error page a download saved, or a file cut off before it got
            // that far.
            Ok(meta) if !starts_with_gguf(&self.path) => {
                format!("{name} is not a gguf file - it is {} bytes of something else", meta.len())
            }
            Ok(meta) => format!(
                "could not load {name} ({} MB on disk) - if the download was interrupted it will be short: {e}",
                meta.len() / 1_000_000
            ),
        }
    }

    fn load(&mut self) -> Result<(), String> {
        if self.loaded.is_none() {
            // llama.cpp narrates its whole startup to stderr. Interesting once,
            // and then it is a hundred lines between you and any message the
            // editor actually wanted to print.
            // ...unless something has gone wrong and the narration is the only
            // account of it there is, which `PIXUI_LLAMA_LOGS` asks for.
            if std::env::var_os("PIXUI_LLAMA_LOGS").is_none() {
                llama_cpp_2::send_logs_to_tracing(
                    llama_cpp_2::LogOptions::default().with_logs_enabled(false),
                );
            }
            let backend = LlamaBackend::init().map_err(|e| format!("llama backend: {e}"))?;
            // Everything on the GPU where there is one. llama.cpp ignores this
            // on a build without one, so it needs no platform test here.
            let params = LlamaModelParams::default().with_n_gpu_layers(99);
            let model = LlamaModel::load_from_file(&backend, &self.path, &params)
                .map_err(|e| self.why_it_would_not_load(e))?;
            // Asked of the model rather than guessed from its filename: the
            // instruct-tuned builds share a tokeniser with the thinking ones,
            // and handing one an empty `<think>` block it was never trained on
            // is how a perfectly good model answers with nothing at all.
            self.thinks = model
                .chat_template(None)
                .ok()
                .and_then(|t| t.to_string().ok())
                .is_some_and(|t| t.contains("<think>"));
            leave_before_ggml_does();
            self.loaded = Some(Held::open(backend, model, START)?);
        }
        Ok(())
    }
}

/// What a failed decode nearly always means here.
///
/// llama.cpp reports a graph that would not compute as `-3`, and on a Mac the
/// reason is almost always the same one: the GPU has a working set of about
/// two thirds of the machine's memory, and a twelve-gigabyte model shares it
/// with everything else already drawing on screen. The number is llama.cpp's;
/// the sentence is for whoever is sitting in front of it.
fn no_room(e: llama_cpp_2::DecodeError) -> String {
    match e {
        llama_cpp_2::DecodeError::Unknown(-3) => {
            "the gpu ran out of room - close what else is using it, or choose smaller weights"
                .to_string()
        }
        llama_cpp_2::DecodeError::NoKvCacheSlot => {
            "there was no room in the cache for that selection".to_string()
        }
        other => format!("decode: {other}"),
    }
}

/// Whether the file begins the way every gguf file begins.
fn starts_with_gguf(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && &magic == b"GGUF"
}

/// The conversation, rendered by the model's own chat template.
///
/// Not a format written out here: every family spells its turns differently —
/// ChatML, Harmony, Llama's headers — and a hand-written one works for exactly
/// the model it was written for. The template ships inside the GGUF, so the
/// model is asked how it wants to be spoken to.
///
/// The empty `<think>` block is still appended for the models that reason out
/// loud, because a template has no way to be told the thinking is already
/// done, and several hundred tokens of deliberation about a comma is not what
/// anybody asked for.
fn render(model: &LlamaModel, system: &str, ask: &Ask, thinks: bool) -> Result<String, String> {
    let template = model
        .chat_template(None)
        .map_err(|e| format!("this model has no chat template: {e}"))?;
    let mut chat = vec![
        LlamaChatMessage::new("system".into(), system.trim().to_string())
            .map_err(|e| format!("system turn: {e}"))?,
    ];
    let said = |role: &str, text: String, chat: &mut Vec<LlamaChatMessage>| {
        LlamaChatMessage::new(role.to_string(), text)
            .map(|m| chat.push(m))
            .map_err(|e| format!("{role} turn: {e}"))
    };
    if ask.talking() {
        // The context is fixed to the front of the first thing asked rather
        // than stored with the conversation: a chat resumed a week later is
        // then told about the vault as it is now, not as it was when it began.
        let context = surroundings(ask);
        for (i, turn) in ask.turns.iter().enumerate() {
            let role = if turn.mine { "user" } else { "assistant" };
            let text = if i == 0 {
                format!("{context}{}", turn.text)
            } else {
                turn.text.clone()
            };
            said(role, text, &mut chat)?;
        }
    } else {
        said("user", instruction(ask), &mut chat)?;
    }
    let mut out = model
        .apply_chat_template(&template, &chat, true)
        .map_err(|e| format!("applying the chat template: {e}"))?;
    // Harmony carries the reasoning budget in the rendered prompt, and a
    // template cannot be told to ask for less. Rewriting a comma is not worth
    // a page of deliberation the user waits through and never sees.
    out = out.replace("Reasoning: medium", "Reasoning: low");
    if thinks && !out.contains("</think>") {
        out.push_str("<think>\n\n</think>\n\n");
    }
    Ok(out)
}

/// What to ask the model to do, which is the same whichever model it is.
///
/// The surroundings go in the user turn rather than in the system prompt, and
/// that is on purpose: the system prompt is a setting, and a setting somebody
/// edited two months ago would not know to mention any of this. Everything the
/// context needs to explain itself travels with the context.
///
/// The order is deliberate too. What never changes goes first - the vault - and
/// what changes every time goes last, so the prompt is a stable prefix with a
/// short tail, which is the shape a key/value cache can eventually be kept
/// across.
fn instruction(ask: &Ask) -> String {
    let mut out = surroundings(ask);
    out.push_str(&format!(
        "Text:\n{}\n\nInstruction: {}",
        ask.source,
        ask.request.trim()
    ));
    if ask.within.is_some() || !ask.vault.is_empty() {
        // Both halves, because either one alone is wrong. Without the first
        // the model has been handed a note and told only what it may not do
        // with it, and it answers as if the note were not there. Without the
        // second it rewrites the note, which is the more interesting target.
        out.push_str(
            "\n\nEverything above the instruction is context. Use it: the names, numbers \
             and facts in the note and in the vault list are there to be drawn on, and \
             the passage should read as part of what surrounds it. But rewrite only the \
             passage under Text - do not rewrite or repeat the rest of the note, and do \
             not include the markers.",
        );
    }
    out
}

/// The vault and the note, which both kinds of question start with.
///
/// What never changes goes first - the vault - and what changes every time
/// goes after it, so the prompt is a stable prefix with a short tail, which is
/// the shape a key/value cache can eventually be kept across.
fn surroundings(ask: &Ask) -> String {
    let mut out = String::new();
    if !ask.vault.is_empty() {
        out.push_str("These are the notes in this vault, one line each:\n\n");
        out.push_str(&ask.vault);
        out.push_str("\n\n");
    }
    if let Some(within) = &ask.within {
        let named = if ask.file.is_empty() {
            "the note open in the editor".to_string()
        } else {
            format!("`{}`, the note open in the editor", ask.file)
        };
        if ask.talking() {
            out.push_str(&format!(
                "You are looking at {named}. Here is every file in its project, \
                 with the lines numbered:\n\n{within}\n"
            ));
        } else {
            out.push_str(&format!(
                "Here is {named}, with the passage in question marked between {} and {}:\n\n{within}\n\n",
                crate::digest::OPEN,
                crate::digest::CLOSE,
            ));
        }
    }
    if ask.talking() && !out.is_empty() {
        out.push_str(
            "That is context for what follows. Draw on it when the question is about \
             these notes, and set it aside when the question is not.\n\n---\n\n",
        );
    }
    out
}

/// Arrange to leave without running the rest of the C++ teardown.
///
/// llama.cpp's Metal device asserts on the way out that every buffer it handed
/// out has been freed. Nothing frees them: the model is loaded once and lives
/// on its worker thread for the rest of the process, and a process does not
/// unwind on the way to `exit` — least of all when the quit came from the macOS
/// menu, which calls `exit` from inside AppKit and runs no Rust destructor at
/// all. So an ordinary quit ended in an abort and a page of backtrace.
///
/// Registered immediately after the model loads, which is after ggml's own
/// teardown was registered — and handlers run in reverse, so this one runs
/// first and takes the process down before the assert can fire. Everything it
/// skips is memory the kernel reclaims a microsecond later anyway.
///
/// It would be better not to need this. If llama.cpp ever tolerates being torn
/// down with buffers outstanding, delete it and let exit take its course.
fn leave_before_ggml_does() {
    extern "C" {
        fn atexit(handler: extern "C" fn()) -> i32;
        fn _exit(code: i32) -> !;
    }
    extern "C" fn leave() {
        // With the status the process meant to leave with, not a zero of this
        // handler's own: `_exit` does not know what `exit` was called with.
        let code = crate::EXIT_CODE.load(std::sync::atomic::Ordering::SeqCst);
        unsafe { _exit(code) }
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        atexit(leave);
    });
}

impl Backend for Local {
    fn name(&self) -> String {
        self.path
            .file_stem()
            .map(|s| s.to_string_lossy().to_uppercase())
            .unwrap_or_else(|| "LOCAL MODEL".into())
    }

    /// Put the weights down. Twelve gigabytes is not a cache.
    fn release(&mut self) {
        self.loaded = None;
    }

    fn edit(&mut self, ask: &Ask, watch: &mut dyn super::Watcher) -> Reply {
        let out = self.attempt(ask, watch);
        if out.is_err() {
            // A decode that failed leaves the cache holding something nobody
            // can describe, and a record of it that says otherwise is worse
            // than no record: the next question would be read against it and
            // answered out of somebody else's sentence. So a question that
            // went wrong costs the one after it a full read, and nothing else.
            if let Some(held) = self.loaded.as_mut() {
                held.forget();
            }
        }
        out
    }
}

impl Local {
    fn attempt(&mut self, ask: &Ask, watch: &mut dyn super::Watcher) -> Reply {
        let started = std::time::Instant::now();
        self.load()?;
        let thinks = self.thinks;
        // A conversation is not an edit, and the editing prompt - which is all
        // about handing a passage back and nothing else - makes a poor one for
        // it. The setting stays what it says on the tin; the question says what
        // else belongs in front of it.
        let system = ask.system(&self.system);
        let held = self.loaded.as_ref().expect("loaded above");
        // How far this model was trained to read. Asked of the model rather
        // than set by hand: it is the model's own number, the same on every
        // machine, and nobody sitting in front of a note editor is in a
        // position to pick a better one.
        let ceiling = held.model.n_ctx_train().min(MOST);
        // A conversation that has outgrown the room is read from its newest
        // turns, not refused. An edit is one passage and either fits or does
        // not; a conversation is as long as it has gone on, and the oldest
        // of it is what the model can best do without.
        let fitted;
        let ask = if ask.talking() {
            let (model, limit) = (&held.model, ceiling as usize - RESERVE);
            let turns = super::fitted(&ask.turns, limit, |turns| {
                let asked = Ask {
                    turns: turns.to_vec(),
                    ..ask.clone()
                };
                render(model, &system, &asked, thinks)
                    .and_then(|t| {
                        model
                            .str_to_token(&t, AddBos::Never)
                            .map_err(|e| format!("tokenising: {e}"))
                    })
                    .map(|t| t.len())
                    .unwrap_or(usize::MAX)
            });
            fitted = Ask {
                turns,
                ..ask.clone()
            };
            &fitted
        } else {
            ask
        };
        let text = render(&held.model, &system, ask, thinks)?;
        // `PIXUI_PROMPT=<file>` writes out what the model is given, every time
        // it is given anything. For when the application and the answer
        // disagree about what the notes say and there is no telling, from the
        // outside, which of them is wrong.
        //
        // Written once in full and after that only where it differs from the
        // time before. A prompt is the whole conversation, so the one after it
        // is the same thing with a turn on the end: writing each out entire
        // meant four copies of one conversation to read four turns of it, and
        // the part worth reading - what this turn added - was the part that
        // took searching for.
        if let Some(where_to) = std::env::var_os("PIXUI_PROMPT") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(where_to)
            {
                let last = LAST_PROMPT.lock().ok().map(|l| l.clone());
                let same = last
                    .as_deref()
                    .map(|before| shared_prefix_chars(before, &text))
                    .unwrap_or(0);
                let _ = if same > 0 {
                    writeln!(
                        f,
                        "\n===== PROMPT ({} chars, first {same} the same as the last one) =====\n{}",
                        text.len(),
                        &text[same..]
                    )
                } else {
                    writeln!(f, "\n===== PROMPT ({} chars) =====\n{text}", text.len())
                };
                if let Ok(mut l) = LAST_PROMPT.lock() {
                    l.clone_from(&text);
                }
            }
        }
        let tokens = held
            .model
            .str_to_token(&text, AddBos::Never)
            .map_err(|e| format!("tokenising: {e}"))?;
        if tokens.len() + RESERVE > ceiling as usize {
            return Err(format!(
                "that selection is longer than this model can read - {} tokens against {}",
                tokens.len(),
                ceiling
            ));
        }

        // The window is sized to this request rather than to the ceiling. The
        // key/value cache is allocated with the context and paid for in memory
        // whether or not it is used, so a one-line edit should not reserve room
        // for a whole note - and the ceiling being the model's whole training
        // length costs nothing until something actually asks for it.
        let want = ((tokens.len() + RESERVE) as u32)
            .next_power_of_two()
            .clamp(1024, ceiling);
        if held.room < want {
            let held = self.loaded.take().expect("loaded above");
            self.loaded = Some(held.regrown(want)?);
        }
        let held = self.loaded.as_mut().expect("loaded above");
        if held.room < want {
            return Err(format!(
                "there is not enough memory for a question this long - {} tokens",
                tokens.len()
            ));
        }

        // What of this question the last one already read. Almost all of it,
        // for a conversation: the same system prompt, the same project, the
        // same turns with one more on the end. Only the tail is new, and only
        // the tail is read.
        let already = held.keep_prefix(&tokens);
        let (trims, probed) = (held.trims, held.probed);
        let Held {
            ctx,
            model,
            seen,
            marks,
            ..
        } = held;

        // The question is read a batch at a time rather than all at once.
        // llama.cpp asserts that a batch fits inside `n_batch` and *aborts the
        // process* when it does not - not an error, a SIGABRT - so a selection
        // of more than a couple of thousand tokens used to take the editor
        // down with it. The ceiling is asked of the context rather than
        // guessed: it is the same number the assertion is against, so it
        // cannot drift out of step with whatever llama.cpp defaults to next.
        let per = MOUTHFUL.min(ctx.n_batch() as usize).max(1);
        let mut batch = LlamaBatch::new(per, 1);
        let last = tokens.len() - 1;
        let mut at = already;
        // Where to stop and take a mark, for a model that needs one. Short of
        // the end on purpose: what the next question shares with this one runs
        // out a few tokens before the end of it, because the tail is the
        // template's way of saying "your turn" and the next question spells
        // that same moment differently. A mark taken at the very end would sit
        // past the common part and be no use. See MARGIN.
        let stop = (!trims || !probed)
            .then(|| tokens.len().saturating_sub(MARGIN).max(already))
            .filter(|stop| *stop > already);
        let mut spans = Vec::new();
        let mut from = already;
        for edge in [stop.unwrap_or(tokens.len()), tokens.len()] {
            while from < edge {
                let to = (from + per).min(edge);
                spans.push((from, to));
                from = to;
            }
        }
        for (open, close) in spans {
            let chunk = &tokens[open..close];
            // A stop asked for while the question is still being read is a
            // stop: there is nothing written yet to hand back, and the reply
            // below is the one an empty answer already produces.
            if !watch.carry_on() {
                break;
            }
            batch.clear();
            for token in chunk {
                // Only the very last token of the whole question is asked for
                // its logits: that is the one the first word is sampled from.
                batch
                    .add(*token, at as i32, &[0], at == last)
                    .map_err(|e| format!("batching: {e}"))?;
                at += 1;
            }
            ctx.decode(&mut batch).map_err(no_room)?;
            // The cache holds what has just gone into it, and says so before
            // anything can go wrong further down: an account that runs ahead
            // of the truth is worse than no account, because the next question
            // is answered against it.
            seen.extend_from_slice(chunk);
            if stop == Some(close) {
                // Fifty megabytes and a tenth of a second, once a question.
                // Against reading five thousand tokens again, which is nine
                // seconds.
                if let Ok(state) = ctx.state_seq_get(0, LlamaStateSeqFlags::PARTIAL_ONLY) {
                    remember(marks, seen.len(), state);
                }
            }
            // Say how far in it has got. Before this the first word out of the
            // worker was the first word of the answer, so the whole of reading
            // a long question - eight seconds of it, on the model this ships
            // against - looked from the outside like nothing happening.
            watch.tick(
                super::Progress {
                    prompt: tokens.len(),
                    read: at,
                    elapsed: started.elapsed(),
                    ..super::Progress::default()
                },
                "",
            );
        }
        // The question has been read; from here the numbers move.
        let mut report = super::Progress {
            prompt: tokens.len(),
            read: tokens.len(),
            written: 0,
            elapsed: started.elapsed(),
            // Read off what comes back rather than predicted: a model that
            // thinks does not always use the turn, and the empty `<think>`
            // block above is an argument for not using it.
            deliberating: false,
            generating: std::time::Duration::ZERO,
            ..super::Progress::default()
        };
        watch.tick(report, "");
        // The clock the rate is measured on starts here, once the weights are
        // in and the question has been read.
        let mut writing_since = std::time::Instant::now();
        // A passage handed back is unwrapped from whatever it was wrapped in;
        // an answer in a conversation only has its thinking taken off. The
        // fence a passage should not have is the fence an answer that is code
        // needs.
        let tidy = |text: &str| {
            if ask.talking() {
                super::without_thinking(text)
            } else {
                super::clean_reply(text)
            }
        };

        // The sampler Qwen3 asks for outside its thinking mode. Greedy decoding
        // was the obvious choice and the wrong one: at temperature zero a small
        // model's safest continuation of a passage is the passage, so it handed
        // the text back unchanged. The seed is fixed, so the same question still
        // gets the same answer twice — which is what makes a suggestion
        // something you can re-read rather than re-roll.
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(20),
            LlamaSampler::top_p(0.8, 1),
            LlamaSampler::temp(0.7),
            LlamaSampler::dist(0x5EED),
        ]);
        let mut out = Vec::new();
        let mut pos = tokens.len() as i32;
        let ceiling = pos + RESERVE as i32;
        while pos < ceiling {
            // Between two tokens is the only place a question can be given up
            // on: the decode itself is one call into a library that does not
            // come back until it is done.
            if !watch.carry_on() {
                break;
            }
            let token = sampler.sample(ctx, batch.n_tokens() - 1);
            sampler.accept(token);
            let piece = model
                .token_to_piece_bytes(token, 32, true, None)
                .map_err(|e| format!("detokenising: {e}"))?;
            if model.is_eog_token(token) {
                // Not every ending ends the turn. A reasoning model closes its
                // deliberation with an end-of-generation token and then opens
                // the channel the answer is actually in, so a stop here would
                // hand back the thinking and none of the reply. The turn is
                // over once something has been said on the final channel.
                let said = String::from_utf8_lossy(&out);
                if !said.contains("<|channel|>") || said.contains("<|channel|>final") {
                    break;
                }
                out.extend(piece);
            } else {
                out.extend(piece);
            }
            batch.clear();
            batch
                .add(token, pos, &[0], true)
                .map_err(|e| format!("batching: {e}"))?;
            ctx.decode(&mut batch).map_err(no_room)?;
            seen.push(token);
            pos += 1;

            report.written += 1;
            report.elapsed = started.elapsed();
            report.generating = writing_since.elapsed();
            // Where the answer has got to, read from the last few bytes of it:
            // a model that reasons opens a channel for the thinking and another
            // for the reply, and those two are worth counting separately. Only
            // the tail is looked at, and a marker straddling the boundary is
            // caught by the next token.
            let tail = String::from_utf8_lossy(&out[out.len().saturating_sub(64)..]).to_string();
            if !report.deliberating
                && (tail.contains("<|channel|>analysis") || tail.contains("<think>"))
            {
                report.deliberating = true;
                report.written = 0;
                writing_since = std::time::Instant::now();
            } else if report.deliberating
                && (tail.contains("<|channel|>final") || tail.contains("</think>"))
            {
                report.deliberating = false;
                report.written = 0;
                writing_since = std::time::Instant::now();
            }
            // What it has said so far, so somebody watching sees the answer
            // arrive rather than a number going up. The markers the reply is
            // wrapped in are taken out on the way, or the first thing on
            // screen is machinery.
            watch.tick(report, &tidy(&String::from_utf8_lossy(&out)));
        }

        let text = String::from_utf8_lossy(&out).to_string();
        // A reasoning model that ran out of room mid-thought has said nothing
        // on the channel the answer belongs on, and its deliberation is not a
        // suggestion. Better to say so than to paste the thinking into a note.
        if text.contains("<|channel|>") && !text.contains("<|channel|>final") {
            return Err("the model thought for longer than there was room for".into());
        }
        let text = tidy(&text);
        if text.is_empty() {
            // Being stopped before it had said anything is not the same as
            // having nothing to say, and the two used to come back identical.
            // Handing the passage back means "this is already what you asked
            // for" - a fair answer from a model that finished and a false one
            // from a model that was interrupted, which since a stop can now
            // land while the question is still being read is most of them.
            if !watch.carry_on() {
                return Err("stopped before it had written anything".into());
            }
            // Nothing to say is an answer to an *edit*: the passage is
            // already what the instruction asked for, and handing the source
            // back says so in the one vocabulary a diff understands. A
            // conversation has no source, so the same line handed back an
            // empty string and the panel drew a turn with nothing in it -
            // which reads as the application having lost the answer.
            if ask.talking() {
                return Err("it did not answer".into());
            }
            return Ok(ask.source.clone());
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::shared_prefix;

    #[test]
    fn marks_are_kept_spread_out_rather_than_clustered() {
        use super::MARKS;
        // Positions only; the states themselves cannot be forged from safe
        // code, so this stands in for them with what the rule actually reads.
        fn thin(marks: &mut Vec<usize>, at: usize) {
            marks.retain(|p| *p != at);
            marks.push(at);
            marks.sort_unstable();
            while marks.len() > MARKS {
                let mut crowded = 1;
                let mut closest = usize::MAX;
                for i in 1..marks.len() {
                    let gap = marks[i] - marks[i - 1];
                    if gap < closest {
                        closest = gap;
                        crowded = i;
                    }
                }
                marks.remove(crowded);
            }
        }
        // What a conversation actually does: one mark just before the project
        // it is about, and then a cluster at the far end as it goes on. The
        // early one is the one every *new* question needs, and the rule has to
        // keep it - it was losing it that cost 13.7 seconds against 2.1.
        let mut marks = Vec::new();
        for at in [5659, 5757, 5821, 5835, 5902, 6400, 7000] {
            thin(&mut marks, at);
        }
        assert!(marks.len() <= MARKS);
        assert_eq!(marks[0], 5659, "the mark in front of the project is kept");
        assert!(marks.windows(2).all(|w| w[0] < w[1]), "in order: {marks:?}");
        // And the same position twice is one mark, not two.
        let mut marks = Vec::new();
        thin(&mut marks, 100);
        thin(&mut marks, 100);
        assert_eq!(marks, vec![100]);
    }

    #[test]
    fn what_is_already_read_is_never_the_whole_question() {
        // A conversation: the same thing again with one more turn on the end.
        assert_eq!(shared_prefix(&[1, 2, 3], &[1, 2, 3, 4, 5]), 3);
        // The same question twice still leaves a token to sample the first
        // word from, which is the whole reason for the ceiling.
        assert_eq!(shared_prefix(&[1, 2, 3], &[1, 2, 3]), 2);
        // Remembering more than was asked for counts only what was asked for.
        assert_eq!(shared_prefix(&[1, 2, 3, 4, 5], &[1, 2, 3]), 2);
        // A different conversation shares its opening and nothing after it.
        assert_eq!(shared_prefix(&[1, 2, 9, 9], &[1, 2, 3, 4]), 2);
        // And two that share nothing share nothing.
        assert_eq!(shared_prefix(&[9, 9], &[1, 2, 3]), 0);
        assert_eq!(shared_prefix::<u8>(&[], &[1, 2, 3]), 0);
        assert_eq!(shared_prefix(&[1, 2, 3], &[]), 0);
    }
}
