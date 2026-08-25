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

use crate::indent;
use crate::text::{Buffer, Cursor};

/// Which shape a visual selection takes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VisualKind {
    /// `v`: a run of characters, which may wrap across lines.
    Char,
    /// `V`: whole lines.
    Line,
    /// `Ctrl-v`: a rectangle of columns.
    Block,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual(VisualKind),
    /// Typing a `:` command.
    Command,
    /// Typing a `/` or `?` search pattern.
    Search {
        backward: bool,
    },
}

/// What is currently selected, in whichever shape the visual mode is in.
///
/// Ranges are inclusive at both ends, which is how vim thinks about a
/// selection: the character under the cursor is part of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    Chars {
        from: Cursor,
        to: Cursor,
    },
    Lines {
        from: usize,
        to: usize,
    },
    Block {
        top: usize,
        bottom: usize,
        left: usize,
        right: usize,
    },
}

impl Selection {
    /// The half-open column range covered on `line`, if any.
    ///
    /// A block reports its columns whether or not the line reaches them, so the
    /// rectangle stays a rectangle on screen even over short lines — which is
    /// the whole point of seeing it.
    pub fn columns_on(&self, line: usize, line_len: usize) -> Option<(usize, usize)> {
        match *self {
            Selection::Chars { from, to } => {
                if line < from.line || line > to.line {
                    return None;
                }
                let lo = if line == from.line { from.col } else { 0 };
                let hi = if line == to.line {
                    to.col + 1
                } else {
                    line_len.max(1)
                };
                Some((lo, hi))
            }
            Selection::Lines { from, to } => {
                (line >= from && line <= to).then(|| (0, line_len.max(1)))
            }
            Selection::Block {
                top,
                bottom,
                left,
                right,
            } => (line >= top && line <= bottom).then(|| (left, right + 1)),
        }
    }

    /// The lines the selection touches, inclusive.
    pub fn line_span(&self) -> (usize, usize) {
        match *self {
            Selection::Chars { from, to } => (from.line, to.line),
            Selection::Lines { from, to } => (from, to),
            Selection::Block { top, bottom, .. } => (top, bottom),
        }
    }
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual(VisualKind::Char) => "VISUAL",
            Mode::Visual(VisualKind::Line) => "V-LINE",
            Mode::Visual(VisualKind::Block) => "V-BLOCK",
            Mode::Command => "COMMAND",
            Mode::Search { .. } => "SEARCH",
        }
    }
}

/// The unnamed register. Vim distinguishes charwise from linewise yanks, and so
/// must this: it decides whether `p` pastes inline or opens a new line.
#[derive(Clone, Debug)]
pub enum Register {
    Chars(String),
    Lines(Vec<String>),
    /// A rectangle, one string per row. Pasting it re-forms the rectangle at
    /// the cursor rather than laying the rows out end to end.
    Block(Vec<String>),
}

/// Something the editor cannot do by itself, handed back to the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VimEvent {
    /// A `:` command line was submitted, without the leading colon.
    Command(String),
}

/// A pending `f`/`F`/`t`/`T`: which character, which way, and whether to stop
/// short of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FindSpec {
    pub backward: bool,
    /// `t`/`T` stop one character before the target rather than on it.
    pub till: bool,
    pub target: char,
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
    /// `f`, `F`, `t`, `T`: to a character on this line.
    Find(FindSpec),
    /// `;` and `,`: the last find again, optionally the other way.
    RepeatFind {
        reverse: bool,
    },
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

/// A blockwise insert in progress.
///
/// `I` and `A` in block mode type on one line and replicate to the rest when
/// insert mode ends — the reason anyone reaches for blockwise in the first
/// place. Nothing can be replicated until the typing is finished, so the intent
/// has to be parked here until then.
#[derive(Clone, Debug)]
struct BlockInsert {
    lines: Vec<usize>,
    col: usize,
    /// The line actually being typed on.
    anchor_line: usize,
    /// Whether a line too short to reach `col` is padded to it (`A`) or left
    /// alone (`I`).
    pad: bool,
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
    block_insert: Option<BlockInsert>,
    /// The last `/` pattern and the direction it was entered in, for `n`/`N`.
    last_search: Option<(String, bool)>,
    /// The pattern currently highlighted. Cleared by Escape, so the highlight
    /// does not outstay its welcome the way vim's `hlsearch` famously does.
    search_hl: Option<String>,
    /// The last `f`/`t`, for `;` and `,`.
    last_find: Option<FindSpec>,
}

