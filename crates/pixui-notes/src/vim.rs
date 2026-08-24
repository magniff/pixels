//! A modal (vim-style) editing engine over [`Buffer`].
//!
//! Normal mode is parsed rather than switch-cased, because vim's grammar really
//! is a grammar: `[count] operator [count] motion`. Accumulating keystrokes in
//! `pending` and re-parsing the whole thing each time is what lets `3dw`,
//! `d3w`, `dd`, `2dd` and `dG` all fall out of one code path instead of five.
//!
//! A parse can come back *incomplete* — `d` on its own is not an error, it is a
//! prefix — which is why `pending` is only cleared on success or on a genuinely
//! invalid sequence.

use pixui::{Key, Mods};

use crate::text::{Buffer, Cursor};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    /// Charwise visual. Linewise (`V`) and block (`Ctrl-v`) are not implemented.
    Visual,
    /// Typing a `:` command.
    Command,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
            Mode::Command => "COMMAND",
        }
    }
}

/// The unnamed register. Vim distinguishes charwise from linewise yanks, and so
/// must this: it decides whether `p` pastes inline or opens a new line.
#[derive(Clone, Debug)]
pub enum Register {
    Chars(String),
    Lines(Vec<String>),
}

/// Something the editor cannot do by itself, handed back to the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VimEvent {
    /// A `:` command line was submitted, without the leading colon.
    Command(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Motion {
    Left,
    Right,
    Down,
    Up,
    WordFwd,
    WordBack,
    WordEnd,
    LineStart,
    FirstNonBlank,
    LineEnd,
    FileStart,
    FileEnd,
}

impl Motion {
    /// Whether an operator applied to this motion works on whole lines.
    fn linewise(self) -> bool {
        matches!(
            self,
            Motion::Down | Motion::Up | Motion::FileStart | Motion::FileEnd
        )
    }
}

/// What an operator will act on.
enum Range {
    Chars { from: Cursor, to: Cursor },
    Lines { from: usize, to: usize },
}

enum Parse {
    Done,
    /// A valid prefix; wait for more keys.
    Incomplete,
    Invalid,
}

#[derive(Default)]
pub struct Vim {
    pub mode: Mode,
    /// Keys typed so far in an unfinished normal-mode command.
    pub pending: String,
    pub cmdline: String,
    pub register: Option<Register>,
    /// Where visual mode started.
    pub anchor: Cursor,
    /// Transient message for the status line.
    pub status: String,
}

impl Vim {
    pub fn new() -> Self {
        Self::default()
    }

    /// The inclusive selection in visual mode, normalised so `.0 <= .1`.
    pub fn selection(&self, buf: &Buffer) -> Option<(Cursor, Cursor)> {
        if self.mode != Mode::Visual {
            return None;
        }
        let (a, b) = (self.anchor, buf.cursor);
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// Feed one key press. Returns an event when the app has work to do.
    pub fn handle(&mut self, buf: &mut Buffer, key: Key, mods: Mods) -> Option<VimEvent> {
        match self.mode {
            Mode::Insert => {
                self.handle_insert(buf, key, mods);
                None
            }
            Mode::Command => self.handle_command(key),
            Mode::Normal | Mode::Visual => self.handle_normal(buf, key, mods),
        }
    }

    // ---------------------------------------------------------------- insert

    fn handle_insert(&mut self, buf: &mut Buffer, key: Key, mods: Mods) {
        match key {
            Key::Escape => {
                self.mode = Mode::Normal;
                // vim steps left on leaving insert, so the caret lands on the
                // character you just typed rather than past it.
                buf.cursor.col = buf.cursor.col.saturating_sub(1);
                buf.clamp_cursor(false);
            }
            Key::Enter => buf.insert_newline(),
            Key::Backspace => buf.backspace(),
            Key::Space => buf.insert_char(' '),
            Key::Tab => {
                for _ in 0..2 {
                    buf.insert_char(' ');
                }
            }
            Key::Char(c) if !mods.cmd && !mods.ctrl => buf.insert_char(c),
            Key::Left => buf.cursor.col = buf.cursor.col.saturating_sub(1),
            Key::Right => buf.cursor.col = (buf.cursor.col + 1).min(buf.line_len(buf.cursor.line)),
            Key::Up | Key::Down => {
                let d = if key == Key::Up { -1i32 } else { 1 };
                let next = buf.cursor.line as i32 + d;
                if next >= 0 && (next as usize) < buf.line_count() {
                    buf.cursor.line = next as usize;
                    buf.clamp_cursor(true);
                }
            }
            _ => {}
        }
    }

    // --------------------------------------------------------------- command

    fn handle_command(&mut self, key: Key) -> Option<VimEvent> {
        match key {
            Key::Escape => {
                self.mode = Mode::Normal;
                self.cmdline.clear();
            }
            Key::Enter => {
                let cmd = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Normal;
                return Some(VimEvent::Command(cmd));
            }
            Key::Backspace => {
                if self.cmdline.pop().is_none() {
                    // Backspacing past the colon leaves command mode, which is
                    // what vim does and what the fingers expect.
                    self.mode = Mode::Normal;
                }
            }
            Key::Space => self.cmdline.push(' '),
            Key::Char(c) => self.cmdline.push(c),
            _ => {}
        }
        None
    }

    // ---------------------------------------------------------------- normal

    fn handle_normal(&mut self, buf: &mut Buffer, key: Key, mods: Mods) -> Option<VimEvent> {
        // Control combinations are not part of the pending grammar.
        if mods.ctrl {
            match key {
                Key::Char('r') => {
                    self.status = if buf.redo() {
                        "redo".into()
                    } else {
                        "nothing to redo".into()
                    };
                }
                Key::Char('d') | Key::Char('u') => {
                    let half = 12i32;
                    let d = if key == Key::Char('d') { half } else { -half };
                    let next = (buf.cursor.line as i32 + d).clamp(0, buf.line_count() as i32 - 1);
                    buf.cursor.line = next as usize;
                    buf.clamp_cursor(false);
                }
                _ => {}
            }
            self.pending.clear();
            return None;
        }

        match key {
            Key::Escape => {
                self.pending.clear();
                if self.mode == Mode::Visual {
                    self.mode = Mode::Normal;
                }
                return None;
            }
            Key::Space => {
                self.pending.clear();
                return None;
            }
            Key::Left => return self.simple_motion(buf, Motion::Left),
            Key::Right => return self.simple_motion(buf, Motion::Right),
            Key::Up => return self.simple_motion(buf, Motion::Up),
            Key::Down => return self.simple_motion(buf, Motion::Down),
            Key::Char(c) => self.pending.push(c),
            _ => return None,
        }

        match self.parse_pending(buf) {
            Parse::Incomplete => {}
            Parse::Done | Parse::Invalid => self.pending.clear(),
        }
        None
    }

    fn simple_motion(&mut self, buf: &mut Buffer, m: Motion) -> Option<VimEvent> {
        self.apply_motion(buf, m, 1);
        None
    }

    /// Try to interpret everything typed so far.
    fn parse_pending(&mut self, buf: &mut Buffer) -> Parse {
        let pending = self.pending.clone();
        let chars: Vec<char> = pending.chars().collect();
        let mut i = 0;

        let count1 = take_count(&chars, &mut i);
        if i >= chars.len() {
            return Parse::Incomplete;
        }

        let c = chars[i];

        // ---- operators ---------------------------------------------------
        if matches!(c, 'd' | 'c' | 'y') && self.mode == Mode::Normal {
            let op = c;
            i += 1;
            let count2 = take_count(&chars, &mut i);
            if i >= chars.len() {
                return Parse::Incomplete;
            }
            let count = count1.unwrap_or(1) * count2.unwrap_or(1);

            // `dd` / `cc` / `yy` act on whole lines.
            if chars[i] == op {
                let from = buf.cursor.line;
                let to = (from + count - 1).min(buf.line_count() - 1);
                self.apply_operator(buf, op, Range::Lines { from, to });
                return Parse::Done;
            }

            // Text objects: `diw`, `ci"`, `da(` and friends. These are checked
            // before motions because `i` and `a` are also normal-mode commands,
            // and after an operator they can only mean an object.
            if matches!(chars[i], 'i' | 'a') {
                let Some(&object) = chars.get(i + 1) else {
                    return Parse::Incomplete;
                };
                return match text_object(buf, object, chars[i] == 'i') {
                    Some(range) => {
                        self.apply_operator(buf, op, range);
                        Parse::Done
                    }
                    None => {
                        self.status = "no such text object here".into();
                        Parse::Done
                    }
                };
            }

            return match parse_motion(&chars, i) {
                Some((motion, _)) => {
                    let range = self.motion_range(buf, motion, count);
                    self.apply_operator(buf, op, range);
                    Parse::Done
                }
                None if is_motion_prefix(&chars, i) => Parse::Incomplete,
                None => Parse::Invalid,
            };
        }

        // ---- motions -----------------------------------------------------
        if let Some((motion, _)) = parse_motion(&chars, i) {
            self.apply_motion(buf, motion, count1.unwrap_or(1));
            return Parse::Done;
        }
        if is_motion_prefix(&chars, i) {
            return Parse::Incomplete;
        }

        // ---- visual-mode text objects ------------------------------------
        if self.mode == Mode::Visual && matches!(c, 'i' | 'a') {
            let Some(&object) = chars.get(i + 1) else {
                return Parse::Incomplete;
            };
            match text_object(buf, object, c == 'i') {
                Some(Range::Chars { from, to }) => {
                    self.anchor = from;
                    buf.cursor = Cursor::new(to.line, to.col.saturating_sub(1));
                    buf.clamp_cursor(false);
                }
                Some(Range::Lines { from, to }) => {
                    self.anchor = Cursor::new(from, 0);
                    buf.cursor = Cursor::new(to, buf.line_len(to).saturating_sub(1));
                    buf.clamp_cursor(false);
                }
                None => self.status = "no such text object here".into(),
            }
            return Parse::Done;
        }

        // ---- visual-mode operators ---------------------------------------
        if self.mode == Mode::Visual {
            if let Some((from, to)) = self.selection(buf) {
                match c {
                    'd' | 'x' => {
                        buf.checkpoint();
                        let mut end = to;
                        end.col += 1; // the selection is inclusive
                        self.register = Some(Register::Chars(buf.text_between(from, end)));
                        buf.delete_between(from, end);
                        self.mode = Mode::Normal;
                        buf.clamp_cursor(false);
                        return Parse::Done;
                    }
                    'y' => {
                        let mut end = to;
                        end.col += 1;
                        self.register = Some(Register::Chars(buf.text_between(from, end)));
                        buf.cursor = from;
                        self.mode = Mode::Normal;
                        return Parse::Done;
                    }
                    'c' => {
                        buf.checkpoint();
                        let mut end = to;
                        end.col += 1;
                        self.register = Some(Register::Chars(buf.text_between(from, end)));
                        buf.delete_between(from, end);
                        self.mode = Mode::Insert;
                        return Parse::Done;
                    }
                    'v' => {
                        self.mode = Mode::Normal;
                        return Parse::Done;
                    }
                    _ => {}
                }
            }
        }

        // ---- single-key commands -----------------------------------------
        let count = count1.unwrap_or(1);
        match c {
            'i' => {
                buf.checkpoint();
                self.mode = Mode::Insert;
            }
            'a' => {
                buf.checkpoint();
                buf.cursor.col = (buf.cursor.col + 1).min(buf.line_len(buf.cursor.line));
                self.mode = Mode::Insert;
            }
            'I' => {
                buf.checkpoint();
                buf.cursor.col = first_non_blank(buf.line(buf.cursor.line));
                self.mode = Mode::Insert;
            }
            'A' => {
                buf.checkpoint();
                buf.cursor.col = buf.line_len(buf.cursor.line);
                self.mode = Mode::Insert;
            }
            'o' => {
                buf.checkpoint();
                buf.open_line(true);
                self.mode = Mode::Insert;
            }
            'O' => {
                buf.checkpoint();
                buf.open_line(false);
                self.mode = Mode::Insert;
            }
            'v' => {
                self.anchor = buf.cursor;
                self.mode = Mode::Visual;
            }
            'x' => {
                buf.checkpoint();
                let removed = buf.delete_chars(count);
                self.register = Some(Register::Chars(removed));
                buf.clamp_cursor(false);
            }
            'D' => {
                buf.checkpoint();
                let line = buf.cursor.line;
                let end = buf.line_len(line);
                let removed = buf.delete_range_in_line(line, buf.cursor.col, end);
                self.register = Some(Register::Chars(removed));
                buf.clamp_cursor(false);
            }
            'C' => {
                buf.checkpoint();
                let line = buf.cursor.line;
                let end = buf.line_len(line);
                buf.delete_range_in_line(line, buf.cursor.col, end);
                self.mode = Mode::Insert;
            }
            'p' | 'P' => self.put(buf, c == 'p'),
            'u' => {
                self.status = if buf.undo() {
                    "undo".into()
                } else {
                    "already at oldest".into()
                };
            }
            ':' => {
                self.mode = Mode::Command;
                self.cmdline.clear();
            }
            _ => return Parse::Invalid,
        }
        Parse::Done
    }

    fn put(&mut self, buf: &mut Buffer, after: bool) {
        let Some(reg) = self.register.clone() else {
            self.status = "register empty".into();
            return;
        };
        buf.checkpoint();
        match reg {
            Register::Lines(lines) => {
                let at = if after {
                    buf.cursor.line + 1
                } else {
                    buf.cursor.line
                };
                buf.insert_lines(at, &lines);
                buf.cursor.line = at;
                buf.cursor.col = 0;
            }
            Register::Chars(text) => {
                let col = if after {
                    (buf.cursor.col + 1).min(buf.line_len(buf.cursor.line))
                } else {
                    buf.cursor.col
                };
                buf.insert_str_at(buf.cursor.line, col, &text);
                buf.cursor.col = col + text.chars().count().saturating_sub(1);
            }
        }
        buf.clamp_cursor(false);
    }

    // --------------------------------------------------------------- motions

    fn apply_motion(&mut self, buf: &mut Buffer, m: Motion, count: usize) {
        let past_end = self.mode == Mode::Insert;
        for _ in 0..count {
            match m {
                Motion::Left => buf.cursor.col = buf.cursor.col.saturating_sub(1),
                Motion::Right => {
                    let max = buf.line_len(buf.cursor.line).saturating_sub(1);
                    buf.cursor.col = (buf.cursor.col + 1).min(max);
                }
                Motion::Down | Motion::Up => {
                    // Remember the column so travelling through a short line
                    // and out the far side returns to where you started.
                    let want = buf.desired_col.max(buf.cursor.col);
                    let d: i32 = if m == Motion::Down { 1 } else { -1 };
                    let next = buf.cursor.line as i32 + d;
                    if next >= 0 && (next as usize) < buf.line_count() {
                        buf.cursor.line = next as usize;
                        buf.cursor.col = want;
                        buf.desired_col = want;
                        buf.clamp_cursor(past_end);
                        continue;
                    }
                }
                Motion::WordFwd => buf.cursor = word_forward(buf, buf.cursor),
                Motion::WordBack => buf.cursor = word_back(buf, buf.cursor),
                Motion::WordEnd => buf.cursor = word_end(buf, buf.cursor),
                Motion::LineStart => buf.cursor.col = 0,
                Motion::FirstNonBlank => {
                    buf.cursor.col = first_non_blank(buf.line(buf.cursor.line))
                }
                Motion::LineEnd => buf.cursor.col = buf.line_len(buf.cursor.line).saturating_sub(1),
                Motion::FileStart => buf.cursor = Cursor::new(0, 0),
                Motion::FileEnd => buf.cursor = Cursor::new(buf.line_count() - 1, 0),
            }
            if !matches!(m, Motion::Down | Motion::Up) {
                buf.desired_col = buf.cursor.col;
            }
        }
        buf.clamp_cursor(past_end);
    }

    /// Where an operator applied to this motion should reach.
    fn motion_range(&self, buf: &Buffer, m: Motion, count: usize) -> Range {
        let start = buf.cursor;
        if m.linewise() {
            let to = match m {
                Motion::Down => (start.line + count).min(buf.line_count() - 1),
                Motion::Up => start.line.saturating_sub(count),
                Motion::FileStart => 0,
                _ => buf.line_count() - 1,
            };
            return Range::Lines {
                from: start.line.min(to),
                to: start.line.max(to),
            };
        }

        let mut probe = Buffer::from_text(&buf.to_text());
        probe.cursor = start;
        let mut vim = Vim::new();
        vim.apply_motion(&mut probe, m, count);
        let end = probe.cursor;

        // `e` and `$` include the character they land on; the rest do not.
        // Without this, `d$` leaves the last character of the line behind.
        let end = if matches!(m, Motion::WordEnd | Motion::LineEnd) {
            Cursor::new(end.line, end.col + 1)
        } else {
            end
        };
        if end < start {
            Range::Chars {
                from: end,
                to: start,
            }
        } else {
            Range::Chars {
                from: start,
                to: end,
            }
        }
    }

    fn apply_operator(&mut self, buf: &mut Buffer, op: char, range: Range) {
        buf.checkpoint();
        match range {
            Range::Lines { from, to } => {
                let removed = if op == 'y' {
                    buf.lines()[from..=to.min(buf.line_count() - 1)].to_vec()
                } else {
                    buf.delete_lines(from, to)
                };
                self.register = Some(Register::Lines(removed));
                if op == 'c' {
                    // `cc` keeps a line to type on rather than removing it.
                    buf.open_line(false);
                    self.mode = Mode::Insert;
                } else if op == 'y' {
                    buf.cursor.line = from;
                }
            }
            Range::Chars { from, to } => {
                let text = buf.text_between(from, to);
                self.register = Some(Register::Chars(text));
                if op != 'y' {
                    buf.delete_between(from, to);
                } else {
                    buf.cursor = from;
                }
                if op == 'c' {
                    self.mode = Mode::Insert;
                }
            }
        }
        buf.clamp_cursor(self.mode == Mode::Insert);
    }
}

// ------------------------------------------------------------------- parsing

/// Consume leading digits as a count. A leading `0` is the line-start motion,
/// not a count, so it is deliberately not taken here.
fn take_count(chars: &[char], i: &mut usize) -> Option<usize> {
    let start = *i;
    while *i < chars.len() && chars[*i].is_ascii_digit() && !(chars[*i] == '0' && *i == start) {
        *i += 1;
    }
    if *i == start {
        return None;
    }
    chars[start..*i].iter().collect::<String>().parse().ok()
}

fn parse_motion(chars: &[char], i: usize) -> Option<(Motion, usize)> {
    let c = *chars.get(i)?;
    let m = match c {
        'h' => Motion::Left,
        'l' => Motion::Right,
        'j' => Motion::Down,
        'k' => Motion::Up,
        'w' => Motion::WordFwd,
        'b' => Motion::WordBack,
        'e' => Motion::WordEnd,
        '0' => Motion::LineStart,
        '^' => Motion::FirstNonBlank,
        '$' => Motion::LineEnd,
        'G' => Motion::FileEnd,
        'g' => {
            return match chars.get(i + 1) {
                Some('g') => Some((Motion::FileStart, 2)),
                _ => None,
            }
        }
        _ => return None,
    };
    Some((m, 1))
}

/// Whether the text at `i` could still become a motion with more keys.
fn is_motion_prefix(chars: &[char], i: usize) -> bool {
    matches!(chars.get(i), Some('g')) && chars.get(i + 1).is_none()
}

/// Resolve a text object around the cursor.
///
/// `inner` selects `iw`-style (the thing itself); otherwise `aw`-style (the
/// thing plus its surrounding whitespace, or its delimiters).
///
/// Bracket objects search across lines with depth counting, which is what makes
/// `di(` work on a call split over several lines. Quote objects are scoped to
/// the current line, as they are in vim — a quote is far more often unbalanced
/// across lines than a bracket.
fn text_object(buf: &Buffer, object: char, inner: bool) -> Option<Range> {
    match object {
        'w' => word_object(buf, inner),
        'p' => paragraph_object(buf, inner),
        '"' | '\'' | '`' => quote_object(buf, object, inner),
        '(' | ')' | 'b' => bracket_object(buf, '(', ')', inner),
        '[' | ']' => bracket_object(buf, '[', ']', inner),
        '{' | '}' | 'B' => bracket_object(buf, '{', '}', inner),
        '<' | '>' => bracket_object(buf, '<', '>', inner),
        _ => None,
    }
}

fn word_object(buf: &Buffer, inner: bool) -> Option<Range> {
    let line = buf.cursor.line;
    let chars: Vec<char> = buf.line(line).chars().collect();
    if chars.is_empty() {
        return None;
    }
    let col = buf.cursor.col.min(chars.len() - 1);

    // The run of same-class characters under the cursor. Whitespace counts as
    // a class of its own, so `diw` on a gap deletes the gap.
    let target = class(chars[col]);
    let mut start = col;
    while start > 0 && class(chars[start - 1]) == target {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < chars.len() && class(chars[end + 1]) == target {
        end += 1;
    }
    let (mut from, mut to) = (start, end + 1);

    if !inner {
        // `aw` takes the trailing whitespace, or the leading whitespace when
        // there is none after — which is what makes it work on the last word.
        let had_trailing = to < chars.len() && class(chars[to]) == Class::Blank;
        while to < chars.len() && class(chars[to]) == Class::Blank {
            to += 1;
        }
        if !had_trailing {
            while from > 0 && class(chars[from - 1]) == Class::Blank {
                from -= 1;
            }
        }
    }

    Some(Range::Chars {
        from: Cursor::new(line, from),
        to: Cursor::new(line, to),
    })
}

/// A paragraph is a run of blank or non-blank lines; `ap` also takes the blank
/// lines that follow it.
fn paragraph_object(buf: &Buffer, inner: bool) -> Option<Range> {
    let blank = |l: usize| buf.line(l).trim().is_empty();
    let here = buf.cursor.line;
    let target = blank(here);

    let mut from = here;
    while from > 0 && blank(from - 1) == target {
        from -= 1;
    }
    let mut to = here;
    while to + 1 < buf.line_count() && blank(to + 1) == target {
        to += 1;
    }
    if !inner && !target {
        while to + 1 < buf.line_count() && blank(to + 1) {
            to += 1;
        }
    }
    Some(Range::Lines { from, to })
}

fn quote_object(buf: &Buffer, quote: char, inner: bool) -> Option<Range> {
    let line = buf.cursor.line;
    let chars: Vec<char> = buf.line(line).chars().collect();
    let marks: Vec<usize> = (0..chars.len()).filter(|&i| chars[i] == quote).collect();

    // Quotes pair off left to right. Take the pair containing the cursor, or
    // else the next one along, which is what vim does when you are before it.
    let pair = marks
        .chunks(2)
        .filter(|c| c.len() == 2)
        .find(|c| buf.cursor.col <= c[1])
        .map(|c| (c[0], c[1]))?;

    let (from, to) = if inner {
        (pair.0 + 1, pair.1)
    } else {
        (pair.0, pair.1 + 1)
    };
    Some(Range::Chars {
        from: Cursor::new(line, from),
        to: Cursor::new(line, to),
    })
}

fn bracket_object(buf: &Buffer, open: char, close: char, inner: bool) -> Option<Range> {
    let start = buf.cursor;
    let open_at = scan_for(buf, start, open, close, false)?;
    let close_at = scan_for(buf, start, close, open, true)?;

    let (from, to) = if inner {
        (Cursor::new(open_at.line, open_at.col + 1), close_at)
    } else {
        (open_at, Cursor::new(close_at.line, close_at.col + 1))
    };
    if to < from {
        return None;
    }
    Some(Range::Chars { from, to })
}

/// Walk out from `start` looking for `want`, counting `nest` as depth.
///
/// `forward` picks the direction. The character under the cursor counts as a
/// match but never as nesting, so sitting on a bracket selects its own pair.
fn scan_for(buf: &Buffer, start: Cursor, want: char, nest: char, forward: bool) -> Option<Cursor> {
    let mut cur = start;
    let mut depth = 0i32;
    loop {
        let ch = char_at(buf, cur);
        if ch == want {
            if depth == 0 {
                return Some(cur);
            }
            depth -= 1;
        } else if ch == nest && cur != start {
            depth += 1;
        }
        cur = if forward {
            step_forward(buf, cur)?
        } else {
            step_back(buf, cur)?
        };
    }
}

fn first_non_blank(line: &str) -> usize {
    line.chars().position(|c| !c.is_whitespace()).unwrap_or(0)
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Blank,
    Word,
    Punct,
}

fn class(c: char) -> Class {
    if c.is_whitespace() {
        Class::Blank
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// Flatten the buffer position into a (line, col) walk that can cross lines.
fn step_forward(buf: &Buffer, c: Cursor) -> Option<Cursor> {
    if c.col + 1 < buf.line_len(c.line) {
        Some(Cursor::new(c.line, c.col + 1))
    } else if c.line + 1 < buf.line_count() {
        Some(Cursor::new(c.line + 1, 0))
    } else {
        None
    }
}

fn step_back(buf: &Buffer, c: Cursor) -> Option<Cursor> {
    if c.col > 0 {
        Some(Cursor::new(c.line, c.col - 1))
    } else if c.line > 0 {
        Some(Cursor::new(
            c.line - 1,
            buf.line_len(c.line - 1).saturating_sub(1),
        ))
    } else {
        None
    }
}

fn char_at(buf: &Buffer, c: Cursor) -> char {
    buf.line(c.line).chars().nth(c.col).unwrap_or(' ')
}

fn word_forward(buf: &Buffer, from: Cursor) -> Cursor {
    let mut cur = from;
    let start_class = class(char_at(buf, cur));
    // Leave the current run...
    while class(char_at(buf, cur)) == start_class && start_class != Class::Blank {
        match step_forward(buf, cur) {
            Some(next) => cur = next,
            None => return cur,
        }
    }
    // ...then skip any whitespace before the next one.
    while class(char_at(buf, cur)) == Class::Blank {
        match step_forward(buf, cur) {
            Some(next) => cur = next,
            None => return cur,
        }
    }
    cur
}

fn word_back(buf: &Buffer, from: Cursor) -> Cursor {
    let mut cur = match step_back(buf, from) {
        Some(c) => c,
        None => return from,
    };
    while class(char_at(buf, cur)) == Class::Blank {
        match step_back(buf, cur) {
            Some(next) => cur = next,
            None => return cur,
        }
    }
    let target = class(char_at(buf, cur));
    while let Some(prev) = step_back(buf, cur) {
        if class(char_at(buf, prev)) != target {
            break;
        }
        cur = prev;
    }
    cur
}

fn word_end(buf: &Buffer, from: Cursor) -> Cursor {
    let mut cur = match step_forward(buf, from) {
        Some(c) => c,
        None => return from,
    };
    while class(char_at(buf, cur)) == Class::Blank {
        match step_forward(buf, cur) {
            Some(next) => cur = next,
            None => return cur,
        }
    }
    let target = class(char_at(buf, cur));
    while let Some(next) = step_forward(buf, cur) {
        if class(char_at(buf, next)) != target {
            break;
        }
        cur = next;
    }
    cur
}
