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

/// The two that earned their place.
///
/// Seven were tried on a 24GB Mac against the same battery: six questions with
/// a checkable answer, two it cannot know - where the answer is saying so -
/// and one asking for a change in the block format this app reads.
///
/// | model | on disk | right | writing | later questions |
/// | --- | --- | --- | --- | --- |
/// | Qwen3.5 9B | 5.7 GB | 6/6 | 20.7 tok/s | 0.30 s |
/// | Ornith 1.5 9B | 5.8 GB | 6/6 | 20.9 tok/s | 0.30 s |
/// | Qwen3 14B | 9.0 GB | 6/6 | 12.5 tok/s | 0.48 s |
/// | Qwen3.8 27B (IQ3_S) | 12.0 GB | 5/6 | 9.0 tok/s | 0.85 s |
/// | gpt-oss 20b | 12.1 GB | 3/6 | 40.7 tok/s | 1.48 s |
///
/// Nothing beat the two 9Bs, and the reason is memory rather than merit: 24GB
/// of it leaves the GPU about sixteen, so a model much past nine gigabytes
/// spends what is left of the machine on itself. Qwen3.8's smallest release is
/// 27B, which only fits by quantising it down to where it is slower than a 9B
/// and no more right.
///
/// gpt-oss 20b came out because it was the largest and the worst: three out of
/// six, by never reaching for the clock at all and inventing instead - a
/// Saturday, seven days until Christmas, a date of 20231124, a llama.cpp
/// release numbered v0.1.0beta. A model that says it does not know is worth
/// more here than a bigger one that does not know it does not.
///
/// Two others were tried and are not here for reasons of this application
/// rather than of theirs. LFM2.5 8B writes at 70 tokens a second, three times
/// anything else, and scored nothing because it calls its tools in a syntax
/// this does not parse. Gemma 4 will not load at all: its chat template is
/// eighteen thousand characters of Jinja and the llama.cpp this builds against
/// answers every one of them with the same error.
pub const CATALOGUE: &[Weights] = &[
    Weights {
        label: "QWEN3.5 9B",
        file: "Qwen3.5-9B-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf",
        megabytes: 5680,
        note: "SAYS WHEN IT DOES NOT KNOW",
    },
    Weights {
        label: "ORNITH 1.5 9B",
        file: "Ornith-1.5-9B-Q4_K_M.gguf",
        url: "https://huggingface.co/ornith-ai/Ornith-1.5-9B-GGUF/resolve/main/Ornith-1.5-9B-Q4_K_M.gguf",
        megabytes: 5780,
        note: "ANSWERS SHORT, LOOKS THINGS UP",
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
    /// Whether a conversation may reach the network.
    ///
    /// Off by default, and it is the one setting here that is about something
    /// other than taste: with it on, a question can send a place name or a
    /// search term to somebody else's server. Everything else this program does
    /// happens on the machine it is running on, and that should not stop being
    /// true without somebody saying so.
    pub web: bool,
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
            web: false,
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
                "assist" => out.assist = value != "off",
                "web" => out.web = value == "on",
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
        out.push_str(&format!(
            "assist = {}\n",
            if self.assist { "on" } else { "off" }
        ));
        out.push_str(&format!("web = {}\n", if self.web { "on" } else { "off" }));
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
