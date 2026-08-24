//! Syntax highlighting for fenced code blocks.
//!
//! Sublime Text grammars by way of syntect, but none of syntect's colour
//! schemes: those are built for a full RGB palette and would fight the sixteen
//! warm tones the rest of the app is drawn from. What comes out of syntect is a
//! *scope* per run of characters — `keyword.control.rust`, `string.quoted`,
//! `comment.line` — and mapping those onto the palette ourselves is both the
//! smaller job and the only way the result belongs on the same screen.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

use crate::markdown::{Span, Tok};

/// What a run of code means, in the only categories the palette has room to
/// tell apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Code {
    Plain,
    Keyword,
    Type,
    Function,
    String,
    Number,
    Comment,
    Punctuation,
}

/// The grammars, loaded once. Around two megabytes of embedded definitions, so
/// this is deliberately not done per note.
fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Map a syntect scope stack to one of our categories.
///
/// Scopes are dotted and hierarchical with the most specific innermost, so this
/// walks from the inside out and takes the first thing it recognises. The order
/// of the arms matters: a comment's `//` is scoped
/// `punctuation.definition.comment`, and reading that as punctuation rather
/// than as comment leaves the slashes lit up beside grey text.
fn classify(stack: &ScopeStack) -> Code {
    for scope in stack.as_slice().iter().rev() {
        let name = scope.build_string();
        let head = name.split('.').next().unwrap_or("");

        // Delimiters belong to the thing they delimit, not to punctuation.
        if name.contains("comment") {
            return Code::Comment;
        }
        if head == "string" || name.starts_with("punctuation.definition.string") {
            return Code::String;
        }
        if name.starts_with("constant.numeric") {
            return Code::Number;
        }
        if name.starts_with("constant.language") || name.starts_with("constant.character") {
            return Code::Keyword;
        }
        // Operators are not keywords: `=` reading as loud as `let` makes an
        // expression hard to skim.
        if name.starts_with("keyword.operator") {
            return Code::Punctuation;
        }
        if head == "keyword" {
            return Code::Keyword;
        }
        // `let`, `fn`, `struct`, `pub`, and also `u32` — Rust scopes primitives
        // the same way it scopes the words that introduce them, so they share a
        // colour. Most themes make the same trade.
        if head == "storage" {
            return Code::Keyword;
        }
        if name.starts_with("entity.name") {
            return if name.contains("function") {
                Code::Function
            } else {
                Code::Type
            };
        }
        if name.starts_with("support.function") || name.starts_with("support.macro") {
            return Code::Function;
        }
        if head == "support" || head == "entity" {
            return Code::Type;
        }
        if head == "punctuation" {
            return Code::Punctuation;
        }
    }
    Code::Plain
}

/// Highlight `lines` as `lang`, one span list per line.
///
/// Results are memoised on the text itself rather than on a revision counter,
/// so an edit that puts the code back how it was costs nothing, and nothing has
/// to remember to invalidate anything.
pub fn highlight(lang: &str, lines: &[String]) -> Vec<Vec<Span>> {
    thread_local! {
        static CACHE: RefCell<HashMap<u64, Vec<Vec<Span>>>> = RefCell::new(HashMap::new());
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lang.hash(&mut hasher);
    for l in lines {
        l.hash(&mut hasher);
    }
    let key = hasher.finish();

    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return hit;
    }

    let out = highlight_uncached(lang, lines);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        // A note has a handful of code blocks; anything beyond that is stale.
        if c.len() > 64 {
            c.clear();
        }
        c.insert(key, out.clone());
    });
    out
}

fn highlight_uncached(lang: &str, lines: &[String]) -> Vec<Vec<Span>> {
    let set = syntaxes();
    let syntax = set
        .find_syntax_by_token(lang)
        .or_else(|| set.find_syntax_by_extension(lang))
        .unwrap_or_else(|| set.find_syntax_plain_text());

    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::with_capacity(lines.len());

    for line in lines {
        // The default grammars are the newline-terminated ones, and a line
        // without its terminator parses differently — an unterminated string
        // would swallow the rest of the block.
        let text = format!("{line}\n");
        let ops = match state.parse_line(&text, set) {
            Ok(ops) => ops,
            Err(_) => {
                out.push(vec![Span {
                    text: line.clone(),
                    tok: Tok::Code,
                    bold: false,
                }]);
                continue;
            }
        };

        let mut spans: Vec<Span> = Vec::new();
        let mut last = 0usize;
        let push = |from: usize, to: usize, stack: &ScopeStack, spans: &mut Vec<Span>| {
            // Never emit the newline we added.
            let to = to.min(line.len());
            if from >= to {
                return;
            }
            let Some(text) = line.get(from..to) else {
                return;
            };
            let code = classify(stack);
            match spans.last_mut() {
                // Runs of the same category are merged, so a line of plain
                // code is one span rather than forty.
                Some(prev) if prev.tok == code_token(code) && !prev.bold => {
                    prev.text.push_str(text)
                }
                _ => spans.push(Span {
                    text: text.to_string(),
                    tok: code_token(code),
                    bold: false,
                }),
            }
        };

        for (offset, op) in ops {
            push(last, offset, &stack, &mut spans);
            last = offset;
            let _ = stack.apply(&op);
        }
        push(last, line.len(), &stack, &mut spans);

        if spans.is_empty() {
            spans.push(Span {
                text: String::new(),
                tok: Tok::Code,
                bold: false,
            });
        }
        out.push(spans);
    }
    out
}

/// Categories ride on the existing span token so the renderers need no new
/// vocabulary; `Tok` gains one variant per category rather than a parallel
/// type threaded through both views.
fn code_token(code: Code) -> Tok {
    match code {
        Code::Plain => Tok::CodePlain,
        Code::Keyword => Tok::CodeKeyword,
        Code::Type => Tok::CodeType,
        Code::Function => Tok::CodeFunction,
        Code::String => Tok::CodeString,
        Code::Number => Tok::CodeNumber,
        Code::Comment => Tok::CodeComment,
        Code::Punctuation => Tok::CodePunct,
    }
}

/// Highlighted spans for every buffer line that sits inside a fenced block.
///
/// `None` for lines that are not code, including the fences themselves — those
/// are markdown, and the markdown highlighter should keep them.
pub fn code_regions(lines: &[String]) -> Vec<Option<Vec<Span>>> {
    let mut out: Vec<Option<Vec<Span>>> = vec![None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if !crate::markdown::is_fence(&lines[i]) {
            i += 1;
            continue;
        }
        let lang = lines[i].trim().trim_start_matches('`').trim().to_string();
        let start = i + 1;
        let mut end = start;
        while end < lines.len() && !crate::markdown::is_fence(&lines[end]) {
            end += 1;
        }
        if start < end {
            let body: Vec<String> = lines[start..end].to_vec();
            for (n, spans) in highlight(&lang, &body).into_iter().enumerate() {
                out[start + n] = Some(spans);
            }
        }
        i = end + 1;
    }
    out
}
