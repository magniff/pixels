//! Applying a unified diff to a note, the way `patch` does.
//!
//! The model is shown the lines that changed in a note as a diff, in the
//! shape every diff has had since 1975. This is the other direction: a diff
//! it writes, applied. The hunks say where they go and what they expect to
//! find there; each is found by what it expects rather than trusted for where
//! it says, because a model's line numbers are the thing it gets wrong most
//! and the lines it quotes are the thing it gets wrong least.

/// One hunk of a diff: what it expects to find, and what to put there.
struct Hunk {
    /// Where the diff says the old lines start, counted from one.
    at: usize,
    /// The lines that must be there: context and removals, in order.
    old: Vec<String>,
    /// The lines to leave in their place: context and additions, in order.
    new: Vec<String>,
}

/// Read the hunks out of a diff.
///
/// Lenient about everything but the hunks themselves. Headers, prose before
/// the first `@@` and anything between hunks are ignored; a line inside a
/// hunk that starts with none of ` `, `-`, `+` is taken for context with its
/// space left off, because that is what a model does with context lines
/// more often than not.
fn hunks(patch: &str) -> Result<Vec<Hunk>, String> {
    let mut out: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    // Written without hunk headers, which both models did the first time
    // asked: lines marked - and + and nothing else. Each run of removals
    // that follows a run of additions is a change of its own, found by its
    // own lines - two changes ten lines apart cannot be one hunk, because
    // one hunk's lines have to sit together in the note.
    let headerless = !patch.lines().any(|l| l.starts_with("@@"));
    let mut adding = false;
    for line in patch.lines() {
        if headerless {
            let starts_change = line.starts_with('-') && adding;
            if current.is_none() || starts_change {
                if let Some(h) = current.take() {
                    out.push(h);
                }
                current = Some(Hunk {
                    at: 1,
                    old: Vec::new(),
                    new: Vec::new(),
                });
            }
            adding = line.starts_with('+');
        }
        if let Some(rest) = line.strip_prefix("@@") {
            if let Some(h) = current.take() {
                out.push(h);
            }
            // `-12,3 +12,4` - only the first number is used, and only as a
            // hint about where to look first.
            let at = rest
                .trim()
                .trim_start_matches('-')
                .split([',', ' '])
                .next()
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            current = Some(Hunk {
                at,
                old: Vec::new(),
                new: Vec::new(),
            });
            continue;
        }
        let Some(h) = current.as_mut() else {
            continue;
        };
        if line.starts_with("\\ No newline") {
            continue;
        }
        match line.chars().next() {
            Some('-') => h.old.push(line[1..].to_string()),
            Some('+') => h.new.push(line[1..].to_string()),
            Some(' ') => {
                h.old.push(line[1..].to_string());
                h.new.push(line[1..].to_string());
            }
            None => {
                h.old.push(String::new());
                h.new.push(String::new());
            }
            Some(_) => {
                h.old.push(line.to_string());
                h.new.push(line.to_string());
            }
        }
    }
    if let Some(h) = current.take() {
        out.push(h);
    }
    // A space after the marker - `- text`, `+ text` - is a habit models have
    // from lists, and it is not part of the line. When every marked line in
    // a hunk has it, it goes; when only some do, they are what they say.
    for h in &mut out {
        let marked: Vec<&String> = h.old.iter().chain(h.new.iter()).collect();
        if !marked.is_empty() && marked.iter().all(|l| l.starts_with(' ') || l.is_empty()) {
            for l in h.old.iter_mut().chain(h.new.iter_mut()) {
                if let Some(rest) = l.strip_prefix(' ') {
                    *l = rest.to_string();
                }
            }
        }
    }
    // Trailing blank context that the model added for air and the file does
    // not have is not something to insist on.
    for h in &mut out {
        while h.old.last().is_some_and(|l| l.is_empty())
            && h.new.last().is_some_and(|l| l.is_empty())
        {
            h.old.pop();
            h.new.pop();
        }
    }
    if out.is_empty() {
        return Err("there is no hunk in it - a patch begins each change with @@".into());
    }
    Ok(out)
}

/// Where a hunk's old lines are in the text, looking near where it says
/// first and then everywhere.
fn place(lines: &[String], old: &[String], hint: usize) -> Option<usize> {
    if old.is_empty() {
        return Some(hint.saturating_sub(1).min(lines.len()));
    }
    if old.len() > lines.len() {
        return None;
    }
    let fits = |i: usize| lines[i..i + old.len()] == *old;
    let last = lines.len() - old.len();
    let hint = hint.saturating_sub(1).min(last);
    // Nearest first: the same place it said, then one line either way, and
    // outwards, so a hunk that fits in two places lands where it was meant.
    for d in 0..=last {
        if hint >= d && fits(hint - d) {
            return Some(hint - d);
        }
        if hint + d <= last && fits(hint + d) {
            return Some(hint + d);
        }
    }
    None
}

/// The text with the diff applied, or which hunk did not fit.
pub fn apply(before: &str, patch: &str) -> Result<String, String> {
    let hunks = hunks(patch)?;
    let mut lines: Vec<String> = before.split('\n').map(str::to_string).collect();
    // Applied in order, with each hunk's hint moved by what the ones before
    // it added or took away.
    let mut shift: i64 = 0;
    for (n, h) in hunks.iter().enumerate() {
        let hint = (h.at as i64 + shift).max(1) as usize;
        let Some(at) = place(&lines, &h.old, hint) else {
            return Err(format!(
                "hunk {} does not fit: the note does not have these lines together:\n{}",
                n + 1,
                h.old.join("\n")
            ));
        };
        lines.splice(at..at + h.old.len(), h.new.iter().cloned());
        shift += h.new.len() as i64 - h.old.len() as i64;
    }
    Ok(lines.join("\n"))
}
