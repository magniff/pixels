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
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use super::{Ask, Backend, Reply};

/// Leave this much of the window for the answer, deliberation included.
const RESERVE: usize = 2048;

/// Where the weights are, unless `PIXUI_MODEL` says otherwise.
pub fn default_path() -> PathBuf {
    std::env::var_os("PIXUI_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models/Qwen3-1.7B-Q4_K_M.gguf"))
}

pub struct Local {
    path: PathBuf,
    /// What the model is told it is doing, from the settings.
    system: String,
    /// The largest window this is allowed to open, from the settings. Bigger
    /// is not free: the key/value cache grows with it, in the same memory the
    /// weights are sitting in.
    context: u32,
    loaded: Option<(LlamaBackend, LlamaModel)>,
    /// Whether this model reasons out loud unless told the thinking is done.
    thinks: bool,
}

impl Local {
    pub fn new(path: impl AsRef<Path>, system: String, context: u32) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            system,
            context,
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
            self.loaded = Some((backend, model));
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
            "that selection needs a bigger context window than this is set to".to_string()
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
fn render(model: &LlamaModel, system: &str, user: &str, thinks: bool) -> Result<String, String> {
    let template = model
        .chat_template(None)
        .map_err(|e| format!("this model has no chat template: {e}"))?;
    let chat = [
        LlamaChatMessage::new("system".into(), system.trim().to_string())
            .map_err(|e| format!("system turn: {e}"))?,
        LlamaChatMessage::new("user".into(), user.to_string())
            .map_err(|e| format!("user turn: {e}"))?,
    ];
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
fn instruction(ask: &Ask) -> String {
    format!(
        "Text:\n{}\n\nInstruction: {}",
        ask.source,
        ask.request.trim()
    )
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
        unsafe { _exit(0) }
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

    fn edit(&mut self, ask: &Ask, tick: &mut dyn FnMut(super::Progress)) -> Reply {
        let started = std::time::Instant::now();
        self.load()?;
        let thinks = self.thinks;
        let ceiling = self.context;
        let system = self.system.clone();
        let (backend, model) = self.loaded.as_ref().expect("loaded above");
        let text = render(model, &system, &instruction(ask), thinks)?;
        let tokens = model
            .str_to_token(&text, AddBos::Never)
            .map_err(|e| format!("tokenising: {e}"))?;
        if tokens.len() + RESERVE > ceiling as usize {
            return Err(format!(
                "that selection needs more than the {}k window this is set to",
                ceiling / 1024
            ));
        }

        // The window is sized to this request rather than to the ceiling. The
        // key/value cache is allocated with the context and paid for in memory
        // whether or not it is used, so a one-line edit should not reserve room
        // for a whole note.
        let want = ((tokens.len() + RESERVE) as u32)
            .next_power_of_two()
            .clamp(1024, ceiling);
        let params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(want));
        let mut ctx = model
            .new_context(backend, params)
            .map_err(|e| format!("context: {e}"))?;

        let mut batch = LlamaBatch::new(tokens.len().max(1), 1);
        let last = tokens.len() - 1;
        for (i, token) in tokens.iter().enumerate() {
            batch
                .add(*token, i as i32, &[0], i == last)
                .map_err(|e| format!("batching: {e}"))?;
        }
        ctx.decode(&mut batch).map_err(no_room)?;
        // The question has been read; from here the numbers move.
        let mut report = super::Progress {
            prompt: tokens.len(),
            written: 0,
            elapsed: started.elapsed(),
            // Read off what comes back rather than predicted: a model that
            // thinks does not always use the turn, and the empty `<think>`
            // block above is an argument for not using it.
            deliberating: false,
            generating: std::time::Duration::ZERO,
        };
        tick(report);
        // The clock the rate is measured on starts here, once the weights are
        // in and the question has been read.
        let mut writing_since = std::time::Instant::now();

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
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
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
            tick(report);
        }

        let text = String::from_utf8_lossy(&out).to_string();
        // A reasoning model that ran out of room mid-thought has said nothing
        // on the channel the answer belongs on, and its deliberation is not a
        // suggestion. Better to say so than to paste the thinking into a note.
        if text.contains("<|channel|>") && !text.contains("<|channel|>final") {
            return Err("the model thought for longer than there was room for".into());
        }
        let text = super::clean_reply(&text);
        if text.is_empty() {
            // Nothing to say is an answer: the passage is already what the
            // instruction asked for. Handing back the source says that in the
            // one vocabulary the caller understands — a diff with nothing in it.
            return Ok(ask.source.clone());
        }
        Ok(text)
    }
}