impl Vim {
    pub fn new() -> Self {
        Self::default()
    }

    /// The pattern currently highlighted, if any.
    pub fn search_pattern(&self) -> Option<&str> {
        self.search_hl.as_deref()
    }

    /// The character a prompt is being typed after, if one is open.
    pub fn prompt_prefix(&self) -> Option<char> {
        match self.mode {
            Mode::Command => Some(':'),
            Mode::Search { backward: false } => Some('/'),
            Mode::Search { backward: true } => Some('?'),
            _ => None,
        }
    }

    /// Which visual mode is active, if any.
    pub fn visual_kind(&self) -> Option<VisualKind> {
        match self.mode {
            Mode::Visual(kind) => Some(kind),
            _ => None,
        }
    }

    /// The current selection, normalised so both ends are in order.
    pub fn selection(&self, buf: &Buffer) -> Option<Selection> {
        let kind = self.visual_kind()?;
        let (a, b) = (self.anchor, buf.cursor);
        Some(match kind {
            VisualKind::Char => {
                let (from, to) = if a <= b { (a, b) } else { (b, a) };
                Selection::Chars { from, to }
            }
            VisualKind::Line => Selection::Lines {
                from: a.line.min(b.line),
                to: a.line.max(b.line),
            },
            VisualKind::Block => Selection::Block {
                top: a.line.min(b.line),
                bottom: a.line.max(b.line),
                left: a.col.min(b.col),
                right: a.col.max(b.col),
            },
        })
    }

    /// Place the caret from outside the key handling — a mouse click.
    ///
    /// The mode machine owns its own transitions, so a view that wants to move
    /// the caret asks rather than reaching in and setting fields. Clicking
    /// leaves visual mode, the way clicking anywhere else does.
    pub fn click_at(&mut self, buf: &mut Buffer, at: Cursor) {
        if matches!(self.mode, Mode::Command | Mode::Search { .. }) {
            return;
        }
        if self.visual_kind().is_some() {
            self.mode = Mode::Normal;
        }
        self.pending.clear();
        buf.move_to(at);
        buf.clamp_cursor(self.mode == Mode::Insert);
    }

    /// Extend a selection to `at`, entering charwise visual on the way if the
    /// drag has actually moved.
    ///
    /// A press that never moves is a click, not a selection, so this waits for
    /// the pointer to leave the character it started on before switching modes.
    pub fn drag_to(&mut self, buf: &mut Buffer, anchor: Cursor, at: Cursor) {
        if matches!(
            self.mode,
            Mode::Insert | Mode::Command | Mode::Search { .. }
        ) {
            return;
        }
        if self.visual_kind().is_none() {
            if at == anchor {
                return;
            }
            self.anchor = anchor;
            self.mode = Mode::Visual(VisualKind::Char);
        }
        buf.move_to(at);
        buf.clamp_cursor(false);
    }

    /// Feed one key press. Returns an event when the app has work to do.
    pub fn handle(&mut self, buf: &mut Buffer, key: Key, mods: Mods) -> Option<VimEvent> {
        match self.mode {
            Mode::Insert => {
                self.handle_insert(buf, key, mods);
                None
            }
            Mode::Command => self.handle_command(key),
            Mode::Search { backward } => {
                self.handle_search(buf, key, backward);
                None
            }
            Mode::Normal | Mode::Visual(_) => self.handle_normal(buf, key, mods),
        }
    }

    // ---------------------------------------------------------------- insert

