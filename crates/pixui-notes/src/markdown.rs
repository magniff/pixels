//! A small markdown highlighter.
//!
//! This is a *source* editor, so markers stay visible and get dimmed rather
//! than being hidden — you are editing the markdown, not a rendering of it.
//! The same parser feeds the sidebar, where markers are stripped instead to
//! make a clean preview.

/// What a run of characters is, semantically. Colours live in the app so the
/// parser stays theme-agnostic.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Text,
    /// `#`, `-`, `>` and friends: structure, not content.
    Marker,
    Heading,
    Bold,
    Italic,
    Code,
    Link,
    Quote,
}

/// A run of characters sharing one token type.
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub tok: Tok,
    pub bold: bool,
}

impl Span {
    fn new(text: impl Into<String>, tok: Tok, bold: bool) -> Self {
        Self {
            text: text.into(),
            tok,
            bold,
        }
    }
}

/// Whether a line opens or closes a fenced code block.
pub fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// Split one line into styled spans.
///
/// `in_code` is threaded by the caller because a fenced block spans lines, and
/// a line-at-a-time highlighter cannot know it on its own.
pub fn highlight(line: &str, in_code: bool) -> Vec<Span> {
    if in_code || is_fence(line) {
        return vec![Span::new(line, Tok::Code, false)];
    }

    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];
    let mut out = Vec::new();
    if !indent.is_empty() {
        out.push(Span::new(indent, Tok::Text, false));
    }

    // ---- horizontal rule -------------------------------------------------
    if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-') {
        out.push(Span::new(trimmed, Tok::Marker, false));
        return out;
    }

    // ---- heading ---------------------------------------------------------
    if let Some(hashes) = leading_run(trimmed, '#') {
        if hashes <= 6 && trimmed[hashes..].starts_with(' ') {
            out.push(Span::new(&trimmed[..hashes + 1], Tok::Marker, false));
            out.push(Span::new(&trimmed[hashes + 1..], Tok::Heading, true));
            return out;
        }
    }

    // ---- block quote -----------------------------------------------------
    if let Some(rest) = trimmed.strip_prefix("> ") {
        out.push(Span::new("> ", Tok::Marker, false));
        out.extend(inline(rest).into_iter().map(|mut s| {
            if s.tok == Tok::Text {
                s.tok = Tok::Quote;
            }
            s
        }));
        return out;
    }

    // ---- list item -------------------------------------------------------
    if let Some(marker) = list_marker(trimmed) {
        out.push(Span::new(&trimmed[..marker], Tok::Marker, false));
        out.extend(inline(&trimmed[marker..]));
        return out;
    }

    out.extend(inline(trimmed));
    out
}

/// Length of the list marker at the start of `s`, including its trailing space.
fn list_marker(s: &str) -> Option<usize> {
    for lead in ["- ", "* ", "+ "] {
        if s.starts_with(lead) {
            return Some(2);
        }
    }
    // Ordered items: digits, then `.` or `)`, then a space.
    let digits = s.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &s[digits..];
        if (rest.starts_with(". ") || rest.starts_with(") ")) && digits < 10 {
            return Some(digits + 2);
        }
    }
    None
}

fn leading_run(s: &str, c: char) -> Option<usize> {
    let n = s.chars().take_while(|x| *x == c).count();
    (n > 0).then_some(n)
}

/// Parse inline emphasis, code spans and links inside one line of body text.
fn inline(s: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut plain = String::new();

    let flush = |plain: &mut String, out: &mut Vec<Span>| {
        if !plain.is_empty() {
            out.push(Span::new(std::mem::take(plain), Tok::Text, false));
        }
    };

    while i < bytes.len() {
        // Delimiters are always emitted as their own dim `Marker` spans rather
        // than being folded into the styled run. Two bold asterisks drawn with
        // the faux-bold double-strike merge into a solid block otherwise.
        let take = |chars: &[char], a: usize, b: usize| -> String { chars[a..b].iter().collect() };

        // `code`
        if bytes[i] == '`' {
            if let Some(end) = find(&bytes, i + 1, '`') {
                flush(&mut plain, &mut out);
                out.push(Span::new("`", Tok::Marker, false));
                out.push(Span::new(take(&bytes, i + 1, end), Tok::Code, false));
                out.push(Span::new("`", Tok::Marker, false));
                i = end + 1;
                continue;
            }
        }
        // **bold**
        if bytes[i] == '*' && bytes.get(i + 1) == Some(&'*') {
            if let Some(end) = find_pair(&bytes, i + 2) {
                flush(&mut plain, &mut out);
                out.push(Span::new("**", Tok::Marker, false));
                out.push(Span::new(take(&bytes, i + 2, end), Tok::Bold, true));
                out.push(Span::new("**", Tok::Marker, false));
                i = end + 2;
                continue;
            }
        }
        // *italic*
        if bytes[i] == '*' {
            if let Some(end) = find(&bytes, i + 1, '*') {
                flush(&mut plain, &mut out);
                out.push(Span::new("*", Tok::Marker, false));
                out.push(Span::new(take(&bytes, i + 1, end), Tok::Italic, false));
                out.push(Span::new("*", Tok::Marker, false));
                i = end + 1;
                continue;
            }
        }
        // [label](target)
        if bytes[i] == '[' {
            if let Some(close) = find(&bytes, i + 1, ']') {
                if bytes.get(close + 1) == Some(&'(') {
                    if let Some(paren) = find(&bytes, close + 2, ')') {
                        flush(&mut plain, &mut out);
                        out.push(Span::new("[", Tok::Marker, false));
                        out.push(Span::new(take(&bytes, i + 1, close), Tok::Link, false));
                        // The target is machinery, not prose: dim it.
                        out.push(Span::new(
                            take(&bytes, close, paren + 1),
                            Tok::Marker,
                            false,
                        ));
                        i = paren + 1;
                        continue;
                    }
                }
            }
        }
        plain.push(bytes[i]);
        i += 1;
    }
    flush(&mut plain, &mut out);
    out
}

