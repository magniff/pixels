//! What the assistant is, and where it is kept.
//!
//! Two things the user chooses — which weights to run and what to tell them —
//! plus the small amount of filing that makes those choices survive a restart.
//! Written by hand rather than through a serialisation crate: it is two fields,
//! and a dependency that can parse anything is a strange price to pay for that.

use std::path::{Path, PathBuf};

/// What the model is told it is doing, before it is told what to do.
///
/// Written on several lines and stored that way, because it is edited in a vim
/// editor: with the whole thing on one line there is nothing for `j` and `k` to
/// move between, and every edit is a hunt along a single wrapped paragraph.
/// Flush left, because a string literal keeps whatever indentation it is given
/// and the model would be reading it.
///
/// Everything here was learned the hard way. Telling it the job stops it
/// answering "what is this text?" with the text; saying that handing the
/// passage back is not an answer stops a vague instruction returning nothing;
/// naming markdown stops it eating the markup.
pub const DEFAULT_PROMPT: &str = "You are the editor built into a markdown note app.
Somebody has selected a passage from their own notes
and told you what to do with it: proofread it, tighten
it, rewrite it, change how it sounds.
Do exactly that, to the whole passage, and hand the
passage back. Even a vague instruction gets a real
change - handing back the text as you found it is not
an answer.
Keep any markdown markup, and keep the author's facts.
Put the rewritten passage between <text> and </text>
and write nothing at all outside them: no preamble, no
explanation, no quotes, no code fences.";

/// Weights the app knows how to fetch.
///
/// Sizes are the ones the hub reports, so the progress bar has something to
/// measure against without a round trip to ask.
pub struct Weights {
    pub label: &'static str,
    /// Short enough to sit in a column beside the name.
    pub file: &'static str,
    pub url: &'static str,
    pub megabytes: u64,
    /// What it is good for, short enough to sit in a column of its own.
    pub note: &'static str,
}

pub const CATALOGUE: &[Weights] = &[
    Weights {
        label: "QWEN3 1.7B",
        file: "Qwen3-1.7B-Q4_K_M.gguf",
        url: "https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
        megabytes: 1170,
        note: "PROOFREADS AND TIGHTENS",
    },
    Weights {
        label: "QWEN3 4B",
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        megabytes: 2400,
        note: "REWRITES AND CHANGES TONE",
    },
    Weights {
        label: "QWEN3 14B",
        file: "Qwen3-14B-Q4_K_M.gguf",
        url: "https://huggingface.co/Qwen/Qwen3-14B-GGUF/resolve/main/Qwen3-14B-Q4_K_M.gguf",
        megabytes: 9002,
        note: "THINKS BEFORE IT REWRITES",
    },
    Weights {
        label: "GPT-OSS 20B",
        file: "gpt-oss-20b-MXFP4.gguf",
        url: "https://huggingface.co/ggml-org/gpt-oss-20b-GGUF/resolve/main/gpt-oss-20b-MXFP4.gguf",
        megabytes: 12100,
        note: "REASONS ABOUT WHAT YOU MEANT",
    },
];

/// Where weights are kept. `PIXUI_MODELS` moves it; the default is beside the
/// binary's working directory, which is where the README's curl line puts them.
pub fn models_dir() -> PathBuf {
    std::env::var_os("PIXUI_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models"))
}

/// Every `.gguf` on disk, whether or not the catalogue knows about it.
pub fn installed() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(models_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gguf") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The colour scheme, by the toolkit's name for it.
    pub scheme: String,
    /// The face, likewise.
    pub font: String,
    /// The largest context window the assistant may open, in tokens. The
    /// key/value cache for one is allocated in the same memory the weights are
    /// in, so this is a memory budget as much as a length.
    pub context: u32,
    /// Whether the editing assistant is offered at all. With this off there is
    /// no mark beside a selection and nothing to ask — the app is a text editor
    /// and nothing else, which is a perfectly good thing for it to be.
    pub assist: bool,
    /// The weights file to run, by name rather than by path: the folder can
    /// move between machines and the choice should survive it.
    pub model: Option<String>,
    pub prompt: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            // Gruvbox dark: warm, low contrast, and easy to read from for an
            // hour, which is what a note editor is for.
            scheme: "GRUVBOX DARK".to_string(),
            font: "PIXUI 5X7".to_string(),
            context: 8192,
            // On, because a feature nobody can see is a feature nobody finds.
            assist: true,
            model: None,
            prompt: DEFAULT_PROMPT.to_string(),
        }
    }
}

impl Settings {
    /// The file the settings live in.
    pub fn path() -> PathBuf {
        if let Some(dir) = std::env::var_os("PIXUI_CONFIG") {
            return PathBuf::from(dir);
        }
        let home = std::env::var_os(if cfg!(windows) { "APPDATA" } else { "HOME" })
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        if cfg!(windows) {
            home.join("pixui-notes").join("settings.conf")
        } else {
            home.join(".config")
                .join("pixui-notes")
                .join("settings.conf")
        }
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    /// `key = value`, one per line, with newlines in a value written `\n`.
    /// Anything unrecognised is left alone rather than dropped, so a file from
    /// a newer build survives being read by an older one.
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "scheme" if !value.is_empty() => out.scheme = value.to_string(),
                "font" if !value.is_empty() => out.font = value.to_string(),
                "context" => {
                    if let Ok(n) = value.parse::<u32>() {
                        out.context = n.clamp(2048, 131_072);
                    }
                }
                "assist" => out.assist = value != "off",
                "model" if !value.is_empty() => out.model = Some(value.to_string()),
                "prompt" if !value.is_empty() => out.prompt = unescape(value),
                _ => {}
            }
        }
        out
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("scheme = {}\n", self.scheme));
        out.push_str(&format!("font = {}\n", self.font));
        out.push_str(&format!("context = {}\n", self.context));
        out.push_str(&format!(
            "assist = {}\n",
            if self.assist { "on" } else { "off" }
        ));
        if let Some(model) = &self.model {
            out.push_str(&format!("model = {model}\n"));
        }
        out.push_str(&format!("prompt = {}\n", escape(&self.prompt)));
        out
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
    }

    /// The weights this configuration points at, if they are on disk.
    pub fn model_path(&self) -> Option<PathBuf> {
        let dir = models_dir();
        match &self.model {
            Some(name) => Some(dir.join(name)).filter(|p| p.exists()),
            // Nothing chosen: whatever is installed, which for most people is
            // the one thing they downloaded.
            None => installed().into_iter().next(),
        }
    }
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Whether a path names weights the catalogue describes.
pub fn described(path: &Path) -> Option<&'static Weights> {
    let name = path.file_name()?.to_str()?;
    CATALOGUE.iter().find(|w| w.file == name)
}
