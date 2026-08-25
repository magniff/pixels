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
    /// `~~struck out~~`.
    Strike,
    /// An image, which cannot be drawn — its alt text stands in for it.
    Image,
    // Inside a fenced code block, from the syntax highlighter.
    /// Code with no more specific meaning — not the same as an inline code
    /// span, which is coloured to stand out from the prose around it.
    CodePlain,
    CodeKeyword,
    CodeType,
    CodeFunction,
    CodeString,
    CodeNumber,
    CodeComment,
    CodePunct,
}

/// A run of characters sharing one token type.
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub tok: Tok,
    pub bold: bool,
    /// Where a [`Tok::Link`] points. Carried on the span rather than left in
    /// the marker run beside it, because the rendering throws the markers away
    /// and the target has to survive that to be clickable.
    pub href: Option<String>,
}

impl Span {
    fn new(text: impl Into<String>, tok: Tok, bold: bool) -> Self {
        Self {
            text: text.into(),
            tok,
            bold,
            href: None,
        }
    }

    /// A link's label, carrying where it points.
    fn link(text: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            href: Some(href.into()),
            ..Self::new(text, Tok::Link, false)
        }
    }
}

/// Whether a line opens or closes a fenced code block.
pub fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
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

    // ---- rule, or the underline that makes a heading of the line above ----
    if !trimmed.is_empty() && trimmed.chars().all(|c| c == '=') {
        out.push(Span::new(trimmed, Tok::Marker, false));
        return out;
    }
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

/// Where a document's reference-style links point.
///
/// Collected in a pass over the whole document before anything else, because
/// `[text][ref]` is allowed to appear above the `[ref]: …` line defining it.
pub type Refs = std::collections::HashMap<String, String>;

/// Emphasis inherited from the runs a span sits inside.
///
/// Carried down the recursion rather than applied at the end, so `**bold with
/// *italic* inside**` comes out as both instead of as whichever nesting level
/// happened to be parsed last.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Emph {
    bold: bool,
    italic: bool,
    strike: bool,
}

impl Emph {
    /// The token plain text takes under this emphasis. Strike wins the colour
    /// because it is the one that also draws something.
    fn tok(self) -> Tok {
        if self.strike {
            Tok::Strike
        } else if self.bold {
            Tok::Bold
        } else if self.italic {
            Tok::Italic
        } else {
            Tok::Text
        }
    }
}

/// Parse inline markup inside one run of body text.
fn inline(s: &str) -> Vec<Span> {
    inline_with(s, &Refs::new())
}

/// The same, with the document's link definitions available.
fn inline_with(s: &str, refs: &Refs) -> Vec<Span> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    scan(&chars, refs, Emph::default(), &mut out);
    out
}

fn take(c: &[char], a: usize, b: usize) -> String {
    c[a.min(c.len())..b.min(c.len())].iter().collect()
}

/// How many of `d` start at `at`.
fn run_len(c: &[char], at: usize, d: char) -> usize {
    c[at..].iter().take_while(|&&x| x == d).count()
}

/// The start of the next run of exactly `n` of `d`, at or after `from`.
///
/// Exactly, not at least: a code span opened with one backtick is closed by
/// one, and a longer run inside it is content — which is the whole point of
/// being allowed to write ``a ` b``.
fn closing_run(c: &[char], from: usize, d: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i < c.len() {
        if c[i] == '\\' {
            i += 2;
            continue;
        }
        if c[i] == d {
            let len = run_len(c, i, d);
            if len == n {
                return Some(i);
            }
            i += len;
            continue;
        }
        i += 1;
    }
    None
}

/// Whether a delimiter run at `at` can open emphasis.
///
/// The run must be followed by something other than a space, or `a * b * c`
/// turns into italics. An underscore additionally may not sit inside a word,
/// which is what keeps `snake_case_names` intact.
fn can_open(c: &[char], at: usize, n: usize) -> bool {
    let after = c.get(at + n);
    if !after.is_some_and(|x| !x.is_whitespace()) {
        return false;
    }
    if c[at] == '_' {
        let before = at.checked_sub(1).and_then(|i| c.get(i));
        if before.is_some_and(|x| x.is_alphanumeric()) {
            return false;
        }
    }
    true
}

