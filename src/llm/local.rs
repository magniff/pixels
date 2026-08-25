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
            self.loaded = Some((backend, model));
        }
        Ok(self.loaded.as_ref().expect("just loaded"))
    }
}

/// The conversation, in the format Qwen3 was trained on.
///
/// The empty `<think>` block is not decoration: Qwen3 reasons out loud unless
/// told the thinking is already done, and several hundred tokens of deliberation
/// about a comma is not what anybody asked for.
fn prompt(ask: &Ask) -> String {
    format!(
        "<|im_start|>system\n\
         You are a careful copy editor. You rewrite the text exactly as \
         instructed, keeping the author's voice and any markdown markup. Reply \
         with the rewritten text and nothing else: no preamble, no explanation, \
         no quotes, no code fences.<|im_end|>\n\
         <|im_start|>user\n\
         Instruction: {}\n\n\
         Text:\n{}<|im_end|>\n\
         <|im_start|>assistant\n<think>\n\n</think>\n\n",
        ask.request.trim(),
        ask.source
    )
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

        // Greedy: an edit should be the model's best answer, not a sample from
        // its neighbourhood. Two runs of the same request give the same text,
        // which is what makes a suggestion something you can re-read.
        let mut sampler = LlamaSampler::greedy();
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
