//! The text buffer: lines, a cursor, and undo history.
//!
//! Positions are counted in **characters**, never bytes. The built-in font is
//! ASCII-only so today those are the same number, but indexing a `String` by a
//! byte offset that came from counting characters is a panic waiting for the
//! first person who types outside it.

/// A position in the buffer. `col` may equal the line length, which is where
/// the caret sits when appending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    pub line: usize,
    pub col: usize,
}

impl Cursor {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

#[derive(Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor: Cursor,
}

/// A list of lines plus everything needed to edit them.
pub struct Buffer {
    lines: Vec<String>,
    pub cursor: Cursor,
    /// Column `j`/`k` try to return to, so moving through a short line and out
    /// the other side lands back where you started rather than at its end.
    pub desired_col: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    pub dirty: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: Cursor::default(),
            desired_col: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
        }
    }

    pub fn from_text(text: &str) -> Self {
        let mut lines: Vec<String> = text
            .replace('\r', "")
            .split('\n')
            .map(str::to_owned)
            .collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            lines,
            ..Self::new()
        }
    }

    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, i: usize) -> &str {
        self.lines.get(i).map(String::as_str).unwrap_or("")
    }

    /// Length of line `i` in characters.
    pub fn line_len(&self, i: usize) -> usize {
        self.line(i).chars().count()
    }

    fn chars(&self, i: usize) -> Vec<char> {
        self.line(i).chars().collect()
    }

    fn set_line(&mut self, i: usize, chars: &[char]) {
        if let Some(slot) = self.lines.get_mut(i) {
            *slot = chars.iter().collect();
        }
    }

    /// Clamp the cursor into the buffer. In normal mode the caret sits *on* a
    /// character, so it stops one short of the end; in insert mode it may sit
    /// past the last one.
    pub fn clamp_cursor(&mut self, past_end: bool) {
        self.cursor.line = self.cursor.line.min(self.line_count().saturating_sub(1));
        let len = self.line_len(self.cursor.line);
        let max = if past_end { len } else { len.saturating_sub(1) };
        self.cursor.col = self.cursor.col.min(max);
    }

    // ------------------------------------------------------------------ undo

    /// Record the current state so the next edit can be undone.
    ///
    /// Callers push one snapshot per *user-visible* action, not per keystroke —
    /// which is why entering insert mode pushes once and typing a paragraph
    /// does not.
    pub fn checkpoint(&mut self) {
        self.undo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        // Any new edit invalidates the redo branch, exactly as vim does.
        self.redo.clear();
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.dirty = true;
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        self.lines = prev.lines;
        self.cursor = prev.cursor;
        self.dirty = true;
        self.clamp_cursor(false);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        self.lines = next.lines;
        self.cursor = next.cursor;
        self.dirty = true;
        self.clamp_cursor(false);
        true
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    // --------------------------------------------------------------- editing

    pub fn insert_char(&mut self, c: char) {
        let mut chars = self.chars(self.cursor.line);
        let at = self.cursor.col.min(chars.len());
        chars.insert(at, c);
        self.set_line(self.cursor.line, &chars);
        self.cursor.col = at + 1;
        self.dirty = true;
    }

    pub fn insert_newline(&mut self) {
        let chars = self.chars(self.cursor.line);
        let at = self.cursor.col.min(chars.len());
        let (head, tail) = chars.split_at(at);
        let head: String = head.iter().collect();
        let tail: String = tail.iter().collect();
        self.lines[self.cursor.line] = head;
        self.lines.insert(self.cursor.line + 1, tail);
        self.cursor.line += 1;
        self.cursor.col = 0;
        self.dirty = true;
    }

    /// Insert a blank line below (`below`) or above the cursor and move onto it.
    pub fn open_line(&mut self, below: bool) {
        let at = if below {
            self.cursor.line + 1
        } else {
            self.cursor.line
        };
        self.lines.insert(at, String::new());
        self.cursor.line = at;
        self.cursor.col = 0;
        self.dirty = true;
    }

    pub fn backspace(&mut self) {
        if self.cursor.col > 0 {
            let mut chars = self.chars(self.cursor.line);
            chars.remove(self.cursor.col - 1);
            self.set_line(self.cursor.line, &chars);
            self.cursor.col -= 1;
        } else if self.cursor.line > 0 {
            // Joining onto the previous line leaves the caret at the seam.
            let current = self.lines.remove(self.cursor.line);
            self.cursor.line -= 1;
            self.cursor.col = self.line_len(self.cursor.line);
            self.lines[self.cursor.line].push_str(&current);
        }
        self.dirty = true;
    }

    /// Delete `count` characters at the cursor, returning what was removed.
    pub fn delete_chars(&mut self, count: usize) -> String {
        let mut chars = self.chars(self.cursor.line);
        let from = self.cursor.col.min(chars.len());
        let to = (from + count).min(chars.len());
        let removed: String = chars[from..to].iter().collect();
        chars.drain(from..to);
        self.set_line(self.cursor.line, &chars);
        self.dirty = true;
        removed
    }

    /// Delete a character range within one line, returning what was removed.
    pub fn delete_range_in_line(&mut self, line: usize, from: usize, to: usize) -> String {
        let mut chars = self.chars(line);
        let from = from.min(chars.len());
        let to = to.min(chars.len());
        if from >= to {
            return String::new();
        }
        let removed: String = chars[from..to].iter().collect();
        chars.drain(from..to);
        self.set_line(line, &chars);
        self.dirty = true;
        removed
    }

    /// Delete whole lines `from..=to`, returning them.
    ///
    /// A buffer always has at least one line, so deleting everything leaves a
    /// single empty one rather than an empty `Vec` that every caller would then
    /// have to guard against.
    pub fn delete_lines(&mut self, from: usize, to: usize) -> Vec<String> {
        let from = from.min(self.lines.len().saturating_sub(1));
        let to = to.min(self.lines.len().saturating_sub(1));
        let removed: Vec<String> = self.lines.drain(from..=to).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor.line = from.min(self.lines.len() - 1);
        self.cursor.col = 0;
        self.dirty = true;
        removed
    }

    pub fn insert_lines(&mut self, at: usize, lines: &[String]) {
        let at = at.min(self.lines.len());
        for (i, l) in lines.iter().enumerate() {
            self.lines.insert(at + i, l.clone());
        }
        self.dirty = true;
    }

    /// Insert text into the middle of a line, for a charwise put.
    pub fn insert_str_at(&mut self, line: usize, col: usize, text: &str) {
        let mut chars = self.chars(line);
        let at = col.min(chars.len());
        for (i, c) in text.chars().enumerate() {
            chars.insert(at + i, c);
        }
        self.set_line(line, &chars);
        self.dirty = true;
    }

    /// Extract the text between two positions, `from` before `to`.
    pub fn text_between(&self, from: Cursor, to: Cursor) -> String {
        if from.line == to.line {
            let chars = self.chars(from.line);
            let a = from.col.min(chars.len());
            let b = to.col.min(chars.len());
            return chars[a.min(b)..a.max(b)].iter().collect();
        }
        let mut out = String::new();
        let head = self.chars(from.line);
        out.extend(head.iter().skip(from.col.min(head.len())));
        for l in from.line + 1..to.line {
            out.push('\n');
            out.push_str(self.line(l));
        }
        out.push('\n');
        let tail = self.chars(to.line);
        out.extend(tail.iter().take(to.col.min(tail.len())));
        out
    }

    /// Delete the text between two positions, `from` before `to`.
    pub fn delete_between(&mut self, from: Cursor, to: Cursor) {
        if from.line == to.line {
            self.delete_range_in_line(from.line, from.col.min(to.col), from.col.max(to.col));
            self.cursor = Cursor::new(from.line, from.col.min(to.col));
            return;
        }
        let head: String = self.chars(from.line).into_iter().take(from.col).collect();
        let tail: String = self.chars(to.line).into_iter().skip(to.col).collect();
        self.lines
            .drain(from.line..=to.line.min(self.lines.len() - 1));
        self.lines.insert(from.line, format!("{head}{tail}"));
        self.cursor = from;
        self.dirty = true;
    }
}