/// The start of the run of `d` that closes emphasis opened with `n` of them.
fn closing_emph(c: &[char], from: usize, d: char, n: usize) -> Option<usize> {
    let mut i = from;
    while i < c.len() {
        if c[i] == '\\' {
            i += 2;
            continue;
        }
        if c[i] == d {
            let len = run_len(c, i, d);
            let before_ok = i > from && !c[i - 1].is_whitespace();
            let word_ok = d != '_' || !c.get(i + len).is_some_and(|x| x.is_alphanumeric());
            if len >= n && before_ok && word_ok {
                return Some(i);
            }
            i += len;
            continue;
        }
        i += 1;
    }
    None
}

/// The `]` matching the `[` at `open`, allowing for brackets in the label.
fn find_bracket(c: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < c.len() {
        match c[i] {
            '\\' => i += 1,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Punctuation a backslash may escape. Escaping anything else is a literal
/// backslash, which is how `C:\path` survives being written down.
fn is_escapable(c: char) -> bool {
    "\\`*_{}[]()#+-.!<>|~\"'".contains(c)
}

/// Read a link destination and optional title starting after a `(`, and
/// return the destination with the index just past the closing `)`.
fn link_dest(c: &[char], from: usize) -> Option<(String, usize)> {
    let mut i = from;
    while c.get(i).is_some_and(|x| x.is_whitespace()) {
        i += 1;
    }
    let dest = if c.get(i) == Some(&'<') {
        let end = find(c, i + 1, '>')?;
        let d = take(c, i + 1, end);
        i = end + 1;
        d
    } else {
        // Bare destinations end at whitespace, or at the `)` that closes the
        // link — but parentheses inside one are allowed as long as they pair.
        let start = i;
        let mut depth = 0i32;
        while let Some(&ch) = c.get(i) {
            if ch.is_whitespace() {
                break;
            }
            if ch == '(' {
                depth += 1;
            }
            if ch == ')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            i += 1;
        }
        take(c, start, i)
    };
    while c.get(i).is_some_and(|x| x.is_whitespace()) {
        i += 1;
    }
    // A title, which is for a tooltip nobody here can show, so it is read only
    // to be stepped over.
    if let Some(&q) = c.get(i) {
        if q == '"' || q == '\'' || q == '(' {
            let close = if q == '(' { ')' } else { q };
            let end = find(c, i + 1, close)?;
            i = end + 1;
            while c.get(i).is_some_and(|x| x.is_whitespace()) {
                i += 1;
            }
        }
    }
    (c.get(i) == Some(&')')).then(|| (dest, i + 1))
}

/// A reference label, in the form definitions are keyed by.
fn norm_label(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Whether the inside of `<…>` is an autolink, and where it points.
fn autolink(body: &str) -> Option<String> {
    if body.is_empty() || body.chars().any(char::is_whitespace) {
        return None;
    }
    if let Some((scheme, _)) = body.split_once("://") {
        if !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+')
        {
            return Some(body.to_string());
        }
    }
    if body.starts_with("mailto:") {
        return Some(body.to_string());
    }
    // An email address, which is written bare and linked as mailto.
    let (user, host) = body.split_once('@')?;
    (!user.is_empty() && host.contains('.') && !host.starts_with('.'))
        .then(|| format!("mailto:{body}"))
}

/// How far a bare URL starting at `at` runs.
///
/// Trailing punctuation is left out: a sentence ending in a link is far more
/// common than a URL that really ends in a full stop, and the closing bracket
/// of `(see https://x)` belongs to the prose.
fn bare_url_end(c: &[char], at: usize) -> usize {
    let mut i = at;
    while let Some(&ch) = c.get(i) {
        if ch.is_whitespace() || ch == '<' || ch == '>' || ch == '"' {
            break;
        }
        i += 1;
    }
    while i > at {
        let last = c[i - 1];
        if ".,;:!?".contains(last) {
            i -= 1;
            continue;
        }
        if last == ')' && take(c, at, i).matches('(').count() < take(c, at, i).matches(')').count()
        {
            i -= 1;
            continue;
        }
        break;
    }
    i
}

/// Whether a bare URL may start at `at` — only at the start of a word.
fn at_word_start(c: &[char], at: usize) -> bool {
    at == 0 || !c[at - 1].is_alphanumeric()
}

fn flush(plain: &mut String, emph: Emph, out: &mut Vec<Span>) {
    if !plain.is_empty() {
        out.push(Span::new(std::mem::take(plain), emph.tok(), emph.bold));
    }
}

/// Turn the label of a link into spans that carry where it points.
fn as_link(mut spans: Vec<Span>, dest: &str, emph: Emph) -> Vec<Span> {
    for span in &mut spans {
        if span.href.is_none() {
            span.href = Some(dest.to_string());
        }
        if span.tok == emph.tok() {
            span.tok = Tok::Link;
        }
    }
    spans
}

/// The one scanner, run recursively so emphasis nests.
///
/// Delimiters are emitted as their own dim `Marker` spans rather than folded
/// into the styled run, because the source view draws these spans too and the
/// markup there is text you can put a caret in.
fn scan(c: &[char], refs: &Refs, emph: Emph, out: &mut Vec<Span>) {
    let mut plain = String::new();
    let mut i = 0;

    while i < c.len() {
        // ---- backslash escape --------------------------------------------
        if c[i] == '\\' {
            if let Some(&next) = c.get(i + 1) {
                if is_escapable(next) {
                    flush(&mut plain, emph, out);
                    out.push(Span::new("\\", Tok::Marker, false));
                    plain.push(next);
                    i += 2;
                    continue;
                }
            }
        }

        // ---- `code` ------------------------------------------------------
        if c[i] == '`' {
            let n = run_len(c, i, '`');
            if let Some(end) = closing_run(c, i + n, '`', n) {
                flush(&mut plain, emph, out);
                let ticks: String = std::iter::repeat_n('`', n).collect();
                out.push(Span::new(ticks.clone(), Tok::Marker, false));
                // Code is literal: no escapes, no emphasis, one trimmed space
                // either side stripped so `` ` `` can hold a backtick.
                let body = take(c, i + n, end);
                let body = match (body.starts_with(' '), body.ends_with(' ')) {
                    (true, true) if body.trim().len() + 2 <= body.len() => {
                        body[1..body.len() - 1].to_string()
                    }
                    _ => body,
                };
                out.push(Span::new(body, Tok::Code, emph.bold));
                out.push(Span::new(ticks, Tok::Marker, false));
                i = end + n;
                continue;
            }
        }

        // ---- ~~struck out~~ ----------------------------------------------
        if c[i] == '~' && c.get(i + 1) == Some(&'~') && !emph.strike {
            if let Some(end) = closing_emph(c, i + 2, '~', 2) {
                flush(&mut plain, emph, out);
                out.push(Span::new("~~", Tok::Marker, false));
                let inner = Emph {
                    strike: true,
                    ..emph
                };
                scan(&c[i + 2..end], refs, inner, out);
                out.push(Span::new("~~", Tok::Marker, false));
                i = end + 2;
                continue;
            }
        }

        // ---- *emphasis* and _emphasis_ -----------------------------------
        if (c[i] == '*' || c[i] == '_') && can_open(c, i, run_len(c, i, c[i]).min(3)) {
            let d = c[i];
            // One delimiter is italic, two are bold, three are both — and a
            // longer run than that is three with the rest as text, which is
            // what anybody writing four asterisks meant.
            let n = run_len(c, i, d).min(3);
            let (bold, italic) = (n >= 2, n != 2);
            let wanted = (bold && !emph.bold) || (italic && !emph.italic);
            if wanted {
                if let Some(end) = closing_emph(c, i + n, d, n) {
                    flush(&mut plain, emph, out);
                    let run: String = std::iter::repeat_n(d, n).collect();
                    out.push(Span::new(run.clone(), Tok::Marker, false));
                    let inner = Emph {
                        bold: emph.bold || bold,
                        italic: emph.italic || italic,
                        ..emph
                    };
                    scan(&c[i + n..end], refs, inner, out);
                    out.push(Span::new(run, Tok::Marker, false));
                    i = end + n;
                    continue;
                }
            }
        }

        // ---- <autolink> --------------------------------------------------
        if c[i] == '<' {
            if let Some(end) = find(c, i + 1, '>') {
                let body = take(c, i + 1, end);
                if let Some(dest) = autolink(&body) {
                    flush(&mut plain, emph, out);
                    out.push(Span::new("<", Tok::Marker, false));
                    out.push(Span::link(body, dest));
                    out.push(Span::new(">", Tok::Marker, false));
                    i = end + 1;
                    continue;
                }
            }
        }

        // ---- ![image](src) and [link](target) ----------------------------
        let image = c[i] == '!' && c.get(i + 1) == Some(&'[');
        let open = if image { i + 1 } else { i };
        if c.get(open) == Some(&'[') {
            if let Some(close) = find_bracket(c, open) {
                let label = take(c, open + 1, close);
                let resolved = match c.get(close + 1) {
                    // [label](target)
                    Some('(') => link_dest(c, close + 2),
                    // [label][ref], and [label][] which reuses the label
                    Some('[') => find(c, close + 2, ']').and_then(|end2| {
                        let key = if end2 == close + 2 {
                            label.clone()
                        } else {
                            take(c, close + 2, end2)
                        };
                        refs.get(&norm_label(&key)).map(|d| (d.clone(), end2 + 1))
                    }),
                    // [label] on its own, if something defines it
                    _ => refs
                        .get(&norm_label(&label))
                        .map(|d| (d.clone(), close + 1)),
                };
                if let Some((dest, end)) = resolved {
                    flush(&mut plain, emph, out);
                    if image {
                        out.push(Span::new("![", Tok::Marker, false));
                        out.push(Span::new(label, Tok::Image, emph.bold));
                    } else {
                        out.push(Span::new("[", Tok::Marker, false));
                        let mut inner = Vec::new();
                        scan(&c[open + 1..close], refs, emph, &mut inner);
                        out.extend(as_link(inner, &dest, emph));
                    }
                    out.push(Span::new(take(c, close, end), Tok::Marker, false));
                    i = end;
                    continue;
                }
            }
        }

        // ---- a bare URL, which people write far more often than they link --
        if at_word_start(c, i) {
            let rest: String = take(c, i, (i + 8).min(c.len()));
            if rest.starts_with("http://")
                || rest.starts_with("https://")
                || rest.starts_with("www.")
            {
                let end = bare_url_end(c, i);
                let text = take(c, i, end);
                if end > i && text.len() > 5 {
                    flush(&mut plain, emph, out);
                    let dest = if text.starts_with("www.") {
                        format!("https://{text}")
                    } else {
                        text.clone()
                    };
                    out.push(Span::link(text, dest));
                    i = end;
                    continue;
                }
            }
        }

        plain.push(c[i]);
        i += 1;
    }
    flush(&mut plain, emph, out);
}

fn find(chars: &[char], from: usize, target: char) -> Option<usize> {
    chars[from.min(chars.len())..]
        .iter()
        .position(|c| *c == target)
        .map(|i| i + from)
}

/// A title for a note: its first heading, else its first non-empty line.
pub fn derive_title(lines: &[String]) -> String {
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('#') {
            let title = rest.trim_start_matches('#').trim();
            if !title.is_empty() {
                return truncate(title, 24);
            }
        }
        // A heading may also be written as a line with a rule under it, and
        // that form is usually the one at the very top of a file.
        if !t.is_empty() && lines.get(i + 1).is_some_and(|n| setext_level(n).is_some()) {
            return truncate(t, 24);
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
    let forced = chars.iter().position(|c| *c == '\n');
    if chars.len() <= cols && forced.is_none() {
        return vec![(0, chars.len())];
    }

    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        // A newline in the middle of a paragraph is a hard line break, which
        // markdown writes as two trailing spaces or a trailing backslash. It
        // ends the row wherever it falls, and is not part of either row.
        if let Some(brk) = chars[start..].iter().position(|c| *c == '\n') {
            let brk = start + brk;
            if brk < start + cols {
                rows.push((start, brk));
                start = brk + 1;
                continue;
            }
        }
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
                href: span.href.clone(),
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

// ------------------------------------------------------------ document model

/// How a table column is aligned, from its `---`/`:-:` separator row.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CellAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// What sits at the head of a list item.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Marker {
    Bullet,
    Number(usize),
    /// A task item, checked or not.
    Task(bool),
}

#[derive(Clone, Debug)]
pub struct Item {
    pub marker: Marker,
    /// Nesting level, from the leading indent.
    pub depth: usize,
    pub spans: Vec<Span>,
    /// The source line the item's marker is on. An item is a row of its own in
    /// the rendering, so it carries its own number rather than borrowing the
    /// list's.
    pub line: usize,
}

/// One block of a parsed document.
///
/// The highlighter deals in lines because that is what an editor draws. A
/// *rendering* has to deal in blocks, because a paragraph is not a line and a
/// list is not its items — the shape of the thing is the whole point.
#[derive(Clone, Debug)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    List(Vec<Item>),
    /// A quote holds blocks, not lines: everything markdown has can go
    /// inside one. Located, like the document itself — a heading inside a
    /// quote was typed on a line like any other.
    Quote(Vec<Located>),
    Code {
        lang: String,
        lines: Vec<String>,
        /// The source line `lines[0]` came from — past the fence, if there was
        /// one. Every line below it follows in order, so one number is enough
        /// to number the whole slab.
        first: usize,
    },
    Table {
        align: Vec<CellAlign>,
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
    Rule,
}

/// Inline spans with the markup taken out, for rendering rather than editing.
///
/// The highlighter keeps `**` and backticks visible and dims them, because in
/// the source view they are text you can put a caret in. Rendered, they are
/// instructions that have already been carried out.
pub fn inline_spans(text: &str) -> Vec<Span> {
    inline(text)
        .into_iter()
        .filter(|s| s.tok != Tok::Marker)
        .collect()
}

/// Whether a line is a horizontal rule.
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

/// The cells of a table row, if the line looks like one.
fn table_cells(line: &str) -> Option<Vec<String>> {
    let t = line.trim();
    if !t.contains('|') {
        return None;
    }
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    Some(t.split('|').map(|c| c.trim().to_string()).collect())
}

/// A `| --- | :-: |` row, which is what turns the line above it into a header.
fn table_alignment(line: &str) -> Option<Vec<CellAlign>> {
    let cells = table_cells(line)?;
    let mut out = Vec::new();
    for cell in &cells {
        let c = cell.trim();
        if c.len() < 3 || !c.chars().all(|ch| ch == '-' || ch == ':') || !c.contains('-') {
            return None;
        }
        out.push(match (c.starts_with(':'), c.ends_with(':')) {
            (true, true) => CellAlign::Center,
            (false, true) => CellAlign::Right,
            _ => CellAlign::Left,
        });
    }
    (!out.is_empty()).then_some(out)
}

/// The marker at the head of a list item, and how far past it the text starts.
pub(crate) fn item_marker(line: &str) -> Option<(Marker, usize)> {
    let trimmed = line.trim_start();
    let rest = if let Some(r) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        r
    } else {
        // An ordered item: digits, then `.` or `)`, then a space.
        let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 9 {
            return None;
        }
        let after = &trimmed[digits..];
        let rest = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))?;
        let n = trimmed[..digits].parse().unwrap_or(1);
        let used = line.len() - rest.len();
        return Some((Marker::Number(n), used));
    };

    // A task box turns a bullet into a checkbox.
    let (marker, rest) = if let Some(r) = rest.strip_prefix("[ ] ") {
        (Marker::Task(false), r)
    } else if let Some(r) = rest
        .strip_prefix("[x] ")
        .or_else(|| rest.strip_prefix("[X] "))
    {
        (Marker::Task(true), r)
    } else {
        (Marker::Bullet, rest)
    };
    Some((marker, line.len() - rest.len()))
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Parse lines into blocks.
pub type Located = (usize, Block);

pub fn parse(lines: &[String]) -> Vec<Block> {
    parse_located(lines).into_iter().map(|(_, b)| b).collect()
}

/// Parse lines into blocks, each paired with the source line it began on.
///
/// The preview numbers its gutter with these, so the two views of a note are
/// numbered in the same terms and a paragraph on screen can be found in the
/// text it came from. Only the top level is numbered: a quote's contents are
/// parsed out of lines that have had their markers stripped, and counting
/// those would be counting a document that was never typed.
pub fn parse_located(lines: &[String]) -> Vec<Located> {
    let (refs, defs) = link_defs(lines);
    let kept: Vec<String> = lines
        .iter()
        .zip(&defs)
        .map(|(l, is_def)| if *is_def { String::new() } else { l.clone() })
        .collect();
    parse_located_blocks(&kept, &refs)
}

/// Collect the document's link definitions, and say which lines were spent on
/// them. They are not content, so the caller blanks those lines out.
fn link_defs(lines: &[String]) -> (Refs, Vec<bool>) {
    let mut refs = Refs::new();
    let mut used = vec![false; lines.len()];
    let mut in_code = false;
    for (i, line) in lines.iter().enumerate() {
        if is_fence(line) {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if let Some((label, dest)) = link_def(line) {
            refs.entry(norm_label(&label)).or_insert(dest);
            used[i] = true;
        }
    }
    (refs, used)
}

/// `[label]: destination "title"` — a line that defines a link rather than
/// being one.
fn link_def(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let rest = t.strip_prefix('[')?;
    let close = rest.find("]:")?;
    let label = &rest[..close];
    let after = rest[close + 2..].trim();
    if label.is_empty() || after.is_empty() {
        return None;
    }
    let dest = after.split_whitespace().next()?;
    Some((
        label.to_string(),
        dest.trim_start_matches('<')
            .trim_end_matches('>')
            .to_string(),
    ))
}

/// Inline spans with the markup taken out, resolving reference links.
pub fn inline_spans_with(text: &str, refs: &Refs) -> Vec<Span> {
    inline_with(text, refs)
        .into_iter()
        .filter(|s| s.tok != Tok::Marker)
        .collect()
}

/// A fence's delimiter, its length, and its info string.
fn fence_info(line: &str) -> Option<(char, usize, String)> {
    let t = line.trim_start();
    let d = t.chars().next()?;
    if d != '`' && d != '~' {
        return None;
    }
    let n = t.chars().take_while(|c| *c == d).count();
    if n < 3 {
        return None;
    }
    let info = t[n..].trim().to_string();
    // A backtick fence's info string may not contain a backtick, or every
    // `a ` b` in a paragraph would open a code block.
    if d == '`' && info.contains('`') {
        return None;
    }
    Some((d, n, info))
}

/// Whether `line` closes a fence opened with `n` of `d`.
fn closes_fence(line: &str, d: char, n: usize) -> bool {
    let t = line.trim();
    t.chars().all(|c| c == d) && t.chars().count() >= n && !t.is_empty()
}

/// The heading level a setext underline gives the paragraph above it.
fn setext_level(line: &str) -> Option<u8> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    if t.chars().all(|c| c == '=') {
        return Some(1);
    }
    (t.chars().all(|c| c == '-') && t.len() >= 2).then_some(2)
}

/// Whether a line is indented enough to be a code block on its own.
fn is_indented_code(line: &str) -> bool {
    !line.trim().is_empty() && (line.starts_with("    ") || line.starts_with('\t'))
}

/// Whether a line starts something that a paragraph cannot swallow.
fn starts_block(line: &str) -> bool {
    let t = line.trim();
    t.is_empty()
        || fence_info(line).is_some()
        || is_rule(line)
        || item_marker(line).is_some()
        || t.starts_with('>')
        || leading_run(t, '#').is_some_and(|n| n <= 6 && t[n..].starts_with(' '))
}

/// How a paragraph line ends: markdown's two ways of asking for a line break.
fn hard_break(line: &str) -> bool {
    line.ends_with("  ") || line.ends_with('\\')
}

fn parse_located_blocks(lines: &[String], refs: &Refs) -> Vec<Located> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Where whatever this pass produces began. Each pass emits at most one
        // block, so this is that block's line.
        let start = i;
        let line = &lines[i];
        let trimmed = line.trim();

        // ---- blank ---------------------------------------------------
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // ---- fenced code ---------------------------------------------
        if let Some((d, n, lang)) = fence_info(line) {
            i += 1;
            let mut body = Vec::new();
            while i < lines.len() && !closes_fence(&lines[i], d, n) {
                body.push(lines[i].clone());
                i += 1;
            }
            i += 1; // the closing fence, if there was one
            blocks.push((
                start,
                Block::Code {
                    lang,
                    lines: body,
                    first: start + 1,
                },
            ));
            continue;
        }

        // ---- indented code -------------------------------------------
        // Four spaces is only code where a paragraph could have started;
        // inside one it is a continuation line somebody lined up.
        if is_indented_code(line) {
            let mut body = Vec::new();
            while i < lines.len() && (is_indented_code(&lines[i]) || lines[i].trim().is_empty()) {
                // Trailing blank lines belong to whatever comes next.
                if lines[i].trim().is_empty() && !lines[i + 1..].iter().any(|l| is_indented_code(l))
                {
                    break;
                }
                let stripped = lines[i]
                    .strip_prefix("    ")
                    .or_else(|| lines[i].strip_prefix('\t'))
                    .unwrap_or("")
                    .to_string();
                body.push(stripped);
                i += 1;
            }
            blocks.push((
                start,
                Block::Code {
                    lang: String::new(),
                    lines: body,
                    // No fence to skip past: the first line of an indented
                    // block is the block.
                    first: start,
                },
            ));
            continue;
        }

        // ---- rule ----------------------------------------------------
        if is_rule(line) {
            blocks.push((start, Block::Rule));
            i += 1;
            continue;
        }

        // ---- ATX heading ---------------------------------------------
        if let Some(hashes) = leading_run(trimmed, '#') {
            if hashes <= 6 && trimmed[hashes..].starts_with(' ') {
                // A closing run of hashes is decoration, not content.
                let body = trimmed[hashes + 1..].trim();
                let body = body.trim_end_matches('#').trim_end();
                blocks.push((
                    start,
                    Block::Heading {
                        level: hashes as u8,
                        spans: inline_spans_with(body, refs),
                    },
                ));
                i += 1;
                continue;
            }
        }

        // ---- table ---------------------------------------------------
        // A row only becomes a table if the line under it is an alignment
        // row; otherwise it is a paragraph that happens to contain pipes.
        if let (Some(header), Some(align)) = (
            table_cells(line),
            lines.get(i + 1).and_then(|l| table_alignment(l)),
        ) {
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() {
                match table_cells(&lines[i]) {
                    Some(cells) if !lines[i].trim().is_empty() => {
                        rows.push(cells.iter().map(|c| inline_spans_with(c, refs)).collect());
                        i += 1;
                    }
                    _ => break,
                }
            }
            blocks.push((
                start,
                Block::Table {
                    align,
                    header: header.iter().map(|c| inline_spans_with(c, refs)).collect(),
                    rows,
                },
            ));
            continue;
        }

        // ---- quote ---------------------------------------------------
        // Everything markdown has goes inside a quote — headings, lists,
        // code — so the contents are stripped of one `>` and parsed again
        // rather than being treated as a run of loose lines.
        if trimmed.starts_with('>') {
            let mut body = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim_start();
                if let Some(rest) = t.strip_prefix('>') {
                    body.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
                    i += 1;
                } else if !t.is_empty() && !starts_block(&lines[i]) && !body.is_empty() {
                    // A lazy continuation: a quoted paragraph may run on
                    // without repeating the marker.
                    body.push(t.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            // The body was built one entry per line consumed, so an index
            // into it is that many lines below where the quote began.
            let inner = parse_located_blocks(&body, refs)
                .into_iter()
                .map(|(n, b)| (start + n, b))
                .collect();
            blocks.push((start, Block::Quote(inner)));
            continue;
        }

        // ---- list ----------------------------------------------------
        if item_marker(line).is_some() {
            let (items, next) = parse_list(lines, i, refs);
            blocks.push((start, Block::List(items)));
            i = next;
            continue;
        }

        // ---- paragraph -----------------------------------------------
        // Consecutive plain lines are one paragraph: a soft wrap in the
        // source is not a line break in the output, but two trailing spaces
        // or a trailing backslash asks for one.
        let mut text = String::new();
        let mut level = None;
        while i < lines.len() {
            let l = &lines[i];
            // An underline turns everything above it into a heading. Checked
            // before anything else, because a row of dashes is a rule on its
            // own and an underline under a paragraph, and here there is a
            // paragraph.
            if !text.is_empty() {
                if let Some(n) = setext_level(l) {
                    level = Some(n);
                    i += 1;
                    break;
                }
            }
            if starts_block(l) {
                break;
            }
            if !text.is_empty() {
                text.push(if hard_break(&lines[i - 1]) { '\n' } else { ' ' });
            }
            // The break marker asks for a line break; it is not part of the
            // sentence it ends. Trailing spaces are gone with the trim.
            let chunk = l.trim();
            text.push_str(if hard_break(l) {
                chunk.trim_end_matches('\\').trim_end()
            } else {
                chunk
            });
            i += 1;
        }
        if !text.is_empty() {
            blocks.push((
                start,
                match level {
                    Some(level) => Block::Heading {
                        level,
                        spans: inline_spans_with(&text, refs),
                    },
                    None => Block::Paragraph(inline_spans_with(&text, refs)),
                },
            ));
        }
    }

    blocks
}

/// Read one list, from the item at `start` to the first line that is not part
/// of it, and say where that was.
///
/// An item is its marker line plus every line that continues it: anything
/// indented past the marker, and anything that simply runs on without starting
/// a block of its own. A blank line does not end the list as long as another
/// item follows — that is a loose list, not two lists.
fn parse_list(lines: &[String], start: usize, refs: &Refs) -> (Vec<Item>, usize) {
    let mut items: Vec<Item> = Vec::new();
    let mut i = start;

    while i < lines.len() {
        let Some((marker, used)) = item_marker(&lines[i]) else {
            // A blank line only ends the list if no further item follows it.
            if lines[i].trim().is_empty() {
                let more = lines[i + 1..]
                    .iter()
                    .find(|l| !l.trim().is_empty())
                    .is_some_and(|l| item_marker(l).is_some() || indent_of(l) >= 2);
                if more {
                    i += 1;
                    continue;
                }
            }
            break;
        };
        let indent = indent_of(&lines[i]);
        let mut text = lines[i][used..].trim().to_string();
        let line = i;
        i += 1;

        // Continuation lines: indented past the marker, or a plain run-on.
        while i < lines.len() {
            let l = &lines[i];
            if l.trim().is_empty() || item_marker(l).is_some() || starts_block(l) {
                break;
            }
            if indent_of(l) <= indent && !text.is_empty() && indent_of(l) == 0 && indent > 0 {
                break;
            }
            text.push(' ');
            text.push_str(l.trim());
            i += 1;
        }

        items.push(Item {
            marker,
            depth: indent / 2,
            spans: inline_spans_with(&text, refs),
            line,
        });
    }

    (items, i)
}
