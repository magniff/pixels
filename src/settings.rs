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
/// Thirteen were tried on a 24GB Mac against the same battery: six questions
/// with a checkable answer - a sum, a weekday, a day count, something from the
/// notes themselves, and two dates written the way people write them.
///
/// | model | on disk | right | writing | cold | scenes |
/// | --- | --- | --- | --- | --- | --- |
/// | Qwen3.6 35B-A3B (IQ3_XXS) | 13.2 GB | 7/7 | 37.5 tok/s | 12 s | 9/9, 33/34 |
/// | Gemma 4 26B-A4B (Q3_K_XL) | 12.9 GB | - | - | 10 s | 27/34 |
/// | LFM2 24B-A2B (IQ4_XS) | 12.7 GB | - | - | 12 s | 11/18 |
/// | Trinity Mini 26B-A3B (Q3_K_M) | 12.1 GB | - | - | 13 s | 4/18 |
/// | Qwen3.5 35B-A3B (IQ3_XXS) | 13.1 GB | 7/7 | 36.6 tok/s | 15 s | 7/9 |
/// | Qwen3.5 27B (Q3_K_S) | 12.3 GB | 6/6 | 8.5 tok/s | 37 s | - |
/// | Ornith 1.5 9B | 5.8 GB | 6/6 | 18.8 tok/s | 12 s | 7/9 |
/// | Qwen3.5 9B | 5.7 GB | 5/6 | 20.1 tok/s | 12 s | 6/9 |
/// | gpt-oss 20b | 12.1 GB | 3/6 | 40.7 tok/s | 13 s | - |
///
/// The last column is `tools/e2e.sh`, which drives the whole application and
/// then looks at the vault - nine scenes when the first six were measured,
/// thirty-one by the time the last three were. Qwen3.6 is the only one that
/// has answered every scene of the nine, and it is asked to think first, in
/// a line on the end of every question, which took it from twenty-four of
/// twenty-seven to twenty-six and through the one scene it had never passed.
/// The same line took Gemma from twenty-five to twenty-three, so Gemma is
/// not asked. See `llm::THINK_FIRST`.
///
/// Gemma 4 is the other one worth having. Faster on most scenes - twenty
/// seconds against thirty-three on the one that changes a file behind its
/// back - and terser: asked for one word it gives one. What it misses on the
/// thirty-one is arithmetic it does in its head instead of asking for, and a
/// note in another project it will not read without being told to. What it gets wrong is its own: told the next Christmas
/// is a Friday and the last was a Thursday, it answers Thursday; asked about
/// a note in another project it sometimes says the web is switched off. It
/// would not load at all until its turns were written out by hand, because
/// its template is eighteen thousand characters of Jinja the formatter here
/// cannot run - see `llm::local::gemma_turns`.
///
/// LFM2 24B-A2B loads and is quick, and invents: today was the 31st of July
/// 2023, Christmas 2023 fell on a Monday. Trinity Mini describes a change in
/// prose instead of writing the block for it, and made bike.md by editing
/// line one of a file that did not exist. Neither is offered.
///
/// The 35B is the one to have, and it is a surprise: it is quantised down to
/// three bits to fit, which has spoiled every other model tried that way. It
/// survives because only three billion of its thirty-five are awake for any
/// one token - so it is the largest model here and also the fastest, twice the
/// speed of a 9B while getting more right.
///
/// The dense 27B matches it for correctness at eight tokens a second, which is
/// slower than reading. Size on its own buys nothing: gpt-oss 20b, the biggest
/// thing tried, scored three out of six by never reaching for the clock and
/// inventing instead - a Saturday, seven days until Christmas, a date of
/// 20231124, a llama.cpp release numbered v0.1.0beta. A model that says it does
/// not know is worth more here than a bigger one that does not know it does
/// not.
///
/// Ornith was kept for a while as the smaller one - half the size, half the
/// speed, and nearly as right on the battery above. It is not offered any
/// more, because the scenes are where the difference shows: it parroted a
/// bracketed note out of its own history back as an answer, and it gave up on
/// a lifetime share it had all the numbers for. Offering a model that does
/// that means offering two applications, and only one of them works.
/// Anything on disk can still be chosen; this list is what is fetched.
///
/// Four were tried and are not here for reasons of this application rather
/// than of theirs. LFM2.5 8B writes at 70 tokens a second and scored nothing,
/// because it calls its tools in a syntax this does not parse. Gemma 4 will
/// not load at all: its chat template is eighteen thousand characters of Jinja
/// and the llama.cpp this builds against answers every one with the same
/// error. Kimi K3's smallest quantisation is 466 GB, and DeepSeek V4-Flash's
/// is 82 GB. There is no Qwen3.7; the line goes 3.5, 3.6, 3.8, and every
/// Qwen3.8 is out of reach - the 27B answers five of six at eight tokens a
/// second, and Flash-Next is 72 GB before it is quantised at all.
pub const CATALOGUE: &[Weights] = &[
    Weights {
        label: "QWEN3.6 35B",
        file: "Qwen3.6-35B-A3B-UD-IQ3_XXS.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.6-35B-A3B-GGUF/resolve/main/Qwen3.6-35B-A3B-UD-IQ3_XXS.gguf",
        megabytes: 13200,
        note: "THE MOST RIGHT OF THESE",
    },
    Weights {
        label: "GEMMA 4 26B",
        file: "gemma-4-26B-A4B-it-UD-Q3_K_XL.gguf",
        url: "https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF/resolve/main/gemma-4-26B-A4B-it-UD-Q3_K_XL.gguf",
        megabytes: 12900,
        note: "AS RIGHT, FASTER, AND TERSER",
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
