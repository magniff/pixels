//! Word-level differences between two pieces of prose.
//!
//! Line diffs are the wrong tool here: a rewritten paragraph is one line in the
//! source, so a line diff would say "all of it changed" and leave you reading
//! two paragraphs to find the three words that moved. Words are the unit the
//! suggestion is actually made in.

/// What happened to a run of words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Change {
    Same,
    Removed,
    Added,
}

/// A run of words that all changed the same way.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Piece {
    pub text: String,
    pub change: Change,
}

/// Beyond this many words on either side, the quadratic table is not worth
/// filling in: a suggestion that long is a rewrite, and reads better as one.
const LIMIT: usize = 1200;

/// Split into words, with line breaks kept as words of their own so that a
/// multi-line selection still diffs line by line.
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\n' {
            out.push("\n");
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(&text[start..i]);
        }
    }
    out
}

/// The difference between `before` and `after`, as runs of words.
pub fn words(before: &str, after: &str) -> Vec<Piece> {
    let a = tokens(before);
    let b = tokens(after);
    if a.len() > LIMIT || b.len() > LIMIT {
        return merge(
            a.iter()
                .map(|w| (Change::Removed, *w))
                .chain(b.iter().map(|w| (Change::Added, *w)))
                .collect(),
        );
    }

    // The classic longest-common-subsequence table, walked back from the far
    // corner. Both sides are bounded above, so the table is too.
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push((Change::Same, a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push((Change::Removed, a[i]));
            i += 1;
        } else {
            out.push((Change::Added, b[j]));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|w| (Change::Removed, *w)));
    out.extend(b[j..].iter().map(|w| (Change::Added, *w)));
    merge(out)
}

/// Gather neighbouring words that changed the same way into one piece, so the
/// renderer paints one band per run rather than one per word.
fn merge(words: Vec<(Change, &str)>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for (change, word) in words {
        match out.last_mut() {
            // A line break is its own piece whatever surrounds it: the renderer
            // breaks the row on it rather than drawing it.
            Some(last) if last.change == change && word != "\n" && last.text != "\n" => {
                last.text.push(' ');
                last.text.push_str(word);
            }
            _ => out.push(Piece {
                text: word.to_string(),
                change,
            }),
        }
    }
    out
}

/// Whether anything actually changed.
pub fn is_empty(pieces: &[Piece]) -> bool {
    pieces.iter().all(|p| p.change == Change::Same)
}
