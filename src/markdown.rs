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
        // ~~struck out~~
        if bytes[i] == '~' && bytes.get(i + 1) == Some(&'~') {
            if let Some(end) = find_pair_of(&bytes, i + 2, '~') {
                flush(&mut plain, &mut out);
                out.push(Span::new("~~", Tok::Marker, false));
                out.push(Span::new(take(&bytes, i + 2, end), Tok::Strike, false));
                out.push(Span::new("~~", Tok::Marker, false));
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
        // ![alt](source)
        if bytes[i] == '!' && bytes.get(i + 1) == Some(&'[') {
            if let Some(close) = find(&bytes, i + 2, ']') {
                if bytes.get(close + 1) == Some(&'(') {
                    if let Some(paren) = find(&bytes, close + 2, ')') {
                        flush(&mut plain, &mut out);
                        out.push(Span::new("![", Tok::Marker, false));
                        out.push(Span::new(take(&bytes, i + 2, close), Tok::Image, false));
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
    find_pair_of(chars, from, '*')
}

/// Find the next doubled `c`.
fn find_pair_of(chars: &[char], from: usize, c: char) -> Option<usize> {
    (from..chars.len().saturating_sub(1)).find(|&i| chars[i] == c && chars[i + 1] == c)
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
    Quote(Vec<Vec<Span>>),
    Code {
        lang: String,
        lines: Vec<String>,
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
fn item_marker(line: &str) -> Option<(Marker, usize)> {
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
pub fn parse(lines: &[String]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = &lines[i];
        let trimmed = line.trim();

        // ---- blank ---------------------------------------------------
        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // ---- fenced code ---------------------------------------------
        if is_fence(line) {
            let lang = trimmed.trim_start_matches('`').trim().to_string();
            i += 1;
            let mut body = Vec::new();
            while i < lines.len() && !is_fence(&lines[i]) {
                body.push(lines[i].clone());
                i += 1;
            }
            i += 1; // the closing fence
            blocks.push(Block::Code { lang, lines: body });
            continue;
        }

        // ---- rule ----------------------------------------------------
        if is_rule(line) {
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }

        // ---- heading -------------------------------------------------
        if let Some(hashes) = leading_run(trimmed, '#') {
            if hashes <= 6 && trimmed[hashes..].starts_with(' ') {
                blocks.push(Block::Heading {
                    level: hashes as u8,
                    spans: inline_spans(trimmed[hashes + 1..].trim()),
                });
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
                        rows.push(cells.iter().map(|c| inline_spans(c)).collect());
                        i += 1;
                    }
                    _ => break,
                }
            }
            blocks.push(Block::Table {
                align,
                header: header.iter().map(|c| inline_spans(c)).collect(),
                rows,
            });
            continue;
        }

        // ---- quote ---------------------------------------------------
        if trimmed.starts_with('>') {
            let mut body = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let text = lines[i].trim_start().trim_start_matches('>').trim_start();
                body.push(inline_spans(text));
                i += 1;
            }
            blocks.push(Block::Quote(body));
            continue;
        }

        // ---- list ----------------------------------------------------
        if item_marker(line).is_some() {
            let mut items = Vec::new();
            while i < lines.len() {
                let Some((marker, used)) = item_marker(&lines[i]) else {
                    break;
                };
                items.push(Item {
                    marker,
                    depth: indent_of(&lines[i]) / 2,
                    spans: inline_spans(lines[i][used..].trim()),
                });
                i += 1;
            }
            blocks.push(Block::List(items));
            continue;
        }

        // ---- paragraph -----------------------------------------------
        // Consecutive plain lines are one paragraph: a hard wrap in the
        // source is not a line break in the output.
        let mut text = String::new();
        while i < lines.len() {
            let l = &lines[i];
            if l.trim().is_empty()
                || is_fence(l)
                || is_rule(l)
                || item_marker(l).is_some()
                || l.trim_start().starts_with('>')
                || leading_run(l.trim(), '#').is_some_and(|n| n <= 6)
            {
                break;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(l.trim());
            i += 1;
        }
        if !text.is_empty() {
            blocks.push(Block::Paragraph(inline_spans(&text)));
        }
    }

    blocks
}
