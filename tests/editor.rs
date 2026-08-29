//! The vim grammar and the text buffer: what each key does, asserted on
//! rather than eyeballed. None of this needs a window, which is the point.

use notes::calc;
use notes::diff::{self, Change};
use notes::indent::{self, Opened};
use notes::markdown::Block;
use notes::settings::CATALOGUE;
use notes::text::{Buffer, Cursor};
use notes::vim::{self, Mode, Selection, Vim, VimEvent, VisualKind};
use notes::web;
use pixui::{Key, Mods};

/// Type a sequence of keys, as a user would.
fn press(vim: &mut Vim, buf: &mut Buffer, s: &str) -> Vec<VimEvent> {
    let mut events = Vec::new();
    for c in s.chars() {
        let key = match c {
            ' ' => Key::Space,
            '\n' => Key::Enter,
            '\x1b' => Key::Escape,
            c => Key::Char(c),
        };
        if let Some(e) = vim.handle(buf, key, Mods::default()) {
            events.push(e);
        }
    }
    events
}

fn buffer(text: &str) -> (Vim, Buffer) {
    (Vim::new(), Buffer::from_text(text))
}

// ------------------------------------------------------------------- motions

/// Put the caret on the first occurrence of `at`, then run `cmd`.
fn at_char(text: &str, at: char, cmd: &str) -> String {
    let (mut v, mut b) = buffer(text);
    let col = text
        .chars()
        .position(|c| c == at)
        .expect("marker character is present");
    b.cursor = Cursor::new(0, col);
    press(&mut v, &mut b, cmd);
    b.to_text()
}

/// Enter blockwise visual, which is a real Ctrl chord rather than a letter.
fn ctrl_v(v: &mut Vim, b: &mut Buffer) {
    let ctrl = Mods {
        ctrl: true,
        ..Default::default()
    };
    v.handle(b, Key::Char('v'), ctrl);
}

fn note(body: &str, path: Option<&str>) -> notes::Note {
    notes::Note {
        path: path.map(std::path::PathBuf::from),
        buffer: Buffer::from_text(body),
        project: String::new(),
        seen: None,
    }
}