    fn handle_insert(&mut self, buf: &mut Buffer, key: Key, mods: Mods) {
        match key {
            Key::Escape => {
                self.finish_block_insert(buf);
                self.mode = Mode::Normal;
                // vim steps left on leaving insert, so the caret lands on the
                // character you just typed rather than past it.
                buf.cursor.col = buf.cursor.col.saturating_sub(1);
                buf.clamp_cursor(false);
            }
            // A new line inherits where it is: the list it is in, the quote
            // around that, or the indent of the code it sits inside.
            Key::Enter => match indent::opened(buf.lines(), buf.cursor.line, buf.cursor.col) {
                indent::Opened::Plain => buf.insert_newline(),
                indent::Opened::With(prefix) => {
                    buf.insert_newline();
                    buf.insert_str_at(buf.cursor.line, 0, &prefix);
                    buf.cursor.col = prefix.chars().count();
                }
                // The item was empty, so the list is over. The line is emptied
                // rather than a new one opened: one Enter ends the list, and a
                // second one is an ordinary blank line.
                indent::Opened::Ending => {
                    let line = buf.cursor.line;
                    buf.delete_range_in_line(line, 0, buf.line_len(line));
                    buf.cursor.col = 0;
                }
            },
            Key::Backspace => buf.backspace(),
            Key::Space => buf.insert_char(' '),
            // Tab moves the whole line rather than inserting at the caret: in a
            // document made of nested lists and indented code, one level in or
            // out is what it is nearly always for.
            Key::Tab => {
                let line = buf.cursor.line;
                let step = indent::step(buf.lines(), line);
                let (text, moved) = indent::shifted(buf.line(line), mods.shift, step);
                buf.delete_range_in_line(line, 0, buf.line_len(line));
                buf.insert_str_at(line, 0, &text);
                buf.cursor.col = (buf.cursor.col as i32 + moved).max(0) as usize;
                buf.clamp_cursor(true);
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

    // ---------------------------------------------------------------- search

    fn handle_search(&mut self, buf: &mut Buffer, key: Key, backward: bool) {
        match key {
            Key::Escape => {
                self.mode = Mode::Normal;
                self.cmdline.clear();
            }
            Key::Enter => {
                let pattern = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Normal;
                if pattern.is_empty() {
                    // A bare Enter repeats the previous pattern, as vim does.
                    self.repeat_search(buf, false);
                } else {
                    self.last_search = Some((pattern.clone(), backward));
                    self.search_hl = Some(pattern);
                    self.jump_to_match(buf, backward);
                }
            }
            Key::Backspace => {
                if self.cmdline.pop().is_none() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Space => self.cmdline.push(' '),
            Key::Char(c) => self.cmdline.push(c),
            _ => {}
        }
    }

    fn jump_to_match(&mut self, buf: &mut Buffer, backward: bool) {
        let Some((pattern, _)) = self.last_search.clone() else {
            self.status = "no previous search".into();
            return;
        };
        match next_match(buf, &pattern, buf.cursor, backward) {
            Some(at) => {
                buf.move_to(at);
                buf.clamp_cursor(false);
                self.status = format!("/{pattern}");
            }
            None => self.status = format!("pattern not found: {pattern}"),
        }
    }

    /// `n` and `N`: the last search again, `N` the other way.
    fn repeat_search(&mut self, buf: &mut Buffer, reverse: bool) {
        let Some((pattern, backward)) = self.last_search.clone() else {
            self.status = "no previous search".into();
            return;
        };
        self.search_hl = Some(pattern);
        self.jump_to_match(buf, backward ^ reverse);
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
                Key::Char('v') => {
                    self.enter_visual(buf, VisualKind::Block);
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
                // Escape clears the search highlight too, so it does not
                // outstay its welcome the way vim's `hlsearch` famously does.
                self.search_hl = None;
                if self.visual_kind().is_some() {
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
        if self.visual_kind().is_some() && matches!(c, 'i' | 'a') {
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

        // ---- visual-mode commands ----------------------------------------
        if let Some(kind) = self.visual_kind() {
            match c {
                // `s` is a synonym for `c` on a selection.
                'd' | 'x' | 'y' | 'c' | 's' => {
                    self.apply_visual_operator(buf, if c == 's' { 'c' } else { c });
                    return Parse::Done;
                }
                // Swap which end of the selection the cursor is on, so the
                // other end can be adjusted without starting over.
                'o' => {
                    std::mem::swap(&mut self.anchor, &mut buf.cursor);
                    buf.clamp_cursor(false);
                    return Parse::Done;
                }
                'I' | 'A' if kind == VisualKind::Block => {
                    self.begin_block_insert(buf, c == 'A');
                    return Parse::Done;
                }
                'v' => {
                    self.switch_visual(VisualKind::Char);
                    return Parse::Done;
                }
                'V' => {
                    self.switch_visual(VisualKind::Line);
                    return Parse::Done;
                }
                _ => {}
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
            'o' | 'O' => {
                buf.checkpoint();
                // The line it inherits from is the one you are standing on,
                // whichever side the new one opens.
                let from = buf.cursor.line;
                let carried = indent::opened(buf.lines(), from, usize::MAX);
                buf.open_line(c == 'o');
                if let indent::Opened::With(prefix) = carried {
                    buf.insert_str_at(buf.cursor.line, 0, &prefix);
                    buf.cursor.col = prefix.chars().count();
                }
                self.mode = Mode::Insert;
            }
            'v' => self.enter_visual(buf, VisualKind::Char),
            'V' => self.enter_visual(buf, VisualKind::Line),
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
            '/' => {
                self.mode = Mode::Search { backward: false };
                self.cmdline.clear();
            }
            '?' => {
                self.mode = Mode::Search { backward: true };
                self.cmdline.clear();
            }
            'n' => self.repeat_search(buf, false),
            'N' => self.repeat_search(buf, true),
            '*' => {
                // Search for the word under the cursor, which is the reason
                // anyone tolerates typing a pattern the rest of the time.
                match word_under_cursor(buf) {
                    Some(word) => {
                        self.last_search = Some((word.clone(), false));
                        self.search_hl = Some(word);
                        self.jump_to_match(buf, false);
                    }
                    None => self.status = "no word under the cursor".into(),
                }
            }
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

    fn enter_visual(&mut self, buf: &Buffer, kind: VisualKind) {
        self.anchor = buf.cursor;
        self.mode = Mode::Visual(kind);
    }

    /// The same key again leaves visual mode; a different one changes shape
    /// without losing the selection.
    fn switch_visual(&mut self, kind: VisualKind) {
        self.mode = if self.visual_kind() == Some(kind) {
            Mode::Normal
        } else {
            Mode::Visual(kind)
        };
    }

    /// Apply `d`, `y` or `c` to the current selection, in its own shape.
    fn apply_visual_operator(&mut self, buf: &mut Buffer, op: char) {
        let Some(sel) = self.selection(buf) else {
            return;
        };
        let yank_only = op == 'y';
        if !yank_only {
            buf.checkpoint();
        }

        match sel {
            Selection::Chars { from, to } => {
                let end = Cursor::new(to.line, to.col + 1);
                self.register = Some(Register::Chars(buf.text_between(from, end)));
                if yank_only {
                    buf.move_to(from);
                } else {
                    buf.delete_between(from, end);
                }
            }
            Selection::Lines { from, to } => {
                let last = to.min(buf.line_count().saturating_sub(1));
                if yank_only {
                    self.register = Some(Register::Lines(buf.lines()[from..=last].to_vec()));
                    buf.move_to(Cursor::new(from, 0));
                } else {
                    let removed = buf.delete_lines(from, last);
                    self.register = Some(Register::Lines(removed));
                    if op == 'c' {
                        // `c` on lines leaves a blank one to type into rather
                        // than closing the gap.
                        buf.open_line(false);
                    }
                }
            }
            Selection::Block {
                top,
                bottom,
                left,
                right,
            } => {
                let last = bottom.min(buf.line_count().saturating_sub(1));
                let mut rows = Vec::new();
                for line in top..=last {
                    if yank_only {
                        let chars: Vec<char> = buf.line(line).chars().collect();
                        let a = left.min(chars.len());
                        let b = (right + 1).min(chars.len());
                        rows.push(chars[a..b].iter().collect::<String>());
                    } else {
                        rows.push(buf.delete_range_in_line(line, left, right + 1));
                    }
                }
                self.register = Some(Register::Block(rows));
                buf.move_to(Cursor::new(top, left));
            }
        }

        self.mode = if op == 'c' {
            Mode::Insert
        } else {
            Mode::Normal
        };

        // Changing a block re-enters blockwise insert, so what gets typed lands
        // on every row rather than just the first.
        if op == 'c' {
            if let Selection::Block {
                top, bottom, left, ..
            } = sel
            {
                self.block_insert = Some(BlockInsert {
                    lines: (top..=bottom.min(buf.line_count().saturating_sub(1))).collect(),
                    col: left,
                    anchor_line: top,
                    pad: false,
                });
            }
        }
        buf.clamp_cursor(self.mode == Mode::Insert);
    }

    /// Start a blockwise insert: `I` at the left edge, `A` past the right one.
    fn begin_block_insert(&mut self, buf: &mut Buffer, append: bool) {
        let Some(Selection::Block {
            top,
            bottom,
            left,
            right,
        }) = self.selection(buf)
        else {
            return;
        };
        buf.checkpoint();
        let col = if append { right + 1 } else { left };
        self.block_insert = Some(BlockInsert {
            lines: (top..=bottom.min(buf.line_count().saturating_sub(1))).collect(),
            col,
            anchor_line: top,
            pad: append,
        });
        buf.move_to(Cursor::new(top, col.min(buf.line_len(top))));
        self.mode = Mode::Insert;
    }

    /// Replicate a finished blockwise insert onto the rest of its rows.
    fn finish_block_insert(&mut self, buf: &mut Buffer) {
        let Some(bi) = self.block_insert.take() else {
            return;
        };
        // Wandering off the line being typed on abandons the replication;
        // guessing what was meant would be worse than doing nothing.
        if buf.cursor.line != bi.anchor_line || buf.cursor.col <= bi.col {
            return;
        }
        let typed: String = buf
            .line(bi.anchor_line)
            .chars()
            .skip(bi.col)
            .take(buf.cursor.col - bi.col)
            .collect();
        if typed.is_empty() {
            return;
        }
        for &line in &bi.lines {
            if line == bi.anchor_line {
                continue;
            }
            let len = buf.line_len(line);
            if bi.col <= len {
                buf.insert_str_at(line, bi.col, &typed);
            } else if bi.pad {
                // Appending past the end of a short line pads it out, so the
                // block stays a block.
                let padded = format!("{}{}", " ".repeat(bi.col - len), typed);
                buf.insert_str_at(line, len, &padded);
            }
        }
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
            Register::Block(rows) => {
                let start = buf.cursor.line;
                let col = if after {
                    (buf.cursor.col + 1).min(buf.line_len(buf.cursor.line))
                } else {
                    buf.cursor.col
                };
                for (i, text) in rows.iter().enumerate() {
                    let line = start + i;
                    while line >= buf.line_count() {
                        buf.insert_lines(buf.line_count(), &[String::new()]);
                    }
                    let len = buf.line_len(line);
                    if col <= len {
                        buf.insert_str_at(line, col, text);
                    } else {
                        let padded = format!("{}{}", " ".repeat(col - len), text);
                        buf.insert_str_at(line, len, &padded);
                    }
                }
                buf.move_to(Cursor::new(start, col));
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

        // `G` and `gg` take their count as a *line number*, not a repeat count:
        // `11G` is line eleven, not eleven trips to the end of the file.
        if matches!(m, Motion::FileStart | Motion::FileEnd) {
            let line = match (m, count) {
                (Motion::FileEnd, 1) => buf.line_count() - 1,
                (Motion::FileStart, 1) => 0,
                (_, n) => (n - 1).min(buf.line_count() - 1),
            };
            buf.move_to(Cursor::new(line, first_non_blank(buf.line(line))));
            buf.clamp_cursor(past_end);
            return;
        }

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
                Motion::Find(_) | Motion::RepeatFind { .. } => {
                    if let Some(spec) = self.effective_find(m) {
                        if let Some(col) = find_char_in_line(buf, buf.cursor, spec) {
                            buf.cursor.col = col;
                        }
                        // `;` repeats the find itself, never the repeat.
                        if matches!(m, Motion::Find(_)) {
                            self.last_find = Some(spec);
                        }
                    } else {
                        self.status = "no previous find".into();
                    }
                }
            }
            if !matches!(m, Motion::Down | Motion::Up) {
                buf.desired_col = buf.cursor.col;
            }
        }
        buf.clamp_cursor(past_end);
    }

    /// Resolve a find motion, turning `;`/`,` into the concrete thing they
    /// repeat.
    fn effective_find(&self, m: Motion) -> Option<FindSpec> {
        match m {
            Motion::Find(spec) => Some(spec),
            Motion::RepeatFind { reverse } => self.last_find.map(|mut spec| {
                if reverse {
                    spec.backward = !spec.backward;
                }
                spec
            }),
            _ => None,
        }
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

        // `e`, `$` and a forward find include the character they land on; the
        // rest do not. Without this, `d$` leaves the last character behind and
        // `dfx` leaves the `x`.
        let forward_find = self.effective_find(m).is_some_and(|spec| !spec.backward);
        let end = if forward_find || matches!(m, Motion::WordEnd | Motion::LineEnd) {
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
        ';' => Motion::RepeatFind { reverse: false },
        ',' => Motion::RepeatFind { reverse: true },
        'f' | 'F' | 't' | 'T' => {
            // The target is the next key, so this is a two-character motion.
            let target = *chars.get(i + 1)?;
            return Some((
                Motion::Find(FindSpec {
                    backward: c == 'F' || c == 'T',
                    till: c == 't' || c == 'T',
                    target,
                }),
                2,
            ));
        }
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
///
/// `g` needs its second `g`, and `f`/`F`/`t`/`T` need the character they are
/// looking for. Treating those as errors would make `df` beep instead of wait.
fn is_motion_prefix(chars: &[char], i: usize) -> bool {
    matches!(
        chars.get(i),
        Some('g') | Some('f') | Some('F') | Some('t') | Some('T')
    ) && chars.get(i + 1).is_none()
}

/// Smart case: a pattern typed in lower case matches either case, but the
/// moment it contains a capital it means it.
pub fn case_sensitive(pattern: &str) -> bool {
    pattern.chars().any(char::is_uppercase)
}

/// Character ranges where `pattern` occurs in `line`.
///
/// Character ranges, not byte ranges: everything that positions a caret counts
/// in characters, and mixing the two is how a highlight ends up half a glyph
/// out of step with the text under it.
pub fn matches_in(line: &str, pattern: &str) -> Vec<(usize, usize)> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let fold = |c: char| {
        if case_sensitive(pattern) {
            c
        } else {
            c.to_ascii_lowercase()
        }
    };
    let hay: Vec<char> = line.chars().map(fold).collect();
    let needle: Vec<char> = pattern.chars().map(fold).collect();
    if needle.len() > hay.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()] == needle[..] {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

/// The next occurrence of `pattern` from `from`, wrapping around the buffer.
fn next_match(buf: &Buffer, pattern: &str, from: Cursor, backward: bool) -> Option<Cursor> {
    // Collecting every match keeps the wrap-around trivial, and a note is
    // small enough that scanning it whole costs nothing worth saving.
    let all: Vec<Cursor> = (0..buf.line_count())
        .flat_map(|line| {
            matches_in(buf.line(line), pattern)
                .into_iter()
                .map(move |(start, _)| Cursor::new(line, start))
        })
        .collect();
    if all.is_empty() {
        return None;
    }
    if backward {
        all.iter()
            .rev()
            .find(|c| **c < from)
            .or_else(|| all.last())
            .copied()
    } else {
        all.iter()
            .find(|c| **c > from)
            .or_else(|| all.first())
            .copied()
    }
}

/// The word the caret is sitting on, for `*`.
fn word_under_cursor(buf: &Buffer) -> Option<String> {
    let chars: Vec<char> = buf.line(buf.cursor.line).chars().collect();
    let col = buf.cursor.col.min(chars.len().checked_sub(1)?);
    if class(chars[col]) != Class::Word {
        return None;
    }
    let mut start = col;
    while start > 0 && class(chars[start - 1]) == Class::Word {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < chars.len() && class(chars[end + 1]) == Class::Word {
        end += 1;
    }
    Some(chars[start..=end].iter().collect())
}

/// Where a find lands on the current line, if the target is there at all.
fn find_char_in_line(buf: &Buffer, at: Cursor, spec: FindSpec) -> Option<usize> {
    let chars: Vec<char> = buf.line(at.line).chars().collect();
    if spec.backward {
        let mut i = at.col.min(chars.len());
        while i > 0 {
            i -= 1;
            if chars[i] == spec.target {
                return Some(if spec.till { i + 1 } else { i });
            }
        }
        None
    } else {
        let mut i = at.col + 1;
        while i < chars.len() {
            if chars[i] == spec.target {
                return Some(if spec.till { i.saturating_sub(1) } else { i });
            }
            i += 1;
        }
        None
    }
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
