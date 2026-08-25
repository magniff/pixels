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
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;

use super::{Ask, Backend, Reply};

/// Room for the prompt and the answer together. Qwen3 will take far more, but
/// a bigger window is a bigger key/value cache for every request, and an edit
/// that needs more than this wants to be two edits.
const CONTEXT: u32 = 4096;
/// Leave this much of the window for the answer.
const RESERVE: usize = 768;

/// Where the weights are, unless `PIXUI_MODEL` says otherwise.
pub fn default_path() -> PathBuf {
    std::env::var_os("PIXUI_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models/Qwen3-1.7B-Q4_K_M.gguf"))
}

pub struct Local {
    path: PathBuf,
    loaded: Option<(LlamaBackend, LlamaModel)>,
}

impl Local {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            loaded: None,
        }
    }

    fn load(&mut self) -> Result<&(LlamaBackend, LlamaModel), String> {
        if self.loaded.is_none() {
            // llama.cpp narrates its whole startup to stderr. Interesting once,
            // and then it is a hundred lines between you and any message the
            // editor actually wanted to print.
            llama_cpp_2::send_logs_to_tracing(
                llama_cpp_2::LogOptions::default().with_logs_enabled(false),
            );
            let backend = LlamaBackend::init().map_err(|e| format!("llama backend: {e}"))?;
            // Everything on the GPU where there is one. llama.cpp ignores this
            // on a build without one, so it needs no platform test here.
            let params = LlamaModelParams::default().with_n_gpu_layers(99);
            let model = LlamaModel::load_from_file(&backend, &self.path, &params)
                .map_err(|e| format!("loading {}: {e}", self.path.display()))?;
            leave_before_ggml_does();
            self.loaded = Some((backend, model));
        }
        Ok(self.loaded.as_ref().expect("just loaded"))
    }
}

/// The conversation, in the format Qwen3 was trained on.
///
/// The empty `<think>` block is not decoration: Qwen3 reasons out loud unless
/// told the thinking is already done, and several hundred tokens of
/// deliberation about a comma is not what anybody asked for.
///
/// The instruction comes *after* the text. A small model reads the last thing
/// it was told as the thing to do, and with the order the other way round it
/// reliably answered the question "what is this text?" — by handing the text
/// back.
fn prompt(ask: &Ask) -> String {
    format!(
        "<|im_start|>system\n\
         You are the editor built into a markdown note-taking app. Somebody has \
         selected a passage from their own notes and told you what to do with \
         it: proofread it, tighten it, rewrite it, change how it sounds. Do \
         exactly that, to the whole passage, and hand the passage back. Even a \
         vague instruction gets a real change — handing back the text as you \
         found it is not an answer. Keep any markdown markup, and keep the \
         author's facts. Reply with the rewritten passage and nothing else: no \
         preamble, no explanation, no quotes, no code fences.<|im_end|>\n\
         <|im_start|>user\n\
         Text:\n{}\n\n\
         Instruction: {}<|im_end|>\n\
         <|im_start|>assistant\n<think>\n\n</think>\n\n",
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

/// Take off what a model wraps an answer in when it cannot help itself.
fn unwrap_reply(text: &str) -> String {
    let mut out = text.trim();
    if let Some(rest) = out.strip_prefix("```") {
        // Drop the info string on the opening fence along with it.
        let rest = rest.split_once('\n').map_or(rest, |(_, r)| r);
        out = rest.strip_suffix("```").unwrap_or(rest).trim();
    }
    out.to_string()
}

impl Backend for Local {
    fn name(&self) -> String {
        self.path
            .file_stem()
            .map(|s| s.to_string_lossy().to_uppercase())
            .unwrap_or_else(|| "LOCAL MODEL".into())
    }

    fn edit(&mut self, ask: &Ask) -> Reply {
        let (backend, model) = self.load()?;
        let text = prompt(ask);
        let tokens = model
            .str_to_token(&text, AddBos::Never)
            .map_err(|e| format!("tokenising: {e}"))?;
        if tokens.len() + RESERVE > CONTEXT as usize {
            return Err("that selection is too long for one edit".into());
        }

        let params = LlamaContextParams::default().with_n_ctx(NonZeroU32::new(CONTEXT));
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
        ctx.decode(&mut batch).map_err(|e| format!("decode: {e}"))?;

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
            if model.is_eog_token(token) {
                break;
            }
            out.extend(
                model
                    .token_to_piece_bytes(token, 32, false, None)
                    .map_err(|e| format!("detokenising: {e}"))?,
            );
            batch.clear();
            batch
                .add(token, pos, &[0], true)
                .map_err(|e| format!("batching: {e}"))?;
            ctx.decode(&mut batch).map_err(|e| format!("decode: {e}"))?;
            pos += 1;
        }

        let text = String::from_utf8_lossy(&out).to_string();
        let text = unwrap_reply(&text);
        if text.is_empty() {
            return Err("the model had nothing to say".into());
        }
        Ok(text)
    }
}