fn find(chars: &[char], from: usize, target: char) -> Option<usize> {
    (from..chars.len()).find(|&i| chars[i] == target)
}

/// Find the closing `**` of a bold run.
fn find_pair(chars: &[char], from: usize) -> Option<usize> {
    (from..chars.len().saturating_sub(1)).find(|&i| chars[i] == '*' && chars[i + 1] == '*')
}

/// A title for a note: its first heading, else its first non-empty line.
pub fn derive_title(lines: &[String]) -> String {
    for line in lines {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let title = rest.trim_start_matches('#').trim();
            if !title.is_empty() {
                return truncate(title, 24);
            }
        }
    }
    for line in lines {
        let t = line.trim();
        if !t.is_empty() {
            return truncate(t, 24);
        }
    }
    "UNTITLED".to_string()
}

/// A couple of lines of body text with the markdown taken out, for the sidebar.
pub fn preview(lines: &[String], max_lines: usize, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen_title = false;
    let mut in_code = false;
    for line in lines {
        let t = line.trim();
        if is_fence(t) {
            in_code = !in_code;
            continue;
        }
        if t.is_empty() {
            continue;
        }
        if !seen_title && t.starts_with('#') {
            seen_title = true;
            continue;
        }
        out.push(truncate(&strip_markers(t), width));
        if out.len() >= max_lines {
            break;
        }
    }
    out
}

/// Remove the markup so a preview reads as prose.
fn strip_markers(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    // Drop a leading structural marker.
    let start = if let Some(n) = list_marker(s) {
        n
    } else if s.starts_with("> ") {
        2
    } else if let Some(n) = leading_run(s, '#') {
        (n + 1).min(chars.len())
    } else {
        0
    };
    i += start;
    while i < chars.len() {
        match chars[i] {
            '*' | '`' | '_' | '[' | ']' => i += 1,
            '(' => {
                // Swallow a link target entirely.
                match find(&chars, i, ')') {
                    Some(end) => i = end + 1,
                    None => {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.trim().to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('~');
    out
}

// ------------------------------------------------------------------ wrapping

/// Split a line into visual rows of at most `cols` characters, as `(start, end)`
/// character ranges.
///
/// Wrapping is a function of the raw text alone, not of the highlighting, which
/// is what lets the caret and the styled spans be mapped onto the same rows
/// afterwards without either one having to know about the other.
///
/// Breaks at the last space that fits; a word longer than the whole width is
/// broken mid-word rather than being allowed to overflow.
pub fn wrap_ranges(text: &str, cols: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let cols = cols.max(1);
    if chars.len() <= cols {
        return vec![(0, chars.len())];
    }

    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let hard_end = (start + cols).min(chars.len());
        if hard_end == chars.len() {
            rows.push((start, hard_end));
            break;
        }
        // Walk back to the last space that still fits.
        let mut end = hard_end;
        while end > start && !chars[end - 1].is_whitespace() {
            end -= 1;
        }
        if end == start {
            end = hard_end; // one very long word: break it
        }
        rows.push((start, end));
        start = end;
        // A wrapped row should not begin with the space it broke on.
        while start < chars.len() && chars[start] == ' ' && start < hard_end {
            start += 1;
        }
    }
    if rows.is_empty() {
        rows.push((0, 0));
    }
    rows
}

/// Take the part of `spans` covering the character range `from..to`.
pub fn slice_spans(spans: &[Span], from: usize, to: usize) -> Vec<Span> {
    let mut out = Vec::new();
    let mut pos = 0;
    for span in spans {
        let len = span.text.chars().count();
        let (s, e) = (pos, pos + len);
        pos = e;
        if e <= from || s >= to {
            continue;
        }
        let take_from = from.saturating_sub(s);
        let take_to = (to - s).min(len);
        let text: String = span
            .text
            .chars()
            .skip(take_from)
            .take(take_to - take_from)
            .collect();
        if !text.is_empty() {
            out.push(Span {
                text,
                tok: span.tok,
                bold: span.bold,
            });
        }
    }
    out
}

/// Which visual row `col` falls on, and its offset within that row.
pub fn locate(ranges: &[(usize, usize)], col: usize) -> (usize, usize) {
    for (i, (start, end)) in ranges.iter().enumerate() {
        // The caret may sit one past the end of the final row.
        let last = i + 1 == ranges.len();
        if col < *end || (last && col <= *end) {
            return (i, col.saturating_sub(*start));
        }
    }
    let (i, (start, end)) = (ranges.len() - 1, ranges[ranges.len() - 1]);
    (i, end.saturating_sub(start))
}