/// The diff as a compact string: `-` for gone, `+` for new, plain for kept.
fn shape(before: &str, after: &str) -> String {
    diff::words(before, after)
        .iter()
        .map(|p| match p.change {
            Change::Same => p.text.clone(),
            Change::Removed => format!("-[{}]", p.text),
            Change::Added => format!("+[{}]", p.text),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn lines_of(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

/// What Enter would open, pressed at the end of the last line.
fn open_end(text: &str) -> Opened {
    let lines = lines_of(text);
    let at = lines.len() - 1;
    indent::opened(&lines, at, lines[at].chars().count())
}

fn located(text: &str) -> Vec<(usize, notes::markdown::Block)> {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    notes::markdown::parse_located(&lines)
}

fn doc(text: &str) -> Vec<Block> {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    notes::markdown::parse(&lines)
}

/// The plain text of a run of spans, with the markup already removed.
fn flat(spans: &[notes::markdown::Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// The spans of the one paragraph `text` parses to.
fn para(text: &str) -> Vec<notes::markdown::Span> {
    match doc(text).into_iter().next() {
        Some(Block::Paragraph(spans)) => spans,
        other => panic!("expected a paragraph, got {other:?}"),
    }
}

/// The showcase, parsed.
fn showcase() -> Vec<Block> {
    let lines: Vec<String> = notes::showcase::SHOWCASE
        .lines()
        .map(str::to_owned)
        .collect();
    notes::markdown::parse(&lines)
}

/// Every span in the document, whatever block it came from.
fn all_spans(blocks: &[Block]) -> Vec<notes::markdown::Span> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Heading { spans, .. } | Block::Paragraph(spans) => out.extend(spans.clone()),
            Block::List(items) => out.extend(items.iter().flat_map(|i| i.spans.clone())),
            Block::Quote(inner) => {
                for (_, b) in inner {
                    out.extend(all_spans(std::slice::from_ref(b)));
                }
            }
            Block::Table { header, rows, .. } => {
                out.extend(header.iter().flatten().cloned());
                out.extend(rows.iter().flatten().flatten().cloned());
            }
            Block::Code { .. } | Block::Rule => {}
        }
    }
    out
}

fn sum(text: &str) -> String {
    calc::evaluate(text).unwrap_or_else(|why| format!("!{why}"))
}

#[test]
fn hjkl_moves_and_stops_at_the_edges() {
    let (mut v, mut b) = buffer("abc\ndefgh");
    press(&mut v, &mut b, "ll");
    assert_eq!(b.cursor, Cursor::new(0, 2));
    press(&mut v, &mut b, "l");
    assert_eq!(
        b.cursor,
        Cursor::new(0, 2),
        "cannot move past the last character"
    );
    press(&mut v, &mut b, "j");
    assert_eq!(b.cursor.line, 1);
    press(&mut v, &mut b, "hhh");
    assert_eq!(b.cursor.col, 0, "cannot move left of the first column");
}

#[test]
fn a_count_repeats_a_motion() {
    let (mut v, mut b) = buffer("abcdefghij");
    press(&mut v, &mut b, "5l");
    assert_eq!(b.cursor.col, 5);
    assert!(
        v.pending.is_empty(),
        "a completed command clears the pending keys"
    );
}

#[test]
fn word_motions_step_between_words() {
    let (mut v, mut b) = buffer("alpha beta gamma");
    press(&mut v, &mut b, "w");
    assert_eq!(b.cursor.col, 6);
    press(&mut v, &mut b, "w");
    assert_eq!(b.cursor.col, 11);
    press(&mut v, &mut b, "b");
    assert_eq!(b.cursor.col, 6);
}

#[test]
fn line_and_file_motions() {
    let (mut v, mut b) = buffer("  indented\nsecond\nthird");
    press(&mut v, &mut b, "$");
    assert_eq!(b.cursor.col, 9);
    press(&mut v, &mut b, "0");
    assert_eq!(b.cursor.col, 0);
    press(&mut v, &mut b, "^");
    assert_eq!(b.cursor.col, 2, "^ goes to the first non-blank");
    press(&mut v, &mut b, "G");
    assert_eq!(b.cursor.line, 2);
    press(&mut v, &mut b, "gg");
    assert_eq!(b.cursor.line, 0);
}

#[test]
fn j_and_k_remember_the_column_across_a_short_line() {
    let (mut v, mut b) = buffer("aaaaaaaaaa\nbb\ncccccccccc");
    press(&mut v, &mut b, "8l");
    assert_eq!(b.cursor.col, 8);
    press(&mut v, &mut b, "j");
    assert_eq!(b.cursor.col, 1, "clamped to the short line");
    press(&mut v, &mut b, "j");
    assert_eq!(
        b.cursor.col, 8,
        "and restored on the way out the other side"
    );
}

// -------------------------------------------------------------------- pending

#[test]
fn an_incomplete_command_waits_for_more_keys() {
    let (mut v, mut b) = buffer("hello world");
    press(&mut v, &mut b, "d");
    assert_eq!(v.pending, "d", "`d` alone is a prefix, not an error");
    assert_eq!(b.to_text(), "hello world", "and nothing has happened yet");
    press(&mut v, &mut b, "w");
    assert_eq!(b.to_text(), "world");
    assert!(v.pending.is_empty());
}

#[test]
fn an_invalid_sequence_is_discarded() {
    let (mut v, mut b) = buffer("hello");
    press(&mut v, &mut b, "Z");
    assert!(
        v.pending.is_empty(),
        "a key that cannot start a command clears the buffer"
    );
    assert_eq!(b.to_text(), "hello");
}

// ------------------------------------------------------------------- editing

#[test]
fn x_deletes_the_character_under_the_caret() {
    let (mut v, mut b) = buffer("hello");
    press(&mut v, &mut b, "x");
    assert_eq!(b.to_text(), "ello");
    press(&mut v, &mut b, "2x");
    assert_eq!(b.to_text(), "lo", "with a count, several at once");
}

#[test]
fn dd_deletes_whole_lines() {
    let (mut v, mut b) = buffer("one\ntwo\nthree\nfour");
    press(&mut v, &mut b, "dd");
    assert_eq!(b.to_text(), "two\nthree\nfour");
    press(&mut v, &mut b, "2dd");
    assert_eq!(b.to_text(), "four");
}

#[test]
fn deleting_every_line_leaves_an_empty_buffer_not_no_buffer() {
    let (mut v, mut b) = buffer("one\ntwo");
    press(&mut v, &mut b, "2dd");
    assert_eq!(b.line_count(), 1, "a buffer always has at least one line");
    assert_eq!(b.to_text(), "");
}

#[test]
fn operators_compose_with_motions_and_counts() {
    let (mut v, mut b) = buffer("alpha beta gamma delta");
    press(&mut v, &mut b, "d2w");
    assert_eq!(b.to_text(), "gamma delta", "d2w deletes two words");

    let (mut v, mut b) = buffer("alpha beta gamma");
    press(&mut v, &mut b, "2dw");
    assert_eq!(b.to_text(), "gamma", "the count may lead instead");
}

#[test]
fn d_dollar_deletes_to_the_end_of_the_line() {
    let (mut v, mut b) = buffer("keep this away");
    press(&mut v, &mut b, "5l");
    press(&mut v, &mut b, "d$");
    assert_eq!(b.to_text(), "keep ");
}

#[test]
fn cw_removes_the_word_and_enters_insert() {
    let (mut v, mut b) = buffer("alpha beta");
    press(&mut v, &mut b, "cw");
    assert_eq!(v.mode, Mode::Insert);
    press(&mut v, &mut b, "omega ");
    assert_eq!(b.to_text(), "omega beta");
}

#[test]
fn yy_and_p_duplicate_a_line() {
    let (mut v, mut b) = buffer("first\nsecond");
    press(&mut v, &mut b, "yyp");
    assert_eq!(b.to_text(), "first\nfirst\nsecond");
}

// -------------------------------------------------------------------- insert

#[test]
fn insert_commands_place_the_caret_correctly() {
    let (mut v, mut b) = buffer("bc");
    press(&mut v, &mut b, "ia");
    assert_eq!(b.to_text(), "abc", "i inserts before the caret");

    let (mut v, mut b) = buffer("ab");
    press(&mut v, &mut b, "\x1bA!");
    assert_eq!(b.to_text(), "ab!", "A appends at the end of the line");

    let (mut v, mut b) = buffer("one");
    press(&mut v, &mut b, "otwo");
    assert_eq!(b.to_text(), "one\ntwo", "o opens a line below");

    let (mut v, mut b) = buffer("two");
    press(&mut v, &mut b, "Oone");
    assert_eq!(b.to_text(), "one\ntwo", "O opens a line above");
}

#[test]
fn escape_leaves_insert_and_steps_left() {
    let (mut v, mut b) = buffer("");
    press(&mut v, &mut b, "iabc");
    assert_eq!(b.cursor.col, 3);
    press(&mut v, &mut b, "\x1b");
    assert_eq!(v.mode, Mode::Normal);
    assert_eq!(
        b.cursor.col, 2,
        "the caret lands on the character just typed"
    );
}

#[test]
fn enter_splits_a_line_and_backspace_joins_it_again() {
    let (mut v, mut b) = buffer("abcd");
    press(&mut v, &mut b, "2li\n");
    assert_eq!(b.to_text(), "ab\ncd");
    // Backspace is not a printable character, so drive it directly.
    v.handle(&mut b, Key::Backspace, Mods::default());
    assert_eq!(b.to_text(), "abcd");
}

// ---------------------------------------------------------------------- undo

#[test]
fn undo_and_redo_walk_the_history() {
    let (mut v, mut b) = buffer("original");
    press(&mut v, &mut b, "dd");
    assert_eq!(b.to_text(), "");
    press(&mut v, &mut b, "u");
    assert_eq!(b.to_text(), "original");

    let ctrl = Mods {
        ctrl: true,
        ..Default::default()
    };
    v.handle(&mut b, Key::Char('r'), ctrl);
    assert_eq!(b.to_text(), "", "Ctrl-r redoes");
}

#[test]
fn a_whole_insert_session_undoes_as_one_step() {
    let (mut v, mut b) = buffer("x");
    press(&mut v, &mut b, "Ahello\x1b");
    assert_eq!(b.to_text(), "xhello");
    press(&mut v, &mut b, "u");
    assert_eq!(b.to_text(), "x", "typing a word is one undo, not six");
}

// -------------------------------------------------------------------- visual

#[test]
fn visual_mode_selects_and_deletes() {
    let (mut v, mut b) = buffer("alpha beta");
    press(&mut v, &mut b, "v4l");
    assert_eq!(v.mode, Mode::Visual(VisualKind::Char));
    let Some(Selection::Chars { from, to }) = v.selection(&b) else {
        panic!("charwise visual should give a charwise selection")
    };
    assert_eq!(
        (from.col, to.col),
        (0, 4),
        "v4l covers five characters, inclusive"
    );
    press(&mut v, &mut b, "d");
    assert_eq!(
        b.to_text(),
        " beta",
        "the space after the selection survives"
    );
    assert_eq!(v.mode, Mode::Normal, "the selection is consumed");
}

#[test]
fn escape_leaves_visual_mode_without_editing() {
    let (mut v, mut b) = buffer("untouched");
    press(&mut v, &mut b, "vlll\x1b");
    assert_eq!(v.mode, Mode::Normal);
    assert_eq!(b.to_text(), "untouched");
}

// ------------------------------------------------------------------- command

#[test]
fn a_colon_command_is_handed_back_to_the_application() {
    let (mut v, mut b) = buffer("note");
    press(&mut v, &mut b, ":");
    assert_eq!(v.mode, Mode::Command);
    let events = press(&mut v, &mut b, "w hello\n");
    assert_eq!(events, vec![VimEvent::Command("w hello".into())]);
    assert_eq!(v.mode, Mode::Normal);
    assert_eq!(b.to_text(), "note", "the buffer is untouched by a command");
}

#[test]
fn backspacing_past_the_colon_leaves_command_mode() {
    let (mut v, mut b) = buffer("");
    press(&mut v, &mut b, ":");
    v.handle(&mut b, Key::Backspace, Mods::default());
    assert_eq!(v.mode, Mode::Normal);
}

// -------------------------------------------------------------- text objects

#[test]
fn diw_deletes_the_word_under_the_caret() {
    assert_eq!(at_char("alpha beta gamma", 'b', "diw"), "alpha  gamma");
}

#[test]
fn daw_takes_the_trailing_space_too() {
    assert_eq!(at_char("alpha beta gamma", 'b', "daw"), "alpha gamma");
}

#[test]
fn daw_on_the_last_word_takes_the_leading_space_instead() {
    // There is no trailing whitespace to absorb, so it has to reach backwards
    // or it would leave a dangling space.
    assert_eq!(at_char("alpha beta", 'b', "daw"), "alpha");
}

#[test]
fn diw_works_from_anywhere_inside_the_word() {
    for marker in ['b', 'e', 'a'] {
        let (mut v, mut b) = buffer("one beta two");
        let col = "one beta two"
            .chars()
            .position(|c| c == marker)
            .unwrap()
            .max(4);
        b.cursor = Cursor::new(0, col);
        press(&mut v, &mut b, "diw");
        assert_eq!(b.to_text(), "one  two", "failed entering at {marker}");
    }
}

#[test]
fn ciw_replaces_the_word_and_leaves_you_in_insert() {
    let (mut v, mut b) = buffer("swap this out");
    b.cursor = Cursor::new(0, 5);
    press(&mut v, &mut b, "ciw");
    assert_eq!(v.mode, Mode::Insert);
    press(&mut v, &mut b, "that");
    assert_eq!(b.to_text(), "swap that out");
}

#[test]
fn diw_on_whitespace_deletes_the_gap() {
    // Whitespace is a class of its own, so the object under the caret is the
    // run of spaces rather than a neighbouring word.
    assert_eq!(at_char("a    b", ' ', "diw"), "ab");
}

#[test]
fn quote_objects_select_inside_and_around() {
    assert_eq!(
        at_char("say \"hello there\" now", 'h', "ci\""),
        "say \"\" now"
    );
    assert_eq!(at_char("say \"hello there\" now", 'h', "da\""), "say  now");
}

#[test]
fn a_quote_object_reaches_forward_when_the_caret_is_before_it() {
    assert_eq!(at_char("pick \"this\"", 'p', "di\""), "pick \"\"");
}

#[test]
fn bracket_objects_respect_nesting() {
    assert_eq!(at_char("f(g(x), y)", 'y', "di("), "f()");
    assert_eq!(
        at_char("f(g(x), y)", 'x', "di("),
        "f(g(), y)",
        "the inner pair wins"
    );
}

#[test]
fn bracket_objects_work_from_the_bracket_itself() {
    assert_eq!(at_char("a[one]b", '[', "di["), "a[]b");
    assert_eq!(at_char("a[one]b", ']', "da["), "ab");
}

#[test]
fn bracket_objects_span_lines() {
    let (mut v, mut b) = buffer("call(\n  arg,\n)");
    b.cursor = Cursor::new(1, 3);
    press(&mut v, &mut b, "di(");
    assert_eq!(
        b.to_text(),
        "call()",
        "depth counting has to cross line ends"
    );
}

#[test]
fn brace_and_angle_aliases_resolve() {
    assert_eq!(at_char("x{y}z", 'y', "diB"), "x{}z");
    assert_eq!(at_char("x<y>z", 'y', "di<"), "x<>z");
    assert_eq!(at_char("x(y)z", 'y', "dib"), "x()z");
}

#[test]
fn paragraph_objects_take_a_block_of_lines() {
    let (mut v, mut b) = buffer("one\ntwo\n\nthree");
    press(&mut v, &mut b, "dip");
    assert_eq!(b.to_text(), "\nthree", "ip is the run of non-blank lines");

    let (mut v, mut b) = buffer("one\ntwo\n\nthree");
    press(&mut v, &mut b, "dap");
    assert_eq!(b.to_text(), "three", "ap swallows the blank line after it");
}

#[test]
fn a_text_object_waits_for_its_second_key() {
    let (mut v, mut b) = buffer("untouched");
    press(&mut v, &mut b, "di");
    assert_eq!(v.pending, "di", "`di` is a prefix, not a command");
    assert_eq!(b.to_text(), "untouched");
}

#[test]
fn an_unknown_object_does_nothing_rather_than_something_wrong() {
    let (mut v, mut b) = buffer("untouched");
    press(&mut v, &mut b, "diz");
    assert_eq!(b.to_text(), "untouched");
    assert!(v.pending.is_empty());
}

#[test]
fn a_missing_delimiter_leaves_the_buffer_alone() {
    let (mut v, mut b) = buffer("no brackets here");
    press(&mut v, &mut b, "di(");
    assert_eq!(b.to_text(), "no brackets here");
}

#[test]
fn text_objects_extend_a_visual_selection() {
    let (mut v, mut b) = buffer("alpha beta gamma");
    b.cursor = Cursor::new(0, 7);
    press(&mut v, &mut b, "viw");
    let Some(Selection::Chars { from, to }) = v.selection(&b) else {
        panic!("visual mode has a charwise selection")
    };
    assert_eq!((from.col, to.col), (6, 9), "iw covers exactly `beta`");
    press(&mut v, &mut b, "d");
    assert_eq!(b.to_text(), "alpha  gamma");
}

#[test]
fn yiw_then_p_copies_a_word_without_changing_it() {
    let (mut v, mut b) = buffer("copy me");
    press(&mut v, &mut b, "yiw");
    assert_eq!(b.to_text(), "copy me", "yank leaves the buffer alone");
    press(&mut v, &mut b, "$p");
    assert_eq!(b.to_text(), "copy mecopy");
}

// ------------------------------------------------------------------ markdown

#[test]
fn text_survives_a_round_trip() {
    let text = "# Note\n\nbody line\n";
    assert_eq!(Buffer::from_text(text).to_text(), text);
}

// -------------------------------------------------------------------- config

#[test]
fn the_window_is_configured_to_grow_rather_than_magnify() {
    // Opting into adaptive scaling is a single builder call that is invisible
    // when missing until someone drags a window edge and the whole UI jumps a
    // size. Assert it rather than trusting it.
    let config = notes::config();
    assert_eq!(config.scaling, pixui::Scaling::Adaptive);
    // 1.5 logical points resolves to 3 physical pixels on a 2x display, which
    // no whole number of logical points can name.
    assert!((config.ui_scale - 1.5).abs() < 1e-6);
    let opening = (config.ui_scale * 2.0).round() as i32;
    assert!(
        config.scale_range.0 <= opening && opening <= config.scale_range.1,
        "the opening scale must sit inside the range it can be zoomed within"
    );
    assert!(
        config.min_width < config.width && config.min_height < config.height,
        "the minimum canvas must be smaller than the starting one, or the window \
         opens already at its minimum and can only ever grow"
    );
}

// -------------------------------------------------------------- visual modes

#[test]
fn v_toggles_off_and_switches_shape() {
    let (mut v, mut b) = buffer("one\ntwo");
    press(&mut v, &mut b, "v");
    assert_eq!(v.mode, Mode::Visual(VisualKind::Char));
    press(&mut v, &mut b, "v");
    assert_eq!(
        v.mode,
        Mode::Normal,
        "the same key again leaves visual mode"
    );

    press(&mut v, &mut b, "v");
    press(&mut v, &mut b, "V");
    assert_eq!(
        v.mode,
        Mode::Visual(VisualKind::Line),
        "a different key changes shape without leaving"
    );
}

#[test]
fn linewise_visual_takes_whole_lines_whatever_column_you_start_in() {
    let (mut v, mut b) = buffer("first\nsecond\nthird");
    press(&mut v, &mut b, "lllVj");
    let Some(Selection::Lines { from, to }) = v.selection(&b) else {
        panic!("V should give a linewise selection")
    };
    assert_eq!((from, to), (0, 1));
    press(&mut v, &mut b, "d");
    assert_eq!(b.to_text(), "third");
}

#[test]
fn linewise_yank_and_put_duplicates_the_lines() {
    let (mut v, mut b) = buffer("alpha\nbeta\ngamma");
    press(&mut v, &mut b, "Vjy");
    assert_eq!(
        b.to_text(),
        "alpha\nbeta\ngamma",
        "yank leaves the buffer alone"
    );
    press(&mut v, &mut b, "p");
    assert_eq!(b.to_text(), "alpha\nalpha\nbeta\nbeta\ngamma");
}

#[test]
fn linewise_change_leaves_a_blank_line_to_type_into() {
    let (mut v, mut b) = buffer("one\ntwo\nthree");
    press(&mut v, &mut b, "Vjc");
    assert_eq!(v.mode, Mode::Insert);
    press(&mut v, &mut b, "new");
    assert_eq!(b.to_text(), "new\nthree");
}

#[test]
fn blockwise_visual_selects_a_rectangle() {
    let (mut v, mut b) = buffer("abcdef\nghijkl\nmnopqr");
    press(&mut v, &mut b, "l");
    ctrl_v(&mut v, &mut b);
    press(&mut v, &mut b, "jjl");
    let Some(Selection::Block {
        top,
        bottom,
        left,
        right,
    }) = v.selection(&b)
    else {
        panic!("Ctrl-v should give a block selection")
    };
    assert_eq!((top, bottom, left, right), (0, 2, 1, 2));
}

#[test]
fn blockwise_delete_cuts_a_column_out_of_every_line() {
    let (mut v, mut b) = buffer("abcdef\nghijkl\nmnopqr");
    press(&mut v, &mut b, "l");
    ctrl_v(&mut v, &mut b);
    press(&mut v, &mut b, "jjld");
    assert_eq!(b.to_text(), "adef\ngjkl\nmpqr");
}

#[test]
fn blockwise_insert_replicates_to_every_row() {
    // The whole reason blockwise exists: type once, apply everywhere.
    let (mut v, mut b) = buffer("one\ntwo\nthree");
    ctrl_v(&mut v, &mut b);
    press(&mut v, &mut b, "jj");
    press(&mut v, &mut b, "I- \x1b");
    assert_eq!(b.to_text(), "- one\n- two\n- three");
}

#[test]
fn blockwise_append_pads_short_lines_so_the_block_stays_square() {
    let (mut v, mut b) = buffer("aaa\nb\nccc");
    press(&mut v, &mut b, "ll");
    ctrl_v(&mut v, &mut b);
    press(&mut v, &mut b, "jj");
    press(&mut v, &mut b, "A!\x1b");
    assert_eq!(b.to_text(), "aaa!\nb  !\nccc!");
}

#[test]
fn a_blockwise_insert_that_types_nothing_changes_nothing() {
    let (mut v, mut b) = buffer("one\ntwo");
    ctrl_v(&mut v, &mut b);
    press(&mut v, &mut b, "j");
    press(&mut v, &mut b, "I\x1b");
    assert_eq!(b.to_text(), "one\ntwo");
}

#[test]
fn blockwise_yank_and_put_re_forms_the_rectangle() {
    let (mut v, mut b) = buffer("ab\ncd\n..\n..");
    ctrl_v(&mut v, &mut b);
    press(&mut v, &mut b, "jl");
    press(&mut v, &mut b, "y");
    press(&mut v, &mut b, "jjP");
    assert_eq!(b.to_text(), "ab\ncd\nab..\ncd..");
}

#[test]
fn o_swaps_which_end_of_the_selection_moves() {
    let (mut v, mut b) = buffer("abcdefgh");
    press(&mut v, &mut b, "3lv2l");
    let Some(Selection::Chars { from, to }) = v.selection(&b) else {
        panic!()
    };
    assert_eq!((from.col, to.col), (3, 5));

    // After `o` the cursor is on the far end, so a motion extends backwards.
    press(&mut v, &mut b, "o2h");
    let Some(Selection::Chars { from, to }) = v.selection(&b) else {
        panic!()
    };
    assert_eq!((from.col, to.col), (1, 5));
}

#[test]
fn text_objects_work_in_linewise_visual_too() {
    let (mut v, mut b) = buffer("one\ntwo\n\nthree");
    press(&mut v, &mut b, "Vip");
    let span = v.selection(&b).expect("a selection").line_span();
    assert_eq!(span, (0, 1), "ip is the run of non-blank lines");
}

#[test]
fn a_block_reports_its_columns_even_on_lines_too_short_to_reach_them() {
    // The rectangle has to stay a rectangle on screen; that is the only way to
    // see what a blockwise append is about to pad out.
    let sel = Selection::Block {
        top: 0,
        bottom: 2,
        left: 4,
        right: 6,
    };
    assert_eq!(
        sel.columns_on(1, 0),
        Some((4, 7)),
        "an empty line still shows the block"
    );
    assert_eq!(sel.columns_on(9, 80), None, "but only within its line span");
}

#[test]
fn selection_shapes_report_the_right_columns_per_line() {
    let chars = Selection::Chars {
        from: Cursor::new(1, 3),
        to: Cursor::new(3, 2),
    };
    assert_eq!(chars.columns_on(0, 10), None);
    assert_eq!(
        chars.columns_on(1, 10),
        Some((3, 10)),
        "the first line runs to its end"
    );
    assert_eq!(
        chars.columns_on(2, 10),
        Some((0, 10)),
        "middle lines are whole"
    );
    assert_eq!(
        chars.columns_on(3, 10),
        Some((0, 3)),
        "the last stops at the cursor"
    );

    let lines = Selection::Lines { from: 1, to: 2 };
    assert_eq!(lines.columns_on(1, 7), Some((0, 7)));
    assert_eq!(lines.columns_on(3, 7), None);
}

#[test]
fn an_edit_forgets_the_column_that_j_and_k_remember() {
    // `j`/`k` return to the column you last moved to horizontally. An edit that
    // jumps the caret has to clear that, or the next vertical motion snaps to
    // wherever the caret was several commands ago — which is how a blockwise
    // paste ends up one column off.
    let (mut v, mut b) = buffer("abcdef\nghijkl\nmnopqr");
    press(&mut v, &mut b, "lll");
    assert_eq!(b.cursor.col, 3);
    press(&mut v, &mut b, "dd");
    assert_eq!(
        b.cursor.col, 0,
        "deleting a line puts the caret at its start"
    );
    press(&mut v, &mut b, "j");
    assert_eq!(b.cursor.col, 0, "and `j` must not resurrect the old column");
}

#[test]
fn g_takes_its_count_as_a_line_number_not_a_repeat() {
    let (mut v, mut b) = buffer("one\ntwo\nthree\nfour\nfive");
    press(&mut v, &mut b, "3G");
    assert_eq!(
        b.cursor.line, 2,
        "3G is line three, not three trips to the end"
    );
    press(&mut v, &mut b, "G");
    assert_eq!(b.cursor.line, 4, "a bare G is still the last line");
    press(&mut v, &mut b, "2gg");
    assert_eq!(b.cursor.line, 1, "and gg counts the same way");
    press(&mut v, &mut b, "99G");
    assert_eq!(b.cursor.line, 4, "past the end clamps to the last line");
}

#[test]
fn g_lands_on_the_first_non_blank_of_its_line() {
    let (mut v, mut b) = buffer("one\n    indented\nthree");
    press(&mut v, &mut b, "2G");
    assert_eq!((b.cursor.line, b.cursor.col), (1, 4));
}

// -------------------------------------------------------------------- search

#[test]
fn slash_search_jumps_to_the_next_match_and_wraps() {
    let (mut v, mut b) = buffer("alpha\nbeta\nalpha again");
    press(&mut v, &mut b, "/alpha\n");
    assert_eq!(
        b.cursor,
        Cursor::new(2, 0),
        "search starts *after* the cursor"
    );
    press(&mut v, &mut b, "n");
    assert_eq!(b.cursor, Cursor::new(0, 0), "and wraps around the end");
}

#[test]
fn capital_n_searches_the_other_way() {
    let (mut v, mut b) = buffer("x\nhit\ny\nhit\nz");
    press(&mut v, &mut b, "/hit\n");
    assert_eq!(b.cursor.line, 1);
    press(&mut v, &mut b, "n");
    assert_eq!(b.cursor.line, 3);
    press(&mut v, &mut b, "N");
    assert_eq!(b.cursor.line, 1);
}

#[test]
fn question_mark_searches_backwards() {
    let (mut v, mut b) = buffer("hit\nmiddle\nhit\nend");
    press(&mut v, &mut b, "G");
    press(&mut v, &mut b, "?hit\n");
    assert_eq!(b.cursor.line, 2);
}

#[test]
fn search_is_smart_about_case() {
    // A lower-case pattern matches either case; a capital means it.
    let (mut v, mut b) = buffer("start\nHello\nhello");
    press(&mut v, &mut b, "/hello\n");
    assert_eq!(b.cursor.line, 1, "lower case matches the capitalised one");

    let (mut v, mut b) = buffer("start\nhello\nHello");
    press(&mut v, &mut b, "/Hello\n");
    assert_eq!(
        b.cursor.line, 2,
        "a capital in the pattern is taken literally"
    );
}

#[test]
fn a_pattern_that_is_not_there_leaves_the_cursor_alone() {
    let (mut v, mut b) = buffer("one\ntwo");
    press(&mut v, &mut b, "jl");
    let before = b.cursor;
    press(&mut v, &mut b, "/nowhere\n");
    assert_eq!(b.cursor, before);
    assert!(v.status.to_lowercase().contains("not found"));
}

#[test]
fn star_searches_for_the_word_under_the_cursor() {
    let (mut v, mut b) = buffer("needle in a\nhaystack with needle");
    press(&mut v, &mut b, "*");
    assert_eq!(b.cursor.line, 1, "jumps to the next occurrence");
    assert_eq!(v.search_pattern(), Some("needle"));
}

#[test]
fn escape_clears_the_search_highlight() {
    let (mut v, mut b) = buffer("a\nfind me");
    press(&mut v, &mut b, "/find\n");
    assert!(v.search_pattern().is_some());
    press(&mut v, &mut b, "\x1b");
    assert_eq!(
        v.search_pattern(),
        None,
        "the highlight should not outstay its welcome"
    );
}

#[test]
fn matches_are_reported_as_character_ranges() {
    assert_eq!(vim::matches_in("abcabc", "bc"), vec![(1, 3), (4, 6)]);
    assert_eq!(
        vim::matches_in("aaaa", "aa"),
        vec![(0, 2), (2, 4)],
        "matches do not overlap"
    );
    assert_eq!(
        vim::matches_in("anything", ""),
        vec![],
        "an empty pattern matches nothing"
    );
    assert_eq!(vim::matches_in("short", "much longer"), vec![]);
}

// ---------------------------------------------------------------------- find

#[test]
fn f_moves_to_the_character_and_t_stops_before_it() {
    let (mut v, mut b) = buffer("alpha,beta,gamma");
    press(&mut v, &mut b, "f,");
    assert_eq!(b.cursor.col, 5);
    press(&mut v, &mut b, "0t,");
    assert_eq!(b.cursor.col, 4, "t stops one short");
}

#[test]
fn capital_f_and_t_search_backwards() {
    let (mut v, mut b) = buffer("a,b,c");
    press(&mut v, &mut b, "$");
    press(&mut v, &mut b, "F,");
    assert_eq!(b.cursor.col, 3);
    press(&mut v, &mut b, "$T,");
    assert_eq!(b.cursor.col, 4, "T stops one past the target, coming back");
}

#[test]
fn a_count_finds_the_nth_occurrence() {
    let (mut v, mut b) = buffer("a.b.c.d");
    press(&mut v, &mut b, "3f.");
    assert_eq!(b.cursor.col, 5);
}

#[test]
fn semicolon_repeats_a_find_and_comma_reverses_it() {
    let (mut v, mut b) = buffer("a-b-c-d");
    press(&mut v, &mut b, "f-");
    assert_eq!(b.cursor.col, 1);
    press(&mut v, &mut b, ";");
    assert_eq!(b.cursor.col, 3);
    press(&mut v, &mut b, ";");
    assert_eq!(b.cursor.col, 5);
    press(&mut v, &mut b, ",");
    assert_eq!(b.cursor.col, 3, "comma goes back the other way");
}

#[test]
fn find_composes_with_operators_and_is_inclusive_forwards() {
    let (mut v, mut b) = buffer("keep this, and this");
    press(&mut v, &mut b, "df,");
    assert_eq!(b.to_text(), " and this", "df, takes the comma too");

    let (mut v, mut b) = buffer("keep this, and this");
    press(&mut v, &mut b, "dt,");
    assert_eq!(b.to_text(), ", and this", "dt, stops before it");
}

#[test]
fn a_find_waits_for_its_target_character() {
    let (mut v, mut b) = buffer("untouched");
    press(&mut v, &mut b, "df");
    assert_eq!(v.pending, "df", "`df` is a prefix, not an error");
    assert_eq!(b.to_text(), "untouched");
}

#[test]
fn a_find_that_misses_does_not_move() {
    let (mut v, mut b) = buffer("abc");
    press(&mut v, &mut b, "fz");
    assert_eq!(b.cursor.col, 0);
}

#[test]
fn a_find_does_not_run_off_the_line() {
    let (mut v, mut b) = buffer("abc\nxbz");
    press(&mut v, &mut b, "fz");
    assert_eq!(
        b.cursor,
        Cursor::new(0, 0),
        "the z on the next line is not a candidate"
    );
}

// -------------------------------------------------------------- note filter

#[test]
fn an_empty_filter_keeps_everything() {
    let n = note("# Title\n\nbody", Some("a.md"));
    assert!(notes::note_matches(&n, ""));
}

#[test]
fn the_filter_looks_at_the_title_the_filename_and_the_body() {
    let n = note("# Shopping\n\nmilk and honey", Some("groceries.md"));
    assert!(notes::note_matches(&n, "shopping"), "the title");
    assert!(notes::note_matches(&n, "groceries"), "the filename");
    assert!(
        notes::note_matches(&n, "honey"),
        "and the body — what you half-remember is rarely in the title"
    );
    assert!(!notes::note_matches(&n, "bicycle"));
}

#[test]
fn the_filter_ignores_case() {
    let n = note("# Vim Keys\n\nMotions", Some("vim-keys.md"));
    assert!(notes::note_matches(&n, "vim"));
    assert!(notes::note_matches(&n, "motions"));
}

#[test]
fn a_note_with_no_file_still_filters_on_its_text() {
    let n = note("scratch thoughts", None);
    assert!(notes::note_matches(&n, "thoughts"));
    assert!(!notes::note_matches(&n, "untitled"));
}

// -------------------------------------------------------------------- rename

#[test]
fn text_that_did_not_change_has_no_diff() {
    let pieces = diff::words("the same words", "the same words");
    assert!(diff::is_empty(&pieces));
    assert_eq!(pieces.len(), 1, "and it is one run, not one per word");
}

#[test]
fn a_replaced_word_shows_both_sides() {
    assert_eq!(
        shape("teh quick fox", "the quick fox"),
        "-[teh] +[the] quick fox"
    );
}

#[test]
fn words_added_and_removed_are_found_where_they_are() {
    assert_eq!(shape("one two", "one and two"), "one +[and] two");
    assert_eq!(shape("one and two", "one two"), "one -[and] two");
    assert_eq!(shape("", "all new"), "+[all new]");
}

#[test]
fn a_run_of_changes_is_one_piece() {
    // The renderer paints a band per piece, so neighbouring words that changed
    // the same way have to arrive as one.
    let pieces = diff::words("a b c d", "a x y d");
    let changed: Vec<&str> = pieces
        .iter()
        .filter(|p| p.change != Change::Same)
        .map(|p| p.text.as_str())
        .collect();
    assert_eq!(changed, vec!["b c", "x y"]);
}

#[test]
fn line_breaks_survive_as_pieces_of_their_own() {
    // The panel breaks its row on them rather than drawing them, so they must
    // not be swallowed into the words either side.
    let pieces = diff::words("one\ntwo", "one\ntwo three");
    assert!(pieces.iter().any(|p| p.text == "\n"));
    assert_eq!(pieces.last().unwrap().text, "three");
}

#[test]
fn a_search_lights_up_as_it_is_typed() {
    use notes::text::Buffer;
    use notes::vim::Vim;
    use pixui::{Key, Mods};

    let mut buf = Buffer::from_text("alpha beta\ngamma delta\nbeta again");
    let mut vim = Vim::new();
    assert_eq!(vim.search_pattern(), None, "nothing to light up yet");

    let none = Mods::default();
    vim.handle(&mut buf, Key::Char('/'), none);
    assert_eq!(
        vim.search_pattern(),
        None,
        "an empty pattern matches nothing"
    );

    vim.handle(&mut buf, Key::Char('b'), none);
    assert_eq!(
        vim.search_pattern(),
        Some("b"),
        "one character is a pattern"
    );
    vim.handle(&mut buf, Key::Char('e'), none);
    vim.handle(&mut buf, Key::Char('t'), none);
    assert_eq!(vim.search_pattern(), Some("bet"));
    // Backspace takes it back with it.
    vim.handle(&mut buf, Key::Backspace, none);
    assert_eq!(vim.search_pattern(), Some("be"));

    // Committed, and it stays lit after the prompt closes.
    vim.handle(&mut buf, Key::Enter, none);
    assert_eq!(vim.search_pattern(), Some("be"));

    // A search abandoned leaves the note looking the way it did.
    vim.handle(&mut buf, Key::Char('/'), none);
    vim.handle(&mut buf, Key::Char('z'), none);
    assert_eq!(vim.search_pattern(), Some("z"));
    vim.handle(&mut buf, Key::Escape, none);
    assert_eq!(vim.search_pattern(), Some("be"), "the last committed one");
}

#[test]
fn the_view_can_reach_the_end_of_a_wrapping_note() {
    use notes::last_top;
    use notes::text::Buffer;

    // Twelve short lines in a pane eight rows tall: the last eight lines fit,
    // so the view stops with the fifth line at the top.
    let plain = Buffer::from_text(
        &(1..=12)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert_eq!(last_top(&plain, 40, 8), 4);

    // The same twelve lines, except the last one wraps into four rows. Those
    // rows crowd out three of the lines that used to fit, so the view has to be
    // allowed three lines further down than a count of lines would say.
    let mut lines: Vec<String> = (1..=11).map(|n| format!("line {n}")).collect();
    lines.push("x".repeat(160));
    let wrapped = Buffer::from_text(&lines.join("\n"));
    assert_eq!(last_top(&wrapped, 40, 8), 7);

    // A note shorter than the pane never scrolls at all.
    let short = Buffer::from_text("one\ntwo");
    assert_eq!(last_top(&short, 40, 8), 0);
}

#[test]
fn enter_carries_a_bullet_down() {
    assert_eq!(open_end("- one"), Opened::With("- ".into()));
    assert_eq!(open_end("* one"), Opened::With("* ".into()));
    assert_eq!(open_end("+ one"), Opened::With("+ ".into()));
}

#[test]
fn enter_keeps_the_nesting_it_found() {
    assert_eq!(open_end("- one\n  - nested"), Opened::With("  - ".into()));
    assert_eq!(
        open_end("- one\n    - deeper"),
        Opened::With("    - ".into())
    );
}

#[test]
fn an_ordered_item_counts_on() {
    assert_eq!(open_end("1. one"), Opened::With("2. ".into()));
    assert_eq!(open_end("9. nine"), Opened::With("10. ".into()));
    // A list that changes punctuation halfway down stops being one.
    assert_eq!(open_end("1) one"), Opened::With("2) ".into()));
}

#[test]
fn a_new_task_is_one_you_have_not_done() {
    assert_eq!(open_end("- [x] done"), Opened::With("- [ ] ".into()));
    assert_eq!(open_end("- [ ] todo"), Opened::With("- [ ] ".into()));
}

#[test]
fn enter_on_an_empty_item_ends_the_list() {
    assert_eq!(open_end("- one\n- "), Opened::Ending);
    assert_eq!(open_end("1. "), Opened::Ending);
    assert_eq!(open_end("- [ ] "), Opened::Ending);
}

#[test]
fn a_quote_carries_its_bar_down() {
    assert_eq!(open_end("> quoted"), Opened::With("> ".into()));
    assert_eq!(
        open_end("> - a list in a quote"),
        Opened::With("> - ".into())
    );
    // The list inside ends; the quote around it does not.
    assert_eq!(open_end("> - "), Opened::With("> ".into()));
}

#[test]
fn splitting_a_marker_in_half_does_not_make_two() {
    let lines = lines_of("- one");
    assert_eq!(indent::opened(&lines, 0, 0), Opened::Plain);
    assert_eq!(indent::opened(&lines, 0, 1), Opened::Plain);
    assert_eq!(indent::opened(&lines, 0, 2), Opened::With("- ".into()));
}

#[test]
fn prose_carries_only_its_indent() {
    assert_eq!(open_end("plain text"), Opened::Plain);
    assert_eq!(open_end("  hanging text"), Opened::With("  ".into()));
}

#[test]
fn inside_a_fence_the_indent_is_the_code_s() {
    // Markdown's markers mean nothing in here, and four spaces is what code
    // means by a level.
    assert_eq!(
        open_end("```python\ndef f():\n    return 1"),
        Opened::With("    ".into())
    );
    assert_eq!(open_end("```python\ndef f():"), Opened::With("    ".into()));
    assert_eq!(
        open_end("```rust\nfn main() {"),
        Opened::With("    ".into())
    );
    assert_eq!(open_end("```rust\nlet x = 1;"), Opened::Plain);
}

#[test]
fn a_fence_that_has_closed_is_prose_again() {
    assert!(indent::in_code(&lines_of("```\ncode\n"), 1));
    assert!(!indent::in_code(&lines_of("```\ncode\n```\nafter"), 3));
    assert_eq!(indent::step(&lines_of("```\ncode"), 1), 4);
    assert_eq!(indent::step(&lines_of("- item"), 0), 2);
}

#[test]
fn shifting_moves_the_line_and_says_how_far() {
    assert_eq!(indent::shifted("item", false, 2), ("  item".into(), 2));
    assert_eq!(indent::shifted("  item", true, 2), ("item".into(), -2));
    // Three spaces come all the way back rather than leaving one nobody meant.
    assert_eq!(indent::shifted("   item", true, 2), (" item".into(), -2));
    assert_eq!(indent::shifted("item", true, 2), ("item".into(), 0));
    assert_eq!(indent::shifted("\titem", true, 4), ("item".into(), -1));
}

#[test]
fn typing_enter_continues_the_list_for_real() {
    let (mut v, mut b) = buffer("- one");
    press(&mut v, &mut b, "A");
    v.handle(&mut b, Key::Enter, Mods::default());
    press(&mut v, &mut b, "two");
    assert_eq!(b.to_text(), "- one\n- two");
    assert_eq!(b.cursor, Cursor::new(1, 5));
}

#[test]
fn a_second_enter_leaves_the_list() {
    let (mut v, mut b) = buffer("- one");
    press(&mut v, &mut b, "A");
    v.handle(&mut b, Key::Enter, Mods::default());
    v.handle(&mut b, Key::Enter, Mods::default());
    assert_eq!(b.to_text(), "- one\n", "the marker goes, the line stays");
    assert_eq!(b.cursor, Cursor::new(1, 0));
}

#[test]
fn tab_moves_the_line_a_level_at_a_time() {
    let (mut v, mut b) = buffer("- one");
    press(&mut v, &mut b, "A");
    v.handle(&mut b, Key::Tab, Mods::default());
    assert_eq!(b.to_text(), "  - one");
    assert_eq!(b.cursor.col, 7, "the caret travelled with the text");
    v.handle(
        &mut b,
        Key::Tab,
        Mods {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(b.to_text(), "- one");
    assert_eq!(b.cursor.col, 5);
}

#[test]
fn o_opens_the_next_item_rather_than_a_bare_line() {
    let (mut v, mut b) = buffer("  - nested\nafter");
    press(&mut v, &mut b, "o");
    press(&mut v, &mut b, "next");
    assert_eq!(b.to_text(), "  - nested\n  - next\nafter");
}

// ------------------------------------------------------- source line numbers

#[test]
fn every_block_knows_the_line_it_began_on() {
    // The preview's gutter is numbered with these, so they have to point at
    // the line a reader would find the block on in the source view.
    let blocks = located("# Title\n\nA paragraph\nthat runs on.\n\n- one\n- two\n\n> quoted");
    let at: Vec<usize> = blocks.iter().map(|(n, _)| *n).collect();
    assert_eq!(at, vec![0, 2, 5, 8]);
}

#[test]
fn a_blocks_line_is_where_it_opens_not_where_its_body_starts() {
    // The fence is part of the block, and is the line the source view shows
    // it starting on.
    let blocks = located("intro\n\n```rust\nlet x = 1;\n```\n\nafter");
    assert_eq!(
        blocks.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        vec![0, 2, 6]
    );
}

#[test]
fn link_definitions_do_not_shift_the_numbering() {
    // They are blanked rather than removed, precisely so everything below
    // them keeps the line it was typed on.
    let blocks = located("[ref]: https://example.com\n\nSee [a link][ref].");
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].0, 2);
}

#[test]
fn an_empty_document_parses_to_nothing() {
    assert!(doc("").is_empty());
    assert!(doc("\n\n   \n").is_empty());
}

#[test]
fn a_scheme_is_told_apart_from_a_relative_note() {
    use notes::external_scheme;
    assert_eq!(external_scheme("https://example.com"), Some("https"));
    assert_eq!(
        external_scheme("mailto:someone@example.com"),
        Some("mailto")
    );
    assert_eq!(external_scheme("HTTPS://EXAMPLE.COM"), Some("HTTPS"));
    assert_eq!(external_scheme("vim-keys.md"), None, "a relative note");
    assert_eq!(external_scheme("notes/ideas.md"), None);
    assert_eq!(external_scheme("#a-heading"), None);
    assert_eq!(
        external_scheme("C:/notes/ideas.md"),
        None,
        "a drive letter is one character, and a scheme is not"
    );
}

// ---------------------------------------------------------------- markdown:
// inline constructs

#[test]
fn a_backslash_before_anything_else_is_a_backslash() {
    assert_eq!(flat(&para(r"C:\path\to\file")), r"C:\path\to\file");
}

#[test]
fn a_code_span_drops_one_space_of_padding() {
    assert_eq!(flat(&para("`` ` ``")), "`");
}

#[test]
fn an_autolink_links_to_itself() {
    let spans = para("see <https://example.com/x> for more");
    let link = spans.iter().find(|s| s.href.is_some()).expect("a link");
    assert_eq!(link.text, "https://example.com/x");
    assert_eq!(link.href.as_deref(), Some("https://example.com/x"));
}

#[test]
fn an_autolinked_address_becomes_a_mailto() {
    let spans = para("write to <someone@example.com>");
    let link = spans.iter().find(|s| s.href.is_some()).expect("a link");
    assert_eq!(link.href.as_deref(), Some("mailto:someone@example.com"));
}

#[test]
fn a_bare_url_is_linked_where_it_stands() {
    let spans = para("go to https://example.com/a?b=c now");
    let link = spans.iter().find(|s| s.href.is_some()).expect("a link");
    assert_eq!(link.text, "https://example.com/a?b=c");
    assert_eq!(flat(&spans), "go to https://example.com/a?b=c now");
}

#[test]
fn a_bare_url_gives_back_the_punctuation_that_ends_the_sentence() {
    let spans = para("see https://example.com.");
    let link = spans.iter().find(|s| s.href.is_some()).expect("a link");
    assert_eq!(link.text, "https://example.com", "the full stop is prose");
    assert_eq!(flat(&spans), "see https://example.com.");
}

#[test]
fn a_bare_www_host_gets_a_scheme_to_open_with() {
    let spans = para("at www.example.com");
    let link = spans.iter().find(|s| s.href.is_some()).expect("a link");
    assert_eq!(link.href.as_deref(), Some("https://www.example.com"));
}

#[test]
fn reference_links_resolve_against_their_definitions() {
    let full = para("a [label][ref] b\n\n[ref]: https://example.com");
    assert_eq!(
        full.iter()
            .find(|s| s.href.is_some())
            .unwrap()
            .href
            .as_deref(),
        Some("https://example.com")
    );
}

#[test]
fn brackets_with_no_definition_stay_prose() {
    let spans = para("a [label] with nothing behind it");
    assert!(spans.iter().all(|s| s.href.is_none()));
    assert_eq!(flat(&spans), "a [label] with nothing behind it");
}

#[test]
fn a_definition_line_is_not_content() {
    let blocks = doc("text\n\n[ref]: https://example.com \"a title\"");
    assert_eq!(
        blocks.len(),
        1,
        "the definition is machinery, not a paragraph"
    );
}

#[test]
fn a_link_title_is_read_and_dropped() {
    let spans = para(r#"[a](https://example.com "the title")"#);
    let link = spans.iter().find(|s| s.href.is_some()).expect("a link");
    assert_eq!(link.href.as_deref(), Some("https://example.com"));
    assert_eq!(flat(&spans), "a", "the title is not prose either");
}

#[test]
fn a_destination_may_be_wrapped_in_angle_brackets() {
    let spans = para("[a](<https://example.com/with space>)");
    assert_eq!(
        spans
            .iter()
            .find(|s| s.href.is_some())
            .unwrap()
            .href
            .as_deref(),
        Some("https://example.com/with space")
    );
}

#[test]
fn a_link_label_keeps_its_own_emphasis() {
    let spans = para("[**loud** link](https://example.com)");
    assert_eq!(flat(&spans), "loud link");
    assert!(
        spans.iter().all(|s| s.href.is_some()),
        "every part of the label points at the target"
    );
    assert!(
        spans.iter().any(|s| s.bold),
        "and the bold part is still bold"
    );
}

// ---------------------------------------------------------------- markdown:
// block constructs

#[test]
fn two_trailing_spaces_ask_for_a_line_break() {
    let spans = para("first line  \nsecond line");
    assert_eq!(
        flat(&spans),
        "first line\nsecond line",
        "a hard break survives into the text as a newline"
    );
}

#[test]
fn a_trailing_backslash_asks_for_one_too() {
    assert_eq!(flat(&para("first\\\nsecond")), "first\nsecond");
}

#[test]
fn a_soft_wrap_is_not_a_line_break() {
    assert_eq!(
        flat(&para("first line\nsecond line")),
        "first line second line"
    );
}

#[test]
fn every_link_in_the_showcase_resolves() {
    let spans = all_spans(&showcase());
    let links: Vec<_> = spans.iter().filter(|s| s.href.is_some()).collect();
    assert!(
        links.len() >= 12,
        "got {} links, expected the lot",
        links.len()
    );
    assert!(
        links.iter().all(|s| !s.href.as_deref().unwrap().is_empty()),
        "a reference with no definition would come through empty"
    );
    let hrefs: Vec<&str> = links.iter().map(|s| s.href.as_deref().unwrap()).collect();
    assert!(hrefs.contains(&"mailto:someone@example.com"), "an address");
    assert!(
        hrefs.contains(&"https://www.example.com"),
        "a bare www host"
    );
    assert!(
        hrefs.contains(&"https://example.com/full"),
        "a full reference"
    );
    assert!(
        hrefs.contains(&"https://example.com/collapsed"),
        "a collapsed one"
    );
    assert!(
        hrefs.contains(&"https://example.com/shortcut"),
        "a shortcut"
    );
    assert!(hrefs.contains(&"welcome.md"), "a link to another note");
}

#[test]
fn the_catalogue_points_at_files_it_names() {
    for weights in CATALOGUE {
        assert!(
            weights.url.ends_with(weights.file),
            "{} is fetched from a url that ends in a different file",
            weights.label
        );
        assert!(weights.megabytes > 100, "{} has no size", weights.label);
    }
}

#[test]
fn a_yank_goes_out_to_the_system_clipboard() {
    let (mut v, mut b) = buffer("first\nsecond");
    press(&mut v, &mut b, "yy");
    assert_eq!(
        pixui::clipboard::paste().as_deref(),
        Some("first\n"),
        "a whole line, ending in the newline that says it was a whole line"
    );
    press(&mut v, &mut b, "jvey");
    assert_eq!(
        pixui::clipboard::paste().as_deref(),
        Some("second"),
        "and a charwise yank goes out without one"
    );
    // Deleting is copying too, the way it is in every other editor.
    press(&mut v, &mut b, "dd");
    assert_eq!(pixui::clipboard::paste().as_deref(), Some("second\n"));
}

#[test]
fn text_from_outside_is_what_p_pastes() {
    let (mut v, mut b) = buffer("here");
    pixui::clipboard::copy("from elsewhere");
    press(&mut v, &mut b, "p");
    assert_eq!(
        b.to_text(),
        "hfrom elsewhereere",
        "no newline on it, so it goes in charwise after the cursor"
    );
}

#[test]
fn a_yank_keeps_its_shape_through_the_clipboard() {
    // The round trip must not quietly turn `yy` into a charwise put: the
    // register is only replaced when what the clipboard holds came from
    // somewhere else.
    let (mut v, mut b) = buffer("alpha\nbeta");
    press(&mut v, &mut b, "yyp");
    assert_eq!(b.to_text(), "alpha\nalpha\nbeta");
}

#[test]
fn the_clipboard_keys_do_what_the_vim_keys_do() {
    let (mut v, mut b) = buffer("one\ntwo");
    v.copy_out(&mut b);
    assert_eq!(pixui::clipboard::paste().as_deref(), Some("one\n"));
    assert_eq!(b.cursor, Cursor::new(0, 0), "copying moves nothing");

    press(&mut v, &mut b, "j");
    v.cut_out(&mut b);
    assert_eq!(b.to_text(), "one");
    assert_eq!(pixui::clipboard::paste().as_deref(), Some("two\n"));

    v.paste_in(&mut b);
    assert_eq!(b.to_text(), "one\ntwo", "and it comes back where it was");
}

#[test]
fn pasting_while_typing_puts_the_text_at_the_caret() {
    let (mut v, mut b) = buffer("say  here");
    pixui::clipboard::copy("it");
    press(&mut v, &mut b, "0lllli");
    assert_eq!(v.mode, Mode::Insert);
    v.paste_in(&mut b);
    assert_eq!(b.to_text(), "say it here");
    assert_eq!(b.cursor.col, 6, "the caret ends after what was pasted");
}

#[test]
fn pasting_into_a_search_pattern_types_it_rather_than_the_note() {
    let (mut v, mut b) = buffer("alpha\nbeta gamma\nalpha");
    pixui::clipboard::copy("gamma\nand more\n");
    press(&mut v, &mut b, "/");
    v.paste_in(&mut b);
    assert_eq!(
        v.cmdline, "gamma",
        "one line, because a pattern is one line"
    );
    press(&mut v, &mut b, "\n");
    assert_eq!(b.cursor.line, 1, "and then it is a search like any other");
    assert_eq!(
        b.to_text(),
        "alpha\nbeta gamma\nalpha",
        "the note is untouched"
    );
}

#[test]
fn a_header_is_not_the_head() {
    // The bug this exists for: `<head` is the beginning of `<header` too, so
    // stripping the document head took everything down to the end of the first
    // header with it - which on an encyclopaedia page is the article.
    let page = "<html><head><title>t</title></head><body><header>menu</header>\
                <main><h1>The Title</h1><p>The first paragraph.</p></main></body></html>";
    let text = web::readable(page);
    assert!(
        text.contains("The first paragraph."),
        "the article survives: {text:?}"
    );
    assert!(text.contains("# The Title"), "and its heading is a heading");
    assert!(
        !text.contains("<title>t</title>"),
        "the document head is gone"
    );
}

#[test]
fn a_page_comes_back_as_something_worth_reading() {
    let page = "<html><body><nav>Home About</nav><main>\
                <h2>Beds</h2><p>Four raised beds.</p>\
                <ul><li>legumes</li><li>brassicas</li></ul>\
                <script>var junk = 1;</script></main><footer>copyright</footer></body></html>";
    let text = web::readable(page);
    assert!(
        text.contains("## Beds"),
        "headings keep their level: {text:?}"
    );
    assert!(
        text.contains("- legumes") && text.contains("- brassicas"),
        "list items are a list"
    );
    assert!(!text.contains("var junk"), "script is not prose");
    assert!(!text.contains("copyright"), "nor is the footer");
    assert!(!text.contains('<'), "and no markup survives");
}

#[test]
fn the_article_is_preferred_when_the_page_says_where_it_is() {
    let page = "<html><body><div>chrome everywhere</div>\
                <article><p>the actual thing</p></article><div>more chrome</div></body></html>";
    let text = web::readable(page);
    assert!(text.contains("the actual thing"));
    assert!(
        !text.contains("chrome"),
        "half a page is furniture: {text:?}"
    );
}

#[test]
fn blank_lines_are_not_paid_for_twice() {
    let text = web::readable("<p>one</p><p></p><p></p><p></p><p>two</p>");
    assert!(
        !text.contains("\n\n\n"),
        "every blank line is a token: {text:?}"
    );
}

#[test]
fn something_that_is_not_a_web_address_is_refused_before_it_is_fetched() {
    assert!(web::fetch("notes/water.md").is_err());
    assert!(web::fetch("file:///etc/passwd").is_err());
    assert!(
        web::release("llama.cpp").is_err(),
        "a repo is owner and name"
    );
}

#[test]
fn arithmetic_is_worked_out_rather_than_remembered() {
    // The one the model gets confidently and nearly right.
    assert_eq!(sum("384 * 517"), "198528");
    assert_eq!(sum("2 + 3 * 4"), "14", "times binds tighter than plus");
    assert_eq!(sum("(2 + 3) * 4"), "20");
    assert_eq!(sum("10 / 4"), "2.5");
    assert_eq!(sum("10 % 3"), "1");
    assert_eq!(sum("-5 + 2"), "-3");
    assert_eq!(sum("- -5"), "5");
}

#[test]
fn powers_bind_to_the_right() {
    assert_eq!(sum("2^3^2"), "512", "two to the ninth, not eight squared");
    assert_eq!(sum("2^10"), "1024");
    assert_eq!(
        sum("-2^2"),
        "-4",
        "the minus is applied after, as everyone writes it"
    );
    assert_eq!(sum("2^-1"), "0.5");
}

#[test]
fn a_tenth_and_a_fifth_come_out_as_people_write_them() {
    // Binary floating point cannot hold a tenth; handing somebody
    // 0.30000000000000004 is answering a question they did not ask.
    assert_eq!(sum("0.1 + 0.2"), "0.3");
    assert_eq!(sum("1 / 3"), "0.333333333333");
    assert_eq!(sum("2.5 * 4"), "10", "and a whole number is a whole number");
}

#[test]
fn the_functions_and_constants_it_claims_to_have() {
    assert_eq!(sum("sqrt(144)"), "12");
    assert_eq!(sum("abs(-7)"), "7");
    assert_eq!(sum("round(2.6)"), "3");
    assert_eq!(sum("floor(2.9)"), "2");
    assert_eq!(sum("ceil(2.1)"), "3");
    assert_eq!(sum("min(3, 1, 2)"), "1");
    assert_eq!(sum("max(3, 1, 2)"), "3");
    assert_eq!(sum("log(1000)"), "3");
    assert_eq!(sum("log(8, 2)"), "3");
    assert_eq!(sum("pow(2, 8)"), "256");
    assert!(sum("pi").starts_with("3.14159"));
    assert_eq!(sum("round(sin(0))"), "0");
}

#[test]
fn a_sum_written_the_way_somebody_would_write_it() {
    assert_eq!(
        sum("1_250 * 2"),
        "2500",
        "a number can be grouped for reading"
    );
    assert_eq!(
        sum("min(3,1)"),
        "1",
        "and a comma is the separator between arguments, not inside a number - \
         there is no telling `1,250` from `min(3,1)` and this is the one that has to work"
    );
    assert_eq!(sum("3 × 4"), "12", "the other signs for times and divide");
    assert_eq!(sum("12 ÷ 4"), "3");
    assert_eq!(sum("[2 + 3] * 2"), "10", "and the other brackets");
    assert_eq!(sum("  7  +  1  "), "8");
}

#[test]
fn a_sum_that_does_not_work_says_why() {
    assert_eq!(sum("1 / 0"), "!that divides by zero");
    assert!(sum("2 +").starts_with('!'), "half a sum is not a sum");
    assert!(sum("(2 + 3").contains("not closed"));
    assert!(sum("sqrt(-1)").contains("no square root"));
    assert!(sum("frobnicate(2)").contains("not something this knows"));
    assert!(sum("").contains("no sum in it"), "{}", sum(""));
    assert!(sum("2 & 3").contains("not something this can work out"));
    assert!(
        sum("2 3").contains("left over"),
        "two numbers side by side is a typo"
    );
}
