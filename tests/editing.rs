//! Tests for the parts of the notes app that are pure logic: the vim grammar,
//! the markdown highlighter, and line wrapping.
//!
//! None of this needs a window, which is the point — the editing model was kept
//! separate from the drawing so that `dw` can be asserted on rather than
//! eyeballed.

use notes::markdown::{self, Tok};
use notes::text::{Buffer, Cursor};
use notes::vim::{self, Mode, Selection, Vim, VimEvent, VisualKind};
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
fn a_heading_separates_its_marker_from_its_text() {
    let spans = markdown::highlight("## Try it", false);
    assert_eq!(spans[0].tok, Tok::Marker);
    assert_eq!(spans[0].text, "## ");
    assert_eq!(spans[1].tok, Tok::Heading);
    assert!(spans[1].bold);
}

#[test]
fn bold_delimiters_are_their_own_dim_spans() {
    // The faux-bold double-strike merges two adjacent asterisks into a blob, so
    // the markers must not be drawn bold.
    let spans = markdown::highlight("a **b** c", false);
    let bold: Vec<&str> = spans
        .iter()
        .filter(|s| s.bold)
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(bold, vec!["b"], "only the content is bold");
    assert!(spans.iter().any(|s| s.text == "**" && s.tok == Tok::Marker));
}

#[test]
fn code_spans_and_links_are_recognised() {
    let spans = markdown::highlight("run `cargo test` first", false);
    assert!(spans
        .iter()
        .any(|s| s.tok == Tok::Code && s.text == "cargo test"));

    let spans = markdown::highlight("see [the docs](x.md)", false);
    assert!(spans
        .iter()
        .any(|s| s.tok == Tok::Link && s.text == "the docs"));
}

#[test]
fn everything_inside_a_fence_is_code() {
    let spans = markdown::highlight("# not a heading in here", true);
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].tok, Tok::Code);
}

#[test]
fn list_markers_are_detected_including_ordered_ones() {
    for line in ["- item", "* item", "1. item", "2) item"] {
        let spans = markdown::highlight(line, false);
        assert_eq!(
            spans[0].tok,
            Tok::Marker,
            "{line} should start with a marker"
        );
    }
}

#[test]
fn a_title_comes_from_the_first_heading() {
    let lines: Vec<String> = "\n# Real Title\n\nbody"
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(markdown::derive_title(&lines, 24), "Real Title");

    let lines: Vec<String> = vec!["just prose".into()];
    assert_eq!(markdown::derive_title(&lines, 24), "just prose");
}

#[test]
fn a_preview_skips_the_title_and_strips_the_markup() {
    let lines: Vec<String> = ["# Title", "", "- some **bold** text", "more"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let preview = markdown::preview(&lines, 2, 40);
    assert_eq!(
        preview,
        vec!["some bold text".to_string(), "more".to_string()]
    );
}

// ------------------------------------------------------------------ wrapping

#[test]
fn wrapping_breaks_at_spaces() {
    let text = "one two three four";
    let ranges = markdown::wrap_ranges(text, 8);
    let rows: Vec<String> = ranges
        .iter()
        .map(|(a, b)| text.chars().skip(*a).take(b - a).collect())
        .collect();
    assert!(
        rows.iter().all(|r| r.chars().count() <= 8),
        "no row exceeds the width"
    );
    assert_eq!(
        rows.concat().replace(' ', ""),
        "onetwothreefour",
        "no text is lost"
    );
}

#[test]
fn a_word_longer_than_the_width_is_broken_rather_than_overflowing() {
    let ranges = markdown::wrap_ranges("supercalifragilistic", 6);
    assert!(ranges.len() > 1);
    assert!(ranges.iter().all(|(a, b)| b - a <= 6));
}

#[test]
fn a_short_line_is_a_single_row() {
    assert_eq!(markdown::wrap_ranges("short", 40), vec![(0, 5)]);
    assert_eq!(markdown::wrap_ranges("", 40), vec![(0, 0)]);
}

#[test]
fn the_caret_maps_onto_the_row_it_falls_in() {
    let ranges = markdown::wrap_ranges("one two three", 4);
    let (row, col) = markdown::locate(&ranges, 0);
    assert_eq!((row, col), (0, 0));
    let last = ranges.len() - 1;
    let (row, _) = markdown::locate(&ranges, ranges[last].1);
    assert_eq!(
        row, last,
        "a caret past the final character stays on the last row"
    );
}

#[test]
fn slicing_spans_keeps_their_styles() {
    let spans = markdown::highlight("- a **bold** word", false);
    let sliced = markdown::slice_spans(&spans, 0, 6);
    let text: String = sliced.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        text.chars().count(),
        6,
        "a slice is exactly the requested width"
    );
    assert!(sliced[0].tok == Tok::Marker);
}

// -------------------------------------------------------------------- buffer

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

/// Enter blockwise visual, which is a real Ctrl chord rather than a letter.
fn ctrl_v(v: &mut Vim, b: &mut Buffer) {
    let ctrl = Mods {
        ctrl: true,
        ..Default::default()
    };
    v.handle(b, Key::Char('v'), ctrl);
}

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

fn note(body: &str, path: Option<&str>) -> notes::Note {
    notes::Note {
        path: path.map(std::path::PathBuf::from),
        buffer: Buffer::from_text(body),
        project: String::new(),
        seen: None,
    }
}

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

/// A vault in its own directory, so the tests cannot tread on each other.
///
/// Under the system temp directory, not a relative path: an integration test
/// runs with the *crate* as its working directory, so `target/...` would create
/// a second, nested target directory inside the source tree.
fn vault(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("pixui-notes-tests").join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn renaming_a_note_moves_its_file() {
    let dir = vault("moves");
    let mut app = notes::Notes::open(dir.clone());
    let i = app
        .notes
        .iter()
        .position(|n| n.filename() == "welcome.md")
        .expect("the vault is seeded");

    app.rename_note(i, "greetings");
    assert_eq!(app.notes[i].filename(), "greetings.md", ".md is supplied");
    assert!(
        dir.join("greetings.md").exists(),
        "the file moved with the name"
    );
    assert!(!dir.join("welcome.md").exists(), "and did not stay behind");
}

#[test]
fn renaming_onto_an_existing_note_refuses() {
    let dir = vault("collides");
    let mut app = notes::Notes::open(dir.clone());
    let i = app
        .notes
        .iter()
        .position(|n| n.filename() == "welcome.md")
        .unwrap();

    app.rename_note(i, "ideas.md");
    assert_eq!(
        app.notes[i].filename(),
        "welcome.md",
        "silently replacing a note the user cannot get back is not an option"
    );
    assert!(dir.join("welcome.md").exists());
    assert!(app.status.to_lowercase().contains("exists"));
}

#[test]
fn an_empty_name_is_rejected() {
    let dir = vault("empty");
    let mut app = notes::Notes::open(dir);
    let before = app.notes[0].filename();
    app.rename_note(0, "   ");
    assert_eq!(app.notes[0].filename(), before);
}

#[test]
fn naming_a_note_that_was_never_saved_writes_it() {
    let dir = vault("unsaved");
    let mut app = notes::Notes::open(dir.clone());
    app.notes.push(notes::Note {
        path: None,
        buffer: Buffer::from_text("scratch"),
        project: String::new(),
        seen: None,
    });
    let i = app.notes.len() - 1;

    app.rename_note(i, "scratch");
    assert!(
        dir.join("scratch.md").exists(),
        "there was no file to move, so make one"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("scratch.md")).unwrap(),
        "scratch"
    );
    assert!(!app.notes[i].buffer.dirty, "and it counts as saved");
}

// --------------------------------------------------------------- word diff

use notes::diff::{self, Change};

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
fn a_reply_is_taken_out_of_whatever_it_arrived_in() {
    use notes::llm::clean_reply;
    // The delimiters the prompt asks for, which is the whole reason it asks.
    assert_eq!(
        clean_reply("Here is the proofread version:\n<text>the passage</text>"),
        "the passage"
    );
    // A model that opened the tag and forgot to close it still said where the
    // answer began, which is the half that matters.
    assert_eq!(clean_reply("<text>the passage"), "the passage");
    // And the fallbacks, for a model that ignored the tags entirely.
    assert_eq!(clean_reply("```\nthe passage\n```"), "the passage");
    assert_eq!(clean_reply("```markdown\nthe passage\n```"), "the passage");
    assert_eq!(clean_reply("\"the passage\""), "the passage");
    // A quotation inside the answer is not a wrapper around it.
    assert_eq!(
        clean_reply("he said \"no\" and left"),
        "he said \"no\" and left"
    );
    assert_eq!(clean_reply("  the passage  "), "the passage");
}

#[test]
fn a_reasoning_model_keeps_its_deliberation_to_itself() {
    use notes::llm::clean_reply;
    // Harmony: the thinking and the answer arrive in one stream, on separate
    // channels, and only the last one was asked for.
    assert_eq!(
        clean_reply(concat!(
            "<|channel|>analysis<|message|>The user wants it shorter. I should ",
            "drop the clause.<|end|><|start|>assistant<|channel|>final<|message|>",
            "<text>the passage</text><|return|>"
        )),
        "the passage"
    );
    // The other spelling of the same idea.
    assert_eq!(
        clean_reply("<think>\nhmm, shorter\n</think>\nthe passage"),
        "the passage"
    );
    // A model that never opened a channel is left exactly as it was.
    assert_eq!(clean_reply("the passage"), "the passage");
}

#[test]
fn a_reply_is_folded_into_the_alphabet_the_font_has() {
    // The font is 5x7 ASCII, so anything else lands in a note as a box. The
    // punctuation a model reaches for has an obvious spelling; the rest goes.
    let folded =
        notes::llm::to_ascii("it\u{2019}s \u{201c}fine\u{201d} \u{2014} really\u{2026} \u{1f389}");
    assert_eq!(folded, "it's \"fine\" -- really...");
    assert!(folded.is_ascii());
}

#[test]
fn folding_keeps_the_shape_of_what_it_was_given() {
    // The lines are the passage's shape and the indent is a list's nesting.
    // Only the gap a dropped character leaves behind is tidied away.
    let folded = notes::llm::to_ascii("one \u{2014} two\n  - a \u{1f389} b\nthree");
    assert_eq!(folded, "one -- two\n  - a b\nthree");
}

#[test]
fn the_rehearsal_backend_fixes_what_it_claims_to() {
    use notes::llm::{Ask, Backend};
    let mut stub = notes::llm::Rehearsal;
    let reply = stub
        .edit(
            &Ask {
                source: "  - teh  quick  fox\n  - adn a second".into(),
                request: "fix it".into(),
                ..Default::default()
            },
            &mut notes::llm::Quiet,
        )
        .unwrap();
    assert_eq!(
        reply, "  - the quick fox\n  - and a second",
        "typos fixed, runs of spaces collapsed, indent and lines kept"
    );
}

// --------------------------------------------------------------- the finder

fn library() -> Vec<notes::finder::Candidate> {
    [
        ("Ideas", "ideas.md"),
        ("Markdown showcase", "markdown-showcase.md"),
        ("Vim keys", "vim-keys.md"),
        ("Meeting notes", "meeting-notes.md"),
    ]
    .iter()
    .map(|(title, file)| notes::finder::Candidate {
        title: title.to_string(),
        file: file.to_string(),
    })
    .collect()
}

#[test]
fn a_note_is_found_by_a_few_of_its_letters() {
    use notes::finder::fuzzy;

    // A subsequence, not a substring: the letters in order, wherever they are.
    assert!(fuzzy("markdown-showcase.md", "mdsh").is_some());
    assert!(fuzzy("markdown-showcase.md", "show").is_some());
    // Case is not something anybody wants to think about while typing fast.
    assert!(fuzzy("Markdown showcase", "MARK").is_some());
    // Order is, though: every letter of "sid" is in "ideas.md", and none of
    // them are in that order.
    assert!(fuzzy("ideas.md", "sid").is_none());
    assert!(fuzzy("markdown-showcase.md", "zebra").is_none());

    // The characters that answered come back, so they can be lit in the list.
    let (_, at) = fuzzy("vim-keys.md", "vk").expect("a match");
    assert_eq!(at, vec![0, 4], "the v of vim and the k of keys");
}

#[test]
fn the_notes_that_answer_best_are_offered_first() {
    use notes::finder::{fuzzy, search, On};

    // A run of characters together beats the same characters scattered.
    let together = fuzzy("ideas.md", "idea").expect("a match").0;
    let scattered = fuzzy("i do enjoy a break", "idea").expect("a match").0;
    assert!(
        together > scattered,
        "together {together} should beat scattered {scattered}"
    );

    // And a word beginning beats the middle of one.
    let start = fuzzy("vim keys", "key").expect("a match").0;
    let middle = fuzzy("monkeys", "key").expect("a match").0;
    assert!(start > middle, "start {start} should beat middle {middle}");

    let lib = library();
    let hits = search(&lib, "mark");
    assert_eq!(hits[0].note, 1, "the showcase is what 'mark' means here");

    // Every note in a library of markdown ends in `.md`, so the extension is
    // not part of the name: a query of "m" is about the names, not the filing.
    let hits = search(&lib, "m");
    assert!(
        lib[hits[0].note].title.to_lowercase().starts_with('m'),
        "a note whose name starts with it, not the first one whose extension \
         happens to contain it - got {:?}",
        lib[hits[0].note].title
    );
    assert!(
        !hits.iter().any(|h| lib[h.note].title == "Ideas"),
        "ideas.md answers 'm' only through its extension"
    );

    // Either name can be typed at, and the one that answered is the one lit.
    let hits = search(&lib, "vim-k");
    assert_eq!(hits[0].note, 2);
    assert_eq!(hits[0].on, On::File, "that is a filename, hyphen and all");

    // No query is the whole library, in the order it came.
    let all = search(&lib, "");
    assert_eq!(all.len(), lib.len());
    assert_eq!(all[0].note, 0);

    // And a query nothing answers is an empty list rather than a bad guess.
    assert!(search(&lib, "zzz").is_empty());
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
fn a_panel_is_not_painted_until_it_knows_how_tall_it_is() {
    use notes::panels::{settings, Chrome};
    use pixui::{Canvas, Input, Ui, UiState};

    let theme = pixui::theme::scheme_named("GRUVBOX DARK").expect("a scheme by that name");
    let mut canvas = Canvas::new(600, 400);
    let mut state = UiState::new();
    let input = Input {
        mouse_in_window: true,
        dt: 1.0 / 60.0,
        ..Default::default()
    };
    let mut config = Settings::default();
    let mut chrome = Chrome::default();

    let blank = pixui::Color::hex(0x123456);
    let mut draw = |canvas: &mut Canvas, chrome: Option<&mut Chrome>| {
        canvas.clear(blank);
        let mut ui = Ui::begin(canvas, &input, &theme, &mut state);
        if let Some(chrome) = chrome {
            settings(&mut ui, &mut config, chrome);
        }
        let out = ui.finish();
        (out.animating, canvas.pixels().to_vec())
    };

    // A frame with no panel in it at all, for the two below to be measured
    // against: the toolkit's own post-frame passes paint every row, so "blank"
    // means "the same as a frame that drew nothing", not "untouched".
    let (_, nothing) = draw(&mut canvas, None);

    // The first frame with the panel has no height it can trust, so it lays
    // itself out and paints none of it.
    let (animating, first) = draw(&mut canvas, Some(&mut chrome));
    // Counted rather than compared whole: two screenfuls of pixels printed on
    // failure tell you nothing that the count does not.
    let differing = |a: &[u32], b: &[u32]| a.iter().zip(b).filter(|(x, y)| x != y).count();
    assert_eq!(
        differing(&first, &nothing),
        0,
        "a panel drawn at a height measured for some other page is the flicker"
    );
    assert!(
        animating,
        "and it has to ask for the frame that does paint it"
    );
    let measured = chrome.panel_h;
    assert!(measured > 72, "it measured itself while it was invisible");

    // The next one paints, at the height the first one worked out.
    let (_, second) = draw(&mut canvas, Some(&mut chrome));
    assert!(
        differing(&second, &nothing) > 1000,
        "the second frame is the one you see"
    );
    assert_eq!(chrome.panel_h, measured, "and it did not move on arrival");
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
fn the_status_line_says_what_the_model_is_doing() {
    use notes::assist::{Assist, Phase};
    use notes::llm::Progress;
    use notes::text::Cursor;
    let mut block = Assist::new(Cursor::new(0, 0), Cursor::new(0, 8), "the text".into());
    block.phase = Phase::Thinking;

    // Before a word has come back, what there is to report is the question,
    // and how much of it has been read. Reading a long one is the slowest part
    // of answering it: a line that sat unchanged through eight seconds of it
    // was indistinguishable from one that had stopped being drawn.
    block.progress = Progress {
        prompt: 412,
        read: 256,
        ..Progress::default()
    };
    assert_eq!(block.headline(), "READING 256/412 TOKENS");

    // And once it is all in, the two agree.
    block.progress.read = 412;
    assert_eq!(block.headline(), "READING 412/412 TOKENS");

    // A reasoning model is thinking, and says so rather than looking slow.
    block.progress = Progress {
        prompt: 412,
        written: 90,
        elapsed: std::time::Duration::from_secs(4),
        generating: std::time::Duration::from_secs(3),
        deliberating: true,
        ..Progress::default()
    };
    assert_eq!(block.headline(), "THINKING - 90 TOKENS AT 30/S");

    block.progress.deliberating = false;
    assert_eq!(block.headline(), "WRITING - 90 TOKENS AT 30/S");
}

#[test]
fn a_question_in_flight_reports_where_it_has_got_to() {
    use notes::llm::{Ask, Backend, Progress};
    /// A watcher that keeps everything it is told, so a test can look.
    struct Noting {
        seen: Vec<Progress>,
    }
    impl notes::llm::Watcher for Noting {
        fn tick(&mut self, at: Progress, _said: &str) {
            self.seen.push(at);
        }
        fn carry_on(&self) -> bool {
            true
        }
    }

    let mut stub = notes::llm::Rehearsal;
    let mut watch = Noting { seen: Vec::new() };
    let _ = stub.edit(
        &Ask {
            source: "teh quick fox".into(),
            request: "fix it".into(),
            ..Default::default()
        },
        &mut watch,
    );
    let seen = watch.seen;
    assert_eq!(
        seen.len(),
        1,
        "the stub answers at once, so it reports once"
    );
    assert_eq!(seen[0].prompt, 3);
    // Nothing written in no time is not an infinite rate.
    assert_eq!(seen[0].rate(), 0.0);

    // The rate is over the writing, not over the wait: three seconds of that
    // wait were the weights being read off disk.
    let along = Progress {
        prompt: 400,
        written: 60,
        elapsed: std::time::Duration::from_secs(6),
        generating: std::time::Duration::from_secs(3),
        deliberating: false,
        ..Progress::default()
    };
    assert_eq!(along.rate(), 20.0);
}

#[test]
fn the_rehearsal_backend_always_leaves_something_to_review() {
    use notes::llm::{Ask, Backend};
    let mut stub = notes::llm::Rehearsal;
    let reply = stub
        .edit(
            &Ask {
                source: "nothing to fix here".into(),
                request: "Improve It".into(),
                ..Default::default()
            },
            &mut notes::llm::Quiet,
        )
        .unwrap();
    assert!(reply.ends_with("(improve it)"), "got {reply:?}");
}

// -------------------------------------------------------------- auto-indent

use notes::indent::{self, Opened};

fn lines_of(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

/// What Enter would open, pressed at the end of the last line.
fn open_end(text: &str) -> Opened {
    let lines = lines_of(text);
    let at = lines.len() - 1;
    indent::opened(&lines, at, lines[at].chars().count())
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

fn located(text: &str) -> Vec<(usize, notes::markdown::Block)> {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    notes::markdown::parse_located(&lines)
}

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
fn parse_and_parse_located_agree_about_the_document() {
    let text = notes::showcase::SHOWCASE;
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let plain = notes::markdown::parse(&lines);
    let numbered = notes::markdown::parse_located(&lines);
    assert_eq!(plain.len(), numbered.len());
    // Numbers only ever move forward: a gutter that counted backwards would be
    // a layout that had lost track of the document.
    let mut last = 0;
    for (n, _) in &numbered {
        assert!(*n >= last, "block at {n} follows one at {last}");
        last = *n;
    }
    assert!(numbered.last().unwrap().0 < lines.len());
}

#[test]
fn list_items_carry_their_own_lines() {
    // A list is one block but many rows, and each row is a line somebody
    // typed — so the gutter numbers them one by one rather than numbering the
    // list once and leaving the rest blank.
    let blocks = located("intro\n\n- one\n- two\n  still two\n\n- four");
    let notes::markdown::Block::List(items) = &blocks[1].1 else {
        panic!("expected a list, got {:?}", blocks[1].1);
    };
    assert_eq!(
        items.iter().map(|i| i.line).collect::<Vec<_>>(),
        vec![2, 3, 6]
    );
}

#[test]
fn a_fenced_block_numbers_from_inside_its_fence() {
    // The fence is not drawn, so the first row of the slab is the line after
    // it. Numbering the slab from the fence would put every line one out.
    let blocks = located("intro\n\n```rust\nlet x = 1;\nlet y = 2;\n```");
    match &blocks[1].1 {
        notes::markdown::Block::Code { first, lines, .. } => {
            assert_eq!(*first, 3);
            assert_eq!(lines.len(), 2);
        }
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn an_indented_block_numbers_from_its_first_line() {
    // There is no fence to skip past: the block starts where the code does.
    let blocks = located("intro\n\n    fn main() {}\n    // two");
    match &blocks[1].1 {
        notes::markdown::Block::Code { first, .. } => assert_eq!(*first, 2),
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn blocks_inside_a_quote_know_their_lines_too() {
    let blocks = located("> # Quoted\n>\n> - a bullet\n> - another");
    let notes::markdown::Block::Quote(inner) = &blocks[0].1 else {
        panic!("expected a quote");
    };
    assert_eq!(inner[0].0, 0, "the heading is on the first line");
    assert_eq!(inner[1].0, 2, "the list starts on the third");
    let notes::markdown::Block::List(items) = &inner[1].1 else {
        panic!("expected a list");
    };
    assert_eq!(
        items.iter().map(|i| i.line).collect::<Vec<_>>(),
        vec![2, 3],
        "and its items are numbered in the document's terms, not the quote's"
    );
}

// ----------------------------------------------------------- document parser

use notes::markdown::{Block, CellAlign, Marker};

fn doc(text: &str) -> Vec<Block> {
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    notes::markdown::parse(&lines)
}

/// The plain text of a run of spans, with the markup already removed.
fn flat(spans: &[notes::markdown::Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn headings_carry_their_level() {
    let blocks = doc("# One\n\n### Three");
    match (&blocks[0], &blocks[1]) {
        (
            Block::Heading {
                level: a,
                spans: s1,
            },
            Block::Heading {
                level: b,
                spans: s2,
            },
        ) => {
            assert_eq!((*a, *b), (1, 3));
            assert_eq!(flat(s1), "One");
            assert_eq!(flat(s2), "Three");
        }
        other => panic!("expected two headings, got {other:?}"),
    }
}

#[test]
fn consecutive_lines_are_one_paragraph() {
    // A hard wrap in the source is not a line break in the output; that is the
    // whole difference between highlighting lines and rendering a document.
    let blocks = doc("one line\nand another\n\nseparate");
    assert_eq!(blocks.len(), 2);
    match &blocks[0] {
        Block::Paragraph(spans) => assert_eq!(flat(spans), "one line and another"),
        other => panic!("expected a paragraph, got {other:?}"),
    }
}

#[test]
fn rendered_spans_drop_the_markup_but_keep_the_emphasis() {
    let blocks = doc("a **bold** and `code` and [link](x.md)");
    let Block::Paragraph(spans) = &blocks[0] else {
        panic!()
    };
    assert_eq!(
        flat(spans),
        "a bold and code and link",
        "the asterisks, backticks and target are instructions, not text"
    );
    assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
}

#[test]
fn lists_capture_their_markers_and_depth() {
    let blocks = doc("- one\n- two\n  - nested\n3. third");
    let Block::List(items) = &blocks[0] else {
        panic!("expected a list")
    };
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].marker, Marker::Bullet);
    assert_eq!(items[0].depth, 0);
    assert_eq!(items[2].depth, 1, "two spaces of indent is one level");
    assert_eq!(items[3].marker, Marker::Number(3));
    assert_eq!(flat(&items[2].spans), "nested");
}

#[test]
fn task_items_are_recognised_either_way() {
    let blocks = doc("- [ ] todo\n- [x] done");
    let Block::List(items) = &blocks[0] else {
        panic!()
    };
    assert_eq!(items[0].marker, Marker::Task(false));
    assert_eq!(items[1].marker, Marker::Task(true));
    assert_eq!(flat(&items[0].spans), "todo");
}

#[test]
fn a_fence_keeps_its_language_and_its_lines_verbatim() {
    let blocks = doc("```rust\nfn main() {}\n    indented\n```");
    match &blocks[0] {
        Block::Code { lang, lines, .. } => {
            assert_eq!(lang, "rust");
            assert_eq!(lines, &["fn main() {}", "    indented"]);
        }
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn markup_inside_a_fence_is_left_alone() {
    let blocks = doc("```\n# not a heading\n- not a list\n```");
    let Block::Code { lines, .. } = &blocks[0] else {
        panic!()
    };
    assert_eq!(lines, &["# not a heading", "- not a list"]);
}

#[test]
fn quotes_gather_their_consecutive_lines() {
    let blocks = doc("> first\n> second\n\nafter");
    match &blocks[0] {
        // One paragraph, not two lines: a soft wrap inside a quote is a soft
        // wrap like any other.
        Block::Quote(inner) => match &inner[..] {
            [(_, Block::Paragraph(spans))] => assert_eq!(flat(spans), "first second"),
            other => panic!("expected one paragraph, got {other:?}"),
        },
        other => panic!("expected a quote, got {other:?}"),
    }
}

#[test]
fn a_table_needs_its_alignment_row() {
    let blocks = doc("| key | moves |\n| --- | ----: |\n| h | left |\n| j | down |");
    match &blocks[0] {
        Block::Table {
            align,
            header,
            rows,
        } => {
            assert_eq!(align, &[CellAlign::Left, CellAlign::Right]);
            assert_eq!(flat(&header[0]), "key");
            assert_eq!(rows.len(), 2);
            assert_eq!(flat(&rows[1][1]), "down");
        }
        other => panic!("expected a table, got {other:?}"),
    }
}

#[test]
fn pipes_without_an_alignment_row_are_just_prose() {
    let blocks = doc("this | that | the other");
    assert!(
        matches!(blocks[0], Block::Paragraph(_)),
        "a sentence containing pipes is not a table"
    );
}

#[test]
fn centre_alignment_is_read_from_the_colons() {
    let blocks = doc("| a |\n| :-: |\n| x |");
    let Block::Table { align, .. } = &blocks[0] else {
        panic!()
    };
    assert_eq!(align, &[CellAlign::Center]);
}

#[test]
fn rules_are_recognised_and_are_not_headings() {
    let blocks = doc("above\n\n---\n\nbelow");
    assert!(matches!(blocks[1], Block::Rule));
    assert_eq!(blocks.len(), 3);
}

#[test]
fn an_empty_document_parses_to_nothing() {
    assert!(doc("").is_empty());
    assert!(doc("\n\n   \n").is_empty());
}

#[test]
fn a_table_sizes_to_its_content_until_it_cannot() {
    let cell = |s: &str| {
        vec![notes::markdown::Span {
            text: s.into(),
            tok: notes::markdown::Tok::Text,
            bold: false,
            href: None,
        }]
    };
    let header = vec![cell("ab"), cell("cd")];
    let rows: Vec<Vec<_>> = vec![];

    let roomy = notes::render::column_widths(&header, &rows, 4000);
    assert!(
        roomy.iter().sum::<i32>() < 200,
        "a small table should not stretch to fill"
    );

    let cramped = notes::render::column_widths(&header, &rows, 40);
    assert!(
        cramped.iter().sum::<i32>() <= 40,
        "and should shrink when it must"
    );
}

#[test]
fn a_link_carries_its_target_into_the_rendering() {
    let spans = notes::markdown::inline_spans("see [the readme](../README.md) for more");
    let link = spans
        .iter()
        .find(|s| s.tok == notes::markdown::Tok::Link)
        .expect("the label should survive as a link");
    assert_eq!(link.text, "the readme");
    assert_eq!(
        link.href.as_deref(),
        Some("../README.md"),
        "the target is dropped from the text but has to reach the renderer"
    );
    let rendered: String = spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        rendered, "see the readme for more",
        "the target itself is not prose and is not drawn"
    );
}

#[test]
fn brackets_without_a_target_are_not_links() {
    let spans = notes::markdown::inline_spans("a [note] in brackets");
    assert!(
        spans.iter().all(|s| s.href.is_none()),
        "nothing to follow, so nothing to click"
    );
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

/// The spans of the one paragraph `text` parses to.
fn para(text: &str) -> Vec<notes::markdown::Span> {
    match doc(text).into_iter().next() {
        Some(Block::Paragraph(spans)) => spans,
        other => panic!("expected a paragraph, got {other:?}"),
    }
}

/// The token each character of a rendered paragraph carries, as one string per
/// span, so a test can say what was emphasised without counting spans.
fn toks(spans: &[notes::markdown::Span]) -> Vec<(String, notes::markdown::Tok, bool)> {
    spans
        .iter()
        .map(|s| (s.text.clone(), s.tok, s.bold))
        .collect()
}

#[test]
fn a_backslash_escapes_the_character_after_it() {
    use notes::markdown::Tok;
    let spans = para(r"\*not italic\* and \_not either\_");
    assert_eq!(flat(&spans), "*not italic* and _not either_");
    assert!(
        spans.iter().all(|s| s.tok == Tok::Text),
        "an escaped delimiter is text, not markup"
    );
}

#[test]
fn a_backslash_before_anything_else_is_a_backslash() {
    assert_eq!(flat(&para(r"C:\path\to\file")), r"C:\path\to\file");
}

#[test]
fn underscores_emphasise_like_asterisks() {
    use notes::markdown::Tok;
    assert_eq!(
        toks(&para("_slanted_")),
        [("slanted".into(), Tok::Italic, false)]
    );
    assert_eq!(
        toks(&para("__heavy__")),
        [("heavy".into(), Tok::Bold, true)]
    );
}

#[test]
fn underscores_inside_a_word_are_left_alone() {
    use notes::markdown::Tok;
    let spans = para("call snake_case_name here");
    assert_eq!(flat(&spans), "call snake_case_name here");
    assert!(
        spans.iter().all(|s| s.tok == Tok::Text),
        "an identifier is not three words in italics"
    );
}

#[test]
fn a_lone_asterisk_between_spaces_is_multiplication() {
    use notes::markdown::Tok;
    let spans = para("2 * 3 * 4");
    assert!(
        spans.iter().all(|s| s.tok == Tok::Text),
        "a delimiter run has to be followed by something to emphasise"
    );
}

#[test]
fn emphasis_nests_rather_than_replacing() {
    use notes::markdown::Tok;
    assert_eq!(
        toks(&para("**bold with *italic* inside**")),
        [
            ("bold with ".into(), Tok::Bold, true),
            ("italic".into(), Tok::Bold, true),
            (" inside".into(), Tok::Bold, true),
        ],
        "the inner run is both, not whichever was parsed last"
    );
}

#[test]
fn a_code_span_may_hold_backticks_if_it_is_fenced_by_more() {
    use notes::markdown::Tok;
    let spans = para("``a ` b``");
    assert_eq!(toks(&spans), [("a ` b".into(), Tok::Code, false)]);
}

#[test]
fn a_code_span_drops_one_space_of_padding() {
    assert_eq!(flat(&para("`` ` ``")), "`");
}

#[test]
fn markup_inside_a_code_span_is_literal() {
    use notes::markdown::Tok;
    let spans = para("`**not bold**`");
    assert_eq!(toks(&spans), [("**not bold**".into(), Tok::Code, false)]);
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
fn a_collapsed_reference_reuses_its_own_label() {
    let blocks = doc("see [pixui][]\n\n[pixui]: https://example.com/p");
    let Block::Paragraph(spans) = &blocks[0] else {
        panic!("expected a paragraph")
    };
    let link = spans.iter().find(|s| s.href.is_some()).expect("a link");
    assert_eq!(link.text, "pixui");
    assert_eq!(link.href.as_deref(), Some("https://example.com/p"));
}

#[test]
fn a_shortcut_reference_is_just_the_label() {
    let blocks = doc("see [pixui]\n\n[pixui]: https://example.com/p");
    let Block::Paragraph(spans) = &blocks[0] else {
        panic!("expected a paragraph")
    };
    assert_eq!(
        spans
            .iter()
            .find(|s| s.href.is_some())
            .unwrap()
            .href
            .as_deref(),
        Some("https://example.com/p")
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
fn a_row_of_equals_makes_the_line_above_a_heading() {
    match &doc("A title\n=======\n\nbody")[0] {
        Block::Heading { level, spans } => {
            assert_eq!(*level, 1);
            assert_eq!(flat(spans), "A title");
        }
        other => panic!("expected a heading, got {other:?}"),
    }
}

#[test]
fn a_row_of_dashes_under_text_is_a_heading_not_a_rule() {
    match &doc("A title\n---\n\nbody")[0] {
        Block::Heading { level, .. } => assert_eq!(*level, 2),
        other => panic!("expected a heading, got {other:?}"),
    }
}

#[test]
fn a_row_of_dashes_on_its_own_is_still_a_rule() {
    assert!(matches!(doc("text\n\n---\n\nmore")[1], Block::Rule));
}

#[test]
fn four_spaces_of_indent_is_a_code_block() {
    match &doc("text\n\n    fn main() {}\n    // and this\n\nafter")[1] {
        Block::Code { lines, lang, .. } => {
            assert!(lang.is_empty(), "an indented block names no language");
            assert_eq!(lines, &["fn main() {}", "// and this"]);
        }
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn a_tilde_fence_can_hold_a_backtick_fence() {
    match &doc("~~~\n```\nnested\n```\n~~~")[0] {
        Block::Code { lines, .. } => assert_eq!(lines, &["```", "nested", "```"]),
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn a_fence_is_closed_only_by_its_own_delimiter() {
    match &doc("````\n```\nstill inside\n```\n````")[0] {
        Block::Code { lines, .. } => assert_eq!(lines.len(), 3),
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn a_heading_may_be_closed_with_hashes() {
    match &doc("## Middle ##")[0] {
        Block::Heading { level, spans } => {
            assert_eq!(*level, 2);
            assert_eq!(flat(spans), "Middle");
        }
        other => panic!("expected a heading, got {other:?}"),
    }
}

#[test]
fn a_list_item_keeps_the_lines_that_continue_it() {
    match &doc("- a first item that\n  runs onto a second line\n- and a second")[0] {
        Block::List(items) => {
            assert_eq!(items.len(), 2, "two items, not three");
            assert_eq!(
                flat(&items[0].spans),
                "a first item that runs onto a second line"
            );
        }
        other => panic!("expected a list, got {other:?}"),
    }
}

#[test]
fn a_blank_line_between_items_does_not_split_the_list() {
    match &doc("- one\n\n- two\n\n- three")[0] {
        Block::List(items) => assert_eq!(items.len(), 3),
        other => panic!("expected one list, got {other:?}"),
    }
}

#[test]
fn a_quote_can_hold_anything_a_document_can() {
    match &doc("> # Heading\n>\n> - a bullet\n> - another")[0] {
        Block::Quote(inner) => {
            assert!(matches!(inner[0].1, Block::Heading { level: 1, .. }));
            match &inner[1].1 {
                Block::List(items) => assert_eq!(items.len(), 2),
                other => panic!("expected a list inside the quote, got {other:?}"),
            }
        }
        other => panic!("expected a quote, got {other:?}"),
    }
}

#[test]
fn a_quote_runs_on_without_repeating_its_marker() {
    match &doc("> first line\nsecond line\n\nafter")[0] {
        Block::Quote(inner) => match &inner[..] {
            [(_, Block::Paragraph(spans))] => {
                assert_eq!(flat(spans), "first line second line")
            }
            other => panic!("expected one paragraph, got {other:?}"),
        },
        other => panic!("expected a quote, got {other:?}"),
    }
}

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
fn a_hard_break_ends_a_wrapped_row() {
    let rows = notes::markdown::wrap_ranges("ab\ncd", 40);
    assert_eq!(
        rows,
        vec![(0, 2), (3, 5)],
        "and the newline itself is drawn by nobody"
    );
}

#[test]
fn a_soft_wrap_is_not_a_line_break() {
    assert_eq!(
        flat(&para("first line\nsecond line")),
        "first line second line"
    );
}

#[test]
fn three_delimiters_are_bold_and_italic_at_once() {
    use notes::markdown::Tok;
    let spans = para("***both***");
    assert_eq!(flat(&spans), "both", "and nothing left over from the run");
    assert_eq!(toks(&spans), [("both".into(), Tok::Bold, true)]);
}

#[test]
fn a_title_may_be_underlined_rather_than_hashed() {
    let lines: Vec<String> = "Underlined\n==========\n\n# Later"
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(notes::markdown::derive_title(&lines, 24), "Underlined");
}

// ------------------------------------------------------- the reference note
//
// The showcase is the document a reader opens to see what the renderer does.
// These tests make it the parser's fixture as well, so a construct cannot be
// claimed there without being parsed here.

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

#[test]
fn the_showcase_uses_every_kind_of_block() {
    let blocks = showcase();
    let has = |f: &dyn Fn(&Block) -> bool| blocks.iter().any(f);
    assert!(has(&|b| matches!(b, Block::Heading { .. })), "headings");
    assert!(has(&|b| matches!(b, Block::Paragraph(_))), "paragraphs");
    assert!(has(&|b| matches!(b, Block::List(_))), "lists");
    assert!(has(&|b| matches!(b, Block::Quote(_))), "quotes");
    assert!(has(&|b| matches!(b, Block::Code { .. })), "code");
    assert!(has(&|b| matches!(b, Block::Table { .. })), "tables");
    assert!(has(&|b| matches!(b, Block::Rule)), "rules");
}

#[test]
fn the_showcase_reaches_every_heading_level() {
    let mut seen: Vec<u8> = showcase()
        .iter()
        .filter_map(|b| match b {
            Block::Heading { level, .. } => Some(*level),
            _ => None,
        })
        .collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen, [1, 2, 3, 4, 5, 6], "all six levels are demonstrated");
}

#[test]
fn the_showcase_uses_every_kind_of_list_marker() {
    use notes::markdown::Marker;
    let markers: Vec<Marker> = showcase()
        .iter()
        .filter_map(|b| match b {
            Block::List(items) => Some(items.clone()),
            _ => None,
        })
        .flatten()
        .map(|i| i.marker)
        .collect();
    assert!(markers.contains(&Marker::Bullet), "bullets");
    assert!(
        markers.iter().any(|m| matches!(m, Marker::Number(_))),
        "ordered"
    );
    assert!(markers.contains(&Marker::Task(false)), "an unchecked task");
    assert!(markers.contains(&Marker::Task(true)), "a checked one");
    assert!(
        markers.iter().any(|m| matches!(m, Marker::Number(7))),
        "a number that is not its position in the list"
    );
}

#[test]
fn the_showcase_uses_every_kind_of_emphasis() {
    use notes::markdown::Tok;
    let toks: Vec<Tok> = all_spans(&showcase()).iter().map(|s| s.tok).collect();
    for want in [
        Tok::Bold,
        Tok::Italic,
        Tok::Code,
        Tok::Strike,
        Tok::Link,
        Tok::Image,
    ] {
        assert!(
            toks.contains(&want),
            "the showcase never demonstrates {want:?}"
        );
    }
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
fn the_showcase_nests_a_quote_inside_a_quote() {
    fn depth(blocks: &[Block]) -> usize {
        blocks
            .iter()
            .map(|b| match b {
                Block::Quote(inner) => {
                    1 + inner
                        .iter()
                        .map(|(_, b)| depth(std::slice::from_ref(b)))
                        .max()
                        .unwrap_or(0)
                }
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }
    assert!(depth(&showcase()) >= 2, "a quote inside a quote");
}

#[test]
fn the_showcase_puts_other_blocks_inside_a_quote() {
    let quoted: Vec<Block> = showcase()
        .into_iter()
        .filter_map(|b| match b {
            Block::Quote(inner) => Some(inner),
            _ => None,
        })
        .flatten()
        .map(|(_, b)| b)
        .collect();
    assert!(
        quoted.iter().any(|b| matches!(b, Block::Heading { .. })),
        "a heading"
    );
    assert!(quoted.iter().any(|b| matches!(b, Block::List(_))), "a list");
    assert!(
        quoted.iter().any(|b| matches!(b, Block::Code { .. })),
        "code"
    );
}

#[test]
fn the_showcase_has_a_code_block_of_each_kind() {
    let code: Vec<(String, Vec<String>)> = showcase()
        .into_iter()
        .filter_map(|b| match b {
            Block::Code { lang, lines, .. } => Some((lang, lines)),
            _ => None,
        })
        .collect();
    assert!(
        code.iter().any(|(l, _)| l == "rust"),
        "a fence naming a language"
    );
    assert!(
        code.iter().any(|(l, _)| l.is_empty()),
        "one with no language"
    );
    assert!(
        code.iter()
            .any(|(_, lines)| lines.iter().any(|l| l.contains("```"))),
        "a fence holding a fence, which only a longer or different one can"
    );
}

#[test]
fn the_showcase_aligns_a_table_three_ways() {
    use notes::markdown::CellAlign;
    let align = showcase()
        .into_iter()
        .find_map(|b| match b {
            Block::Table { align, .. } => Some(align),
            _ => None,
        })
        .expect("a table");
    assert_eq!(
        align,
        [CellAlign::Left, CellAlign::Center, CellAlign::Right]
    );
}

#[test]
fn the_showcase_never_leaves_a_delimiter_in_the_prose() {
    // Rendered text is what was meant, not what was typed. Anything that
    // parsed correctly has had its markup consumed — so a stray delimiter in
    // the output is a construct this parser did not recognise.
    for block in showcase() {
        let (Block::Paragraph(spans) | Block::Heading { spans, .. }) = &block else {
            continue;
        };
        let text: String = spans
            .iter()
            .filter(|s| s.tok != notes::markdown::Tok::Code)
            .map(|s| s.text.as_str())
            .collect();
        // The escaped ones are deliberate: they are there to prove they survive.
        let text = text.replace("*not italic*", "").replace("_not italic_", "");
        assert!(
            !text.contains("**") && !text.contains("~~") && !text.contains("]("),
            "unconsumed markup in: {text}"
        );
    }
}

#[test]
fn the_reference_note_is_installed_into_a_vault_that_has_notes() {
    let dir = std::env::temp_dir().join(format!("pixui-ref-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A vault that is not empty, so the seeding path does not run.
    std::fs::write(dir.join("mine.md"), "# Mine\n").unwrap();

    let app = notes::Notes::open(dir.clone());
    assert!(
        app.notes
            .iter()
            .any(|n| n.filename() == "markdown-showcase.md"),
        "the reference note should be in the list beside the user's own"
    );

    // And never overwritten once it is there.
    std::fs::write(dir.join("markdown-showcase.md"), "# Edited\n").unwrap();
    let again = notes::Notes::open(dir.clone());
    let note = again
        .notes
        .iter()
        .find(|n| n.filename() == "markdown-showcase.md")
        .unwrap();
    assert_eq!(note.buffer.to_text().trim(), "# Edited");

    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------ settings

use notes::settings::{Settings, CATALOGUE};

#[test]
fn settings_survive_a_round_trip_through_a_file() {
    let config = Settings {
        scheme: "NORD".into(),
        font: "COZETTE".into(),
        assist: false,
        web: true,
        model: Some("Ornith-1.5-9B-Q4_K_M.gguf".into()),
        // A prompt has newlines in it, and the format is one line per setting.
        prompt: "first line\nsecond line\nand a backslash \\ too".into(),
    };
    let text = config.to_text();
    assert_eq!(text.lines().count(), 6, "one line per setting");
    assert_eq!(Settings::parse(&text), config);
}

#[test]
fn the_default_face_is_one_the_toolkit_has() {
    let name = Settings::default().font;
    assert!(
        pixui::font::face_named(&name).is_some(),
        "the app defaults to {name}, which the toolkit does not have"
    );
}

#[test]
fn the_default_scheme_is_one_the_toolkit_has() {
    let name = Settings::default().scheme;
    assert!(
        pixui::scheme_named(&name).is_some(),
        "the app defaults to {name}, which the toolkit does not ship"
    );
}

#[test]
fn the_assistant_switch_defaults_to_on_and_survives_being_turned_off() {
    assert!(
        Settings::default().assist,
        "a feature nobody sees is not found"
    );
    let off = Settings::parse("assist = off\n");
    assert!(!off.assist);
    assert!(
        !Settings::parse(&off.to_text()).assist,
        "and a round trip keeps it off"
    );
    assert!(Settings::parse("assist = on\n").assist);
}

#[test]
fn an_unreadable_settings_file_gives_the_defaults() {
    let config = Settings::parse("nonsense\n\nmodel=\nunknown = 3\n");
    assert_eq!(config, Settings::default());
    assert!(config.prompt.contains("markdown"), "the default prompt");
}

#[test]
fn a_newer_settings_file_does_not_lose_what_it_understands() {
    // A key from a build that knows more must not stop the rest being read.
    let config = Settings::parse("model = a.gguf\nfuture-thing = 12\n");
    assert_eq!(config.model.as_deref(), Some("a.gguf"));
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
fn a_download_puts_the_file_in_place_only_once_it_is_whole() {
    // Driven over `file://`, so the state machine is tested without a network:
    // curl is the same program either way, and the part being checked here is
    // the partial-then-rename, not the transfer.
    let dir = std::env::temp_dir().join(format!("pixui-fetch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.bin");
    std::fs::write(&source, vec![7u8; 4096]).unwrap();

    let weights = notes::settings::Weights {
        label: "TEST",
        file: "fetched.gguf",
        url: Box::leak(format!("file://{}", source.display()).into_boxed_str()),
        megabytes: 1,
        note: "",
    };
    let into = dir.join("models");
    let mut down = notes::fetch::Download::start(&weights, &into).expect("curl should start");
    let outcome = loop {
        if let Some(outcome) = down.poll() {
            break outcome;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let path = outcome.expect("the copy should succeed");
    assert_eq!(path, into.join("fetched.gguf"));
    assert_eq!(std::fs::read(&path).unwrap().len(), 4096);
    assert!(
        !into.join("fetched.gguf.part").exists(),
        "the partial file is renamed, not left behind"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_prompt_is_edited_by_the_same_grammar_as_a_note() {
    // Not "a text field that also does vim": the same Buffer and the same Vim
    // the notes are edited with, so anything that works there works here.
    let mut editor = notes::panels::PromptEditor::new("alpha beta gamma");
    press(&mut editor.vim, &mut editor.buf, "dw");
    assert_eq!(editor.text(), "beta gamma");
    press(&mut editor.vim, &mut editor.buf, "A!\x1b");
    assert_eq!(editor.text(), "beta gamma!");
    press(&mut editor.vim, &mut editor.buf, "u");
    assert_eq!(editor.text(), "beta gamma", "and undo is vim's undo");
}

// ----------------------------------------------------------------- clipboard

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
fn a_setting_that_no_longer_exists_is_read_past() {
    // The context ceiling was a setting until it became the model's own number.
    // A file written by the build that had it must still be read by this one.
    let config = Settings::parse("scheme = NORD\ncontext = 32768\nassist = off\n");
    assert_eq!(config.scheme, "NORD");
    assert!(!config.assist, "the settings after it are not lost with it");
    assert!(
        !config.to_text().contains("context"),
        "and it is not written back out"
    );
}

// -------------------------------------------------------------------- digest

use notes::digest;

fn vault_of(notes: &[(&str, &str)]) -> Vec<notes::Note> {
    notes
        .iter()
        .map(|(file, text)| notes::Note {
            path: Some(std::path::PathBuf::from(file)),
            buffer: Buffer::from_text(text),
            project: String::new(),
            seen: None,
        })
        .collect()
}

#[test]
fn every_note_gets_a_line_saying_what_it_is() {
    let vault = vault_of(&[
        (
            "welcome.md",
            "# Welcome\n\nThis is the editor.\n\n## Try it\n\nPress i.",
        ),
        ("ideas.md", "# Ideas\n\n- [ ] An export to HTML"),
    ]);
    let text = digest::vault(&vault);
    assert_eq!(
        text.lines().count(),
        2,
        "one line per note, whatever is in it"
    );
    assert!(
        text.contains("`ideas.md`"),
        "the file, so the model can name it"
    );
    assert!(
        text.contains("\"Welcome\""),
        "and what the note calls itself, in its own case rather than the sidebar's shout"
    );
    assert!(
        text.contains("This is the editor."),
        "and the first thing it says"
    );
    assert!(text.contains("sections: Try it"), "and the shape of it");
}

#[test]
fn the_digest_is_the_same_whichever_note_is_open() {
    // The whole point of the ordering: it is a stable prefix, so it can one day
    // be read once and kept rather than re-read on every question.
    let a = vault_of(&[("b.md", "# Bee\n\nBuzz."), ("a.md", "# Ay\n\nAlpha.")]);
    let b = vault_of(&[("a.md", "# Ay\n\nAlpha."), ("b.md", "# Bee\n\nBuzz.")]);
    assert_eq!(digest::vault(&a), digest::vault(&b));
    assert!(
        digest::vault(&a).starts_with("- `a.md`"),
        "in filename order, not the order the directory was read in"
    );
}

#[test]
fn a_notes_first_line_is_prose_rather_than_punctuation() {
    let vault = vault_of(&[(
        "showcase.md",
        "Markdown showcase\n=================\n\n```rust\nfn main() {}\n```\n\nThe real first line.",
    )]);
    let text = digest::vault(&vault);
    assert!(
        text.contains("The real first line."),
        "the underline and the code block are not what the note says: {text}"
    );
    assert!(!text.contains("fn main"), "code is not a gist");
}

#[test]
fn the_passage_is_marked_where_it_sits_in_the_note() {
    let whole = "# Heading\n\nBefore it.\n\nThe passage.\n\nAfter it.\n";
    let at = whole.find("The passage.").unwrap();
    let marked =
        digest::marked(whole, at, at + "The passage.".len()).expect("there is a note around it");
    assert!(
        marked.contains("# Heading"),
        "the model sees what it is under"
    );
    assert!(marked.contains("After it."), "and what comes next");
    assert_eq!(
        marked,
        format!(
            "# Heading\n\nBefore it.\n\n{}The passage.{}\n\nAfter it.\n",
            digest::OPEN,
            digest::CLOSE
        )
    );
}

#[test]
fn a_passage_that_is_the_whole_note_is_not_sent_twice() {
    let whole = "All of it.";
    assert!(
        digest::marked(whole, 0, whole.len()).is_none(),
        "there is nothing around it, and a second copy only invites a wrong answer"
    );
}

#[test]
fn the_note_is_marked_at_the_cursors_the_selection_gave() {
    // The editor has a selection as two cursors; the digest works in byte
    // offsets. This is the conversion, and it is the one part of the path the
    // command line never exercises.
    let buf = Buffer::from_text("# Title\n\nfirst line\nsecond line\nthird line\n");
    let marked = digest::around(&buf, Cursor::new(2, 6), Cursor::new(3, 6))
        .expect("there is a note around it");
    assert_eq!(
        marked,
        format!(
            "# Title\n\nfirst {}line\nsecond{} line\nthird line\n",
            digest::OPEN,
            digest::CLOSE
        ),
        "the markers land on the columns the selection ended on"
    );
}

// ---------------------------------------------------------------------- chat

use notes::chat;
use notes::llm::Turn;

#[test]
fn a_conversation_survives_being_written_down_and_read_back() {
    let mut talk = chat::Chat::new("reading".into(), "welcome.md".into());
    talk.turns = vec![
        Turn {
            mine: true,
            text: "what does this note say".into(),
        },
        Turn {
            mine: false,
            text: "That it is a markdown editor.\n\n- with vim keys\n- drawn by hand".into(),
        },
        Turn {
            mine: true,
            text: "and the toolkit".into(),
        },
    ];
    let filed = talk.to_text();
    assert_eq!(
        chat::parse(&filed),
        talk.turns,
        "every turn, in order, on both sides"
    );
    assert!(
        filed.starts_with("# what does this note say"),
        "titled by what was first asked"
    );
}

#[test]
fn a_marker_inside_a_code_fence_is_code() {
    // A conversation about this very format is the obvious thing to have, and
    // the obvious thing to break it.
    let filed = "# t\n\n## you\n\nhow are turns marked\n\n## assistant\n\nLike this:\n\n```\n## you\n## assistant\n```\n\nAt the top level only.\n";
    let turns = chat::parse(filed);
    assert_eq!(turns.len(), 2, "two turns, not four: {turns:?}");
    assert!(
        turns[1].text.contains("## you"),
        "the sample survives inside the answer"
    );
}

#[test]
fn an_answer_that_writes_a_marker_cannot_split_itself() {
    let mut talk = chat::Chat::new("work".into(), "n.md".into());
    talk.turns = vec![
        Turn {
            mine: true,
            text: "write the marker on its own line".into(),
        },
        Turn {
            mine: false,
            text: "Sure:\n\n## assistant\n\nThat is it.".into(),
        },
    ];
    let read = chat::parse(&talk.to_text());
    assert_eq!(
        read.len(),
        2,
        "still two turns after a round trip: {read:?}"
    );
    assert!(!read[1].mine, "and the second is still the model's");
}

#[test]
fn conversations_are_filed_where_the_vault_cannot_see_them() {
    let dir = std::env::temp_dir().join(format!("pixui-chat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("welcome.md"), "# Welcome\n\nHello.\n").unwrap();

    let mut talk = chat::Chat::new("reading".into(), "welcome.md".into());
    talk.turns = vec![Turn {
        mine: true,
        text: "anything at all".into(),
    }];
    talk.save(&dir).expect("it saves");

    let path = talk.path.clone().expect("named on the way out");
    assert!(
        path.starts_with(dir.join(".chats").join("reading")),
        "filed under the project, in a folder the forest does not count: {path:?}"
    );
    assert_eq!(
        notes::read_vault(&dir).len(),
        1,
        "the vault is still one note - a conversation is not one of them"
    );
    let listed = chat::filed(&dir, "reading");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].turns, 1);
    assert_eq!(listed[0].title, "anything at all");
    assert!(
        chat::filed(&dir, "allotment").is_empty(),
        "and they belong to one project"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_conversation_is_not_filed_at_all() {
    let dir = std::env::temp_dir().join(format!("pixui-chat-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut talk = chat::Chat::new("reading".into(), "welcome.md".into());
    talk.save(&dir).expect("saving nothing is not an error");
    assert!(
        talk.path.is_none(),
        "nothing was said, so there is nothing to keep"
    );
    assert!(!dir.join("chats").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_chat_can_be_given_a_name_that_sticks() {
    let dir = std::env::temp_dir().join(format!("pixui-rename-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut talk = chat::Chat::new("reading".into(), "welcome.md".into());
    talk.turns = vec![Turn {
        mine: true,
        text: "how does wrapping work".into(),
    }];
    assert_eq!(
        talk.title(),
        "how does wrapping work",
        "named by what was asked"
    );

    assert!(
        talk.command("/rename wrapping notes"),
        "a slash line is a command"
    );
    assert_eq!(talk.title(), "wrapping notes");
    talk.save(&dir).unwrap();

    let back = chat::Chat::open(
        talk.path.as_ref().unwrap(),
        "reading".into(),
        "welcome.md".into(),
    );
    assert_eq!(
        back.title(),
        "wrapping notes",
        "and it is still called that tomorrow"
    );
    assert_eq!(
        chat::filed(&dir, "reading")[0].title,
        "wrapping notes",
        "in the list too"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_line_that_is_not_a_command_is_a_question() {
    let mut talk = chat::Chat::new("work".into(), "n.md".into());
    assert!(
        !talk.command("what does this note say"),
        "prose is not a command"
    );
    assert!(
        talk.command("/nonsense"),
        "an unknown one is still a command, not a question"
    );
    assert!(
        talk.notice
            .as_deref()
            .is_some_and(|n| n.contains("nonsense")),
        "and it says so rather than sending it to the model"
    );
}

/// One file, as the panel would see the project it is in.
fn folder_of<'a>(name: &str, lines: &'a [String]) -> chat::Folder<'a> {
    chat::Folder {
        here: name.to_string(),
        files: vec![(name.to_string(), lines)],
    }
}

#[test]
fn a_reply_is_split_into_what_it_said_and_what_it_offered() {
    let reply = "Splitting that line reads better.\n\n<edit lines=\"6-7\">\nfirst new line\nsecond new line\n</edit>\n\nSay the word.";
    let (prose, edits) = chat::proposals(reply);
    assert!(
        !prose.contains("<edit"),
        "the machinery does not belong in the reply: {prose}"
    );
    assert!(prose.starts_with("Splitting that line"));
    assert!(prose.ends_with("Say the word."));
    assert_eq!(
        edits,
        vec![chat::Change {
            file: None,
            what: chat::What::Edit {
                from: 6,
                to: 7,
                text: "first new line\nsecond new line".into()
            },
            state: None,
        }]
    );
}

#[test]
fn a_single_line_and_a_deletion_are_both_edits() {
    let (_, one) = chat::proposals("<edit lines=\"4\">just this</edit>");
    assert_eq!(
        one,
        vec![chat::Change {
            file: None,
            what: chat::What::Edit {
                from: 4,
                to: 4,
                text: "just this".into()
            },
            state: None,
        }]
    );
    let (_, gone) = chat::proposals("<edit lines=\"4-5\">\n</edit>");
    assert_eq!(
        gone[0].becoming(&folder_of("n.md", &[])),
        "",
        "an empty block takes the lines away"
    );
}

#[test]
fn an_edit_block_inside_a_fence_is_being_talked_about() {
    let reply = "You would write:\n\n```\n<edit lines=\"1-2\">\nnew text\n</edit>\n```\n\nThat is the shape of it.";
    let (prose, edits) = chat::proposals(reply);
    assert!(
        edits.is_empty(),
        "explaining the format is not proposing a change: {edits:?}"
    );
    assert!(
        prose.contains("<edit lines=\"1-2\">"),
        "and the sample survives"
    );
}

#[test]
fn a_change_to_lines_that_are_not_there_replaces_nothing() {
    let note: Vec<String> = "one\ntwo\nthree".lines().map(str::to_string).collect();
    let edit = |from, to| chat::Change {
        file: None,
        what: chat::What::Edit {
            from,
            to,
            text: "x".into(),
        },
        state: None,
    };
    let folder = folder_of("n.md", &note);
    assert_eq!(edit(2, 3).replacing(&folder).as_deref(), Some("two\nthree"));
    assert!(
        edit(9, 9).replacing(&folder).is_none(),
        "so the panel can say so instead of guessing"
    );
    assert!(edit(3, 1).replacing(&folder).is_none());
}

#[test]
fn a_half_typed_command_offers_what_it_could_be() {
    assert_eq!(
        chat::completions("/").len(),
        chat::COMMANDS.len(),
        "a bare slash offers everything"
    );
    let one = chat::completions("/re");
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].name, "rename");
    assert!(
        chat::completions("/rename some name").is_empty(),
        "past the space the name is settled and the rest is an argument"
    );
    assert!(chat::completions("what is this note about").is_empty());
    assert!(chat::completions("/zzz").is_empty());
}

#[test]
fn tab_finishes_a_command_as_far_as_it_can() {
    assert_eq!(
        chat::complete("/re").as_deref(),
        Some("/rename "),
        "one match finishes it, with room for the argument"
    );
    assert_eq!(
        chat::complete("/h").as_deref(),
        Some("/help"),
        "and no trailing space when it takes nothing"
    );
    assert_eq!(chat::complete("/zzz"), None, "nothing to finish");
}

#[test]
fn every_command_in_the_list_is_a_command() {
    // The table is what /help prints and what completion offers. A name in it
    // that nothing answers to would be a lie told in two places.
    for command in chat::COMMANDS {
        let mut talk = chat::Chat::new("work".into(), "n.md".into());
        assert!(talk.command(&format!("/{} something", command.name)));
        let notice = talk.notice.unwrap_or_default();
        assert!(
            !notice.contains("no command called"),
            "/{} is listed but not answered",
            command.name
        );
    }
}

#[test]
fn help_lists_every_command_there_is() {
    let mut talk = chat::Chat::new("work".into(), "n.md".into());
    assert!(talk.command("/help"));
    let printed = talk.notice.expect("it says something");
    for command in chat::COMMANDS {
        assert!(
            printed.contains(command.name) && printed.contains(command.what),
            "/{} is missing from the listing:\n{printed}",
            command.name
        );
    }
    assert!(
        talk.turns.is_empty(),
        "and asking for help is not a question"
    );
}

#[test]
fn a_change_that_was_decided_stays_decided() {
    // The bug this exists for: the decision lived in memory, so a conversation
    // opened again offered a change that had already been taken.
    let reply = "Split it.\n\n<edit lines=\"6-6\">\none\ntwo\n</edit>";
    let settled = chat::settle(reply, 0, true);
    let (_, edits) = chat::proposals(&settled);
    assert_eq!(
        edits[0].state,
        Some(true),
        "written into the block: {settled}"
    );
    assert_eq!(
        edits[0].what,
        chat::What::Edit {
            from: 6,
            to: 6,
            text: "one\ntwo".into()
        },
        "and the change itself is untouched"
    );

    let rejected = chat::settle(&settled, 0, false);
    let (_, edits) = chat::proposals(&rejected);
    assert_eq!(
        edits[0].state,
        Some(false),
        "and a second thought replaces the first"
    );
    assert_eq!(
        rejected.matches("state").count(),
        1,
        "without leaving the old one behind: {rejected}"
    );
}

#[test]
fn a_decision_survives_the_file_it_is_written_in() {
    let dir = std::env::temp_dir().join(format!("pixui-settle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut talk = chat::Chat::new("reading".into(), "welcome.md".into());
    talk.turns = vec![
        Turn {
            mine: true,
            text: "tighten line 6".into(),
        },
        Turn {
            mine: false,
            text: "Like so.\n\n<edit lines=\"6-6\" state=\"applied\">\ntighter\n</edit>".into(),
        },
    ];
    talk.save(&dir).unwrap();
    let back = chat::Chat::open(
        talk.path.as_ref().unwrap(),
        "reading".into(),
        "welcome.md".into(),
    );
    let (_, edits) = chat::proposals(&back.turns[1].text);
    assert_eq!(edits[0].state, Some(true), "still applied a day later");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_settled_change_says_how_much_it_moved() {
    let two_for_one = chat::Change {
        file: None,
        what: chat::What::Edit {
            from: 6,
            to: 6,
            text: "one\ntwo".into(),
        },
        state: Some(true),
    };
    let empty = folder_of("n.md", &[]);
    assert_eq!(two_for_one.tally(&empty), (2, 1), "+2 -1");
    let gone = chat::Change {
        file: None,
        what: chat::What::Edit {
            from: 4,
            to: 8,
            text: String::new(),
        },
        state: Some(true),
    };
    assert_eq!(gone.tally(&empty), (0, 5), "taking lines out adds nothing");
}

#[test]
fn only_the_block_that_was_decided_is_marked() {
    let two = "<edit lines=\"1-1\">a</edit>\n\n<edit lines=\"5-5\">b</edit>";
    let (_, edits) = chat::proposals(&chat::settle(two, 1, true));
    assert_eq!(edits[0].state, None, "the first is still open");
    assert_eq!(
        edits[1].state,
        Some(true),
        "the second is the one that was answered"
    );
}

#[test]
fn a_change_waiting_for_an_answer_holds_the_field() {
    let note: Vec<String> = "one\ntwo\nthree\nfour\nfive\nsix"
        .lines()
        .map(str::to_string)
        .collect();
    let folder = chat::Folder {
        here: "n.md".into(),
        files: vec![("n.md".to_string(), &note[..])],
    };
    let mut talk = chat::Chat::new("work".into(), "n.md".into());
    talk.turns = vec![
        Turn {
            mine: true,
            text: "tighten line 6".into(),
        },
        Turn {
            mine: false,
            text: "Like so.\n\n<edit lines=\"6-6\">\ntighter\n</edit>".into(),
        },
    ];
    assert!(
        talk.pending(&folder),
        "it asked something back and is owed an answer"
    );

    talk.turns[1].text = chat::settle(&talk.turns[1].text, 0, false);
    assert!(!talk.pending(&folder), "rejecting is an answer too");
}

#[test]
fn a_change_that_can_no_longer_be_made_holds_nothing() {
    // Otherwise a block whose lines have gone is a conversation nobody can get
    // out of: it cannot be accepted, cannot be rejected, and blocks the field.
    let note: Vec<String> = vec!["only one line".to_string()];
    let folder = chat::Folder {
        here: "n.md".into(),
        files: vec![("n.md".to_string(), &note[..])],
    };
    let mut talk = chat::Chat::new("work".into(), "n.md".into());
    talk.turns = vec![
        Turn {
            mine: true,
            text: "fix line 40".into(),
        },
        Turn {
            mine: false,
            text: "<edit lines=\"40-41\">\nnope\n</edit>".into(),
        },
    ];
    assert!(!talk.pending(&folder));
}

#[test]
fn nothing_offered_means_nothing_to_answer() {
    let note: Vec<String> = vec!["a line".to_string()];
    let folder = chat::Folder {
        here: "n.md".into(),
        files: vec![("n.md".to_string(), &note[..])],
    };
    let mut talk = chat::Chat::new("work".into(), "n.md".into());
    assert!(
        !talk.pending(&folder),
        "an empty conversation blocks nothing"
    );
    talk.turns = vec![
        Turn {
            mine: true,
            text: "what is this about".into(),
        },
        Turn {
            mine: false,
            text: "A note about pixels.".into(),
        },
    ];
    assert!(
        !talk.pending(&folder),
        "and neither does an answer with no change in it"
    );
}

// ------------------------------------------------------------------ projects

#[test]
fn a_vault_is_a_forest_of_projects() {
    let dir = std::env::temp_dir().join(format!("pixui-forest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("beta")).unwrap();
    std::fs::create_dir_all(dir.join("alpha")).unwrap();
    std::fs::create_dir_all(dir.join(".chats").join("loose")).unwrap();
    std::fs::write(dir.join("loose.md"), "# Loose\n").unwrap();
    std::fs::write(dir.join("alpha").join("two.md"), "# Two\n").unwrap();
    std::fs::write(dir.join("alpha").join("one.md"), "# One\n").unwrap();
    std::fs::write(dir.join("beta").join("only.md"), "# Only\n").unwrap();
    std::fs::write(dir.join(".chats").join("loose").join("a.md"), "# a\n").unwrap();

    let vault = notes::read_vault(&dir);
    let slugs: Vec<String> = vault.iter().map(|n| n.slug()).collect();
    assert_eq!(
        slugs,
        vec!["loose.md", "alpha/one.md", "alpha/two.md", "beta/only.md"],
        "loose notes first, then projects in order, then their notes in order"
    );
    assert_eq!(
        notes::projects(&vault),
        vec!["".to_string(), "alpha".into(), "beta".into()]
    );
    assert!(
        !slugs.iter().any(|s| s.contains("chats")),
        "a dot folder is the program's own and is not a project"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_projects_can_hold_the_same_filename() {
    // Which is the whole reason a note is known by where it sits rather than by
    // what it is called: `todo.md` is not one note.
    let dir = std::env::temp_dir().join(format!("pixui-same-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    std::fs::write(dir.join("work").join("todo.md"), "# Work\n").unwrap();
    std::fs::write(dir.join("home").join("todo.md"), "# Home\n").unwrap();

    let vault = notes::read_vault(&dir);
    assert_eq!(vault.len(), 2);
    assert_eq!(vault[0].filename(), vault[1].filename(), "same name");
    assert_ne!(vault[0].slug(), vault[1].slug(), "different notes");
    assert_ne!(
        notes::chat::folder(&dir, &vault[0].slug()),
        notes::chat::folder(&dir, &vault[1].slug()),
        "and their conversations are not the same conversations"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_three_verbs_are_read_back_as_what_they_are() {
    let reply = "Three things.\n\n<edit file=\"a.md\" lines=\"2-3\">two\n</edit>\n\n<write file=\"b.md\">all of it</write>\n\n<delete file=\"c.md\"></delete>";
    let (prose, changes) = chat::proposals(reply);
    assert_eq!(
        prose, "Three things.",
        "and none of the machinery is left in it"
    );
    assert_eq!(changes.len(), 3);
    assert_eq!(changes[0].file.as_deref(), Some("a.md"));
    assert_eq!(
        changes[0].what,
        chat::What::Edit {
            from: 2,
            to: 3,
            text: "two".into()
        }
    );
    assert_eq!(
        changes[1].what,
        chat::What::Write {
            text: "all of it".into()
        }
    );
    assert_eq!(changes[2].what, chat::What::Delete);
}

#[test]
fn create_is_taken_to_mean_write() {
    // It is the word a model reaches for, and refusing it would be pedantry
    // with a cost.
    let (_, changes) = chat::proposals("<create file=\"new.md\">hello</create>");
    assert_eq!(
        changes[0].what,
        chat::What::Write {
            text: "hello".into()
        }
    );
}

#[test]
fn a_block_with_no_file_named_is_not_one() {
    let (prose, changes) = chat::proposals("<write>no name</write>");
    assert!(changes.is_empty(), "there is nothing to write it to");
    assert!(
        prose.contains("<write>"),
        "and it stays visible rather than vanishing"
    );
}

#[test]
fn writing_over_a_file_counts_what_it_replaces() {
    let write = chat::Change {
        file: Some("a.md".into()),
        what: chat::What::Write {
            text: "one\ntwo\nthree".into(),
        },
        state: None,
    };
    let old: Vec<String> = vec!["old".into(), "old".into()];
    let empty = chat::Folder {
        here: "here.md".into(),
        files: vec![],
    };
    let there = folder_of("a.md", &old);
    assert_eq!(write.tally(&empty), (3, 0), "a file that was not there");
    assert_eq!(write.tally(&there), (3, 2), "and one that was");
    assert_eq!(write.replacing(&there).as_deref(), Some("old\nold"));
    assert_eq!(
        write.replacing(&empty).as_deref(),
        Some(""),
        "nothing to replace yet"
    );
}

#[test]
fn a_change_finds_the_file_it_names_in_the_project() {
    let a: Vec<String> = vec!["in a".into()];
    let b: Vec<String> = vec!["in b".into()];
    let folder = chat::Folder {
        here: "a.md".into(),
        files: vec![("a.md".to_string(), &a[..]), ("b.md".to_string(), &b[..])],
    };
    assert_eq!(
        folder.lines(None),
        Some(&a[..]),
        "unqualified means the one in front of you"
    );
    assert_eq!(folder.lines(Some(&"b.md".to_string())), Some(&b[..]));
    assert!(folder.lines(Some(&"nowhere.md".to_string())).is_none());
}

#[test]
fn the_digest_says_which_project_a_note_is_in() {
    let vault = vault_of(&[("a.md", "# A\n\nfirst"), ("b.md", "# B\n\nsecond")]);
    let text = digest::vault(&vault);
    assert!(text.contains("`a.md`"), "a loose note is named by its file");
    let mut in_project = vault_of(&[("beds.md", "# Beds\n\nfour")]);
    in_project[0].project = "allotment".into();
    assert!(
        digest::vault(&in_project).contains("`allotment/beds.md`"),
        "and one in a project is named by where it sits"
    );
}

#[test]
fn the_three_verbs_do_what_they_say_to_a_project() {
    let dir = std::env::temp_dir().join(format!("pixui-apply-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    std::fs::write(
        dir.join("work").join("one.md"),
        "# One\n\nkeep\nchange me\nkeep\n",
    )
    .unwrap();
    std::fs::write(dir.join("work").join("two.md"), "# Two\n").unwrap();

    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.slug() == "work/one.md")
        .unwrap();

    // Edit: the named lines, in the named file.
    app.apply_change(&chat::Change {
        file: Some("one.md".into()),
        what: chat::What::Edit {
            from: 4,
            to: 4,
            text: "changed".into(),
        },
        state: None,
    });
    let one = app
        .notes
        .iter()
        .find(|n| n.slug() == "work/one.md")
        .unwrap();
    assert_eq!(one.buffer.to_text(), "# One\n\nkeep\nchanged\nkeep\n");

    // Write: a file that is not there yet, in the project being read.
    app.apply_change(&chat::Change {
        file: Some("three.md".into()),
        what: chat::What::Write {
            text: "# Three\n\nnew".into(),
        },
        state: None,
    });
    let three = app
        .notes
        .iter()
        .find(|n| n.slug() == "work/three.md")
        .expect("it is there now");
    assert_eq!(three.buffer.to_text(), "# Three\n\nnew");
    assert!(
        three.buffer.dirty,
        "unsaved, like anything else the editor makes"
    );

    // Write again, over a file that exists: it is replaced, not appended to.
    app.apply_change(&chat::Change {
        file: Some("two.md".into()),
        what: chat::What::Write {
            text: "# Two\n\nrewritten".into(),
        },
        state: None,
    });
    let two = app
        .notes
        .iter()
        .find(|n| n.slug() == "work/two.md")
        .unwrap();
    assert_eq!(two.buffer.to_text(), "# Two\n\nrewritten");

    // Delete: off the disk as well as out of the list.
    app.apply_change(&chat::Change {
        file: Some("one.md".into()),
        what: chat::What::Delete,
        state: None,
    });
    assert!(
        !app.notes.iter().any(|n| n.slug() == "work/one.md"),
        "gone from the vault"
    );
    assert!(
        !dir.join("work").join("one.md").exists(),
        "and gone from the disk"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_note_is_opened_by_name_wherever_it_is_filed() {
    let dir = std::env::temp_dir().join(format!("pixui-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("deep")).unwrap();
    std::fs::write(dir.join("deep").join("buried.md"), "# Buried\n").unwrap();

    let app = notes::Notes::open(dir.clone());
    let found = app.find_note("buried.md").expect("found by its bare name");
    assert_eq!(app.notes[found].slug(), "deep/buried.md");
    assert_eq!(
        app.find_note("deep/buried.md"),
        Some(found),
        "and by where it sits, which is the unambiguous way to say it"
    );
    assert!(app.find_note("nowhere.md").is_none(), "and not invented");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_merge_names_what_it_folds_in() {
    let reply = "Both say the same thing.\n\n<merge into=\"reading.md\" from=\"queue.md, patterns.md\">\nthe one file\n</merge>";
    let (prose, changes) = chat::proposals(reply);
    assert_eq!(prose, "Both say the same thing.");
    assert_eq!(changes[0].file.as_deref(), Some("reading.md"));
    assert_eq!(
        changes[0].what,
        chat::What::Merge {
            from: vec!["queue.md".into(), "patterns.md".into()],
            text: "the one file".into()
        }
    );
    let (_, spaced) = chat::proposals("<merge into=\"a.md\" from=\"b.md c.md\"></merge>");
    assert_eq!(
        spaced[0].what,
        chat::What::Merge {
            from: vec!["b.md".into(), "c.md".into()],
            text: String::new()
        },
        "named with spaces or commas, since it will use both"
    );
    let (_, none) = chat::proposals("<merge into=\"a.md\">nothing to fold</merge>");
    assert!(none.is_empty(), "a merge with nothing to merge is not one");
}

#[test]
fn an_empty_merge_joins_the_parts_as_they_are() {
    let one: Vec<String> = vec!["# One".into(), "first".into()];
    let two: Vec<String> = vec!["# Two".into(), "second".into()];
    let folder = chat::Folder {
        here: "one.md".into(),
        files: vec![
            ("one.md".to_string(), &one[..]),
            ("two.md".to_string(), &two[..]),
        ],
    };
    let merge = chat::Change {
        file: Some("both.md".into()),
        what: chat::What::Merge {
            from: vec!["one.md".into(), "two.md".into()],
            text: String::new(),
        },
        state: None,
    };
    assert_eq!(
        merge.becoming(&folder),
        "# One\nfirst\n\n# Two\nsecond",
        "end to end, in the order they were named"
    );
    assert_eq!(
        merge.tally(&folder),
        (5, 4),
        "five arriving - the blank line between the parts is one of them - and four lost"
    );
    assert!(
        merge.replacing(&folder).is_some(),
        "both parts are there, so it can be done"
    );

    let missing = chat::Change {
        file: Some("both.md".into()),
        what: chat::What::Merge {
            from: vec!["gone.md".into()],
            text: String::new(),
        },
        state: None,
    };
    assert!(
        missing.replacing(&folder).is_none(),
        "and a part that is not there makes it a mistake rather than a change"
    );
}

#[test]
fn a_merge_happens_all_at_once_or_not_at_all() {
    let dir = std::env::temp_dir().join(format!("pixui-merge-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("reading")).unwrap();
    std::fs::write(
        dir.join("reading").join("queue.md"),
        "# Queue\n\n- a book\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("reading").join("patterns.md"),
        "# Patterns\n\nnotes\n",
    )
    .unwrap();

    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.slug() == "reading/queue.md")
        .unwrap();
    app.apply_change(&chat::Change {
        file: Some("queue.md".into()),
        what: chat::What::Merge {
            from: vec!["queue.md".into(), "patterns.md".into()],
            text: "# Queue\n\n- a book\n\nnotes".into(),
        },
        state: None,
    });

    let kept = app
        .notes
        .iter()
        .find(|n| n.slug() == "reading/queue.md")
        .expect("the one merged into stays");
    assert_eq!(kept.buffer.to_text(), "# Queue\n\n- a book\n\nnotes");
    assert!(
        !app.notes.iter().any(|n| n.slug() == "reading/patterns.md"),
        "and the one folded in is gone"
    );
    assert!(
        !dir.join("reading").join("patterns.md").exists(),
        "from the disk too"
    );
    assert!(
        dir.join("reading").join("queue.md").exists(),
        "the target is not deleted for being one of its own parts"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_conversation_belongs_to_the_project_not_to_one_file() {
    let dir = std::env::temp_dir().join(format!("pixui-bound-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("reading")).unwrap();

    // Started while reading one file...
    let mut talk = chat::Chat::new("reading".into(), "queue.md".into());
    talk.turns = vec![Turn {
        mine: true,
        text: "what is in this project".into(),
    }];
    talk.save(&dir).unwrap();

    // ...and found again while reading another.
    let listed = chat::filed(&dir, "reading");
    assert_eq!(
        listed.len(),
        1,
        "the same conversations, whichever file you asked from"
    );

    let reopened = chat::Chat::open(&listed[0].path, "reading".into(), "patterns.md".into());
    assert_eq!(
        reopened.turns, talk.turns,
        "with everything that was said in it"
    );
    assert_eq!(
        reopened.focus, "patterns.md",
        "and looking at whichever file it was opened from this time"
    );
    assert_eq!(reopened.project, "reading");

    assert!(
        chat::filed(&dir, "allotment").is_empty(),
        "another project's conversations are its own"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loose_notes_keep_their_conversations_at_the_top() {
    let dir = std::env::temp_dir().join("pixui-loose-chats");
    assert_eq!(chat::folder(&dir, ""), dir.join(".chats"));
    assert_eq!(
        chat::folder(&dir, "reading"),
        dir.join(".chats").join("reading")
    );
    assert_eq!(
        chat::called(""),
        "THE VAULT",
        "which is what to call it out loud"
    );
    assert_eq!(chat::called("reading"), "READING");
}

#[test]
fn a_created_note_lands_in_its_own_project() {
    // The bug this exists for: a new note was appended after every project, so
    // the sidebar - which begins a heading wherever the project changes - drew
    // its project a second time, and the drawer showed two of them.
    let dir = std::env::temp_dir().join(format!("pixui-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (project, file) in [
        ("aquarium", "stock.md"),
        ("aquarium", "water.md"),
        ("bicycle", "routes.md"),
        ("typography", "faces.md"),
    ] {
        std::fs::create_dir_all(dir.join(project)).unwrap();
        std::fs::write(dir.join(project).join(file), "# x\n").unwrap();
    }

    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/stock.md")
        .unwrap();
    app.apply_change(&chat::Change {
        file: Some("plants.md".into()),
        what: chat::What::Write {
            text: "# Plants\n".into(),
        },
        state: None,
    });

    let slugs: Vec<String> = app.notes.iter().map(|n| n.slug()).collect();
    assert_eq!(
        slugs,
        vec![
            // The reference note the vault installs, lying loose at the top.
            "markdown-showcase.md",
            "aquarium/plants.md",
            "aquarium/stock.md",
            "aquarium/water.md",
            "bicycle/routes.md",
            "typography/faces.md",
        ],
        "in the order the vault reads, so each project is one run of notes"
    );

    // Which is the property the sidebar depends on: walking the list, a
    // project must never be met twice.
    let mut met: Vec<&str> = Vec::new();
    for note in &app.notes {
        if met.last() != Some(&note.project.as_str()) {
            assert!(
                !met.contains(&note.project.as_str()),
                "{} is met twice, so it would be drawn twice",
                note.project
            );
            met.push(&note.project);
        }
    }
    assert_eq!(
        app.notes[app.current].slug(),
        "aquarium/plants.md",
        "and the new note is the one being read"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_note_goes_into_the_project_being_read() {
    let dir = std::env::temp_dir().join(format!("pixui-newnote-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bicycle")).unwrap();
    std::fs::write(dir.join("bicycle").join("routes.md"), "# Routes\n").unwrap();
    std::fs::create_dir_all(dir.join("typography")).unwrap();
    std::fs::write(dir.join("typography").join("faces.md"), "# Faces\n").unwrap();

    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.slug() == "bicycle/routes.md")
        .unwrap();
    let at = app.insert_note(notes::Note::blank("bicycle".into()));
    assert_eq!(app.notes[at].project, "bicycle");
    // The property the sidebar rests on: whatever else the order is, a
    // project's notes are one unbroken run, so its heading is drawn once.
    let mut met: Vec<String> = Vec::new();
    for note in &app.notes {
        if met.last() != Some(&note.project) {
            assert!(
                !met.contains(&note.project),
                "{} is met twice, so its heading would be drawn twice",
                note.project
            );
            met.push(note.project.clone());
        }
    }
    assert!(
        app.notes
            .iter()
            .position(|n| n.project == "typography")
            .unwrap()
            > at,
        "and it went in before the projects that sort after it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------- projects on disk

fn vault_with(dir: &std::path::Path, files: &[(&str, &str)]) {
    let _ = std::fs::remove_dir_all(dir);
    for (path, text) in files {
        let at = dir.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }
}

#[test]
fn renaming_a_note_keeps_it_in_its_project() {
    let dir = std::env::temp_dir().join(format!("pixui-rn-{}", std::process::id()));
    vault_with(&dir, &[("aquarium/stock.md", "# Stock\n")]);
    let mut app = notes::Notes::open(dir.clone());
    let i = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/stock.md")
        .unwrap();
    app.rename_note(i, "livestock.md");
    assert!(
        dir.join("aquarium").join("livestock.md").exists(),
        "in the folder it was in"
    );
    assert!(
        !dir.join("livestock.md").exists(),
        "not at the top of the vault"
    );
    assert_eq!(app.notes[i].slug(), "aquarium/livestock.md");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_note_takes_it_off_the_disk() {
    let dir = std::env::temp_dir().join(format!("pixui-dn-{}", std::process::id()));
    vault_with(
        &dir,
        &[
            ("aquarium/stock.md", "# Stock\n"),
            ("aquarium/water.md", "# Water\n"),
        ],
    );
    let mut app = notes::Notes::open(dir.clone());
    let i = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/stock.md")
        .unwrap();
    app.delete_note(i);
    assert!(!dir.join("aquarium").join("stock.md").exists());
    assert!(!app.notes.iter().any(|n| n.slug() == "aquarium/stock.md"));
    assert!(
        app.notes.iter().any(|n| n.slug() == "aquarium/water.md"),
        "and only that one"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renaming_a_project_takes_its_notes_and_its_chats_with_it() {
    let dir = std::env::temp_dir().join(format!("pixui-rp-{}", std::process::id()));
    vault_with(
        &dir,
        &[
            ("aquarium/stock.md", "# Stock\n"),
            ("aquarium/water.md", "# Water\n"),
            (
                ".chats/aquarium/why-shrimp-die.md",
                "# why shrimp die\n\n## you\n\nwhy\n",
            ),
        ],
    );
    let mut app = notes::Notes::open(dir.clone());
    app.rename_project("aquarium", "fishtank");

    assert!(
        dir.join("fishtank").join("stock.md").exists(),
        "the folder moved"
    );
    assert!(!dir.join("aquarium").exists());
    assert_eq!(
        app.notes.iter().filter(|n| n.project == "fishtank").count(),
        2,
        "and the notes know where they are"
    );
    assert!(
        app.notes.iter().all(|n| n
            .path
            .as_ref()
            .is_none_or(|p| !p.starts_with(dir.join("aquarium")))),
        "with their paths pointing at the new folder"
    );
    assert_eq!(
        chat::filed(&dir, "fishtank").len(),
        1,
        "and the conversations came too - a project that moved without them \
         would look like one nobody had talked about"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_project_takes_everything_in_it() {
    let dir = std::env::temp_dir().join(format!("pixui-dp-{}", std::process::id()));
    vault_with(
        &dir,
        &[
            ("aquarium/stock.md", "# Stock\n"),
            ("aquarium/water.md", "# Water\n"),
            ("bicycle/routes.md", "# Routes\n"),
            (".chats/aquarium/a.md", "# a\n\n## you\n\nq\n"),
        ],
    );
    let mut app = notes::Notes::open(dir.clone());
    app.delete_project("aquarium");

    assert!(!dir.join("aquarium").exists());
    assert!(!app.notes.iter().any(|n| n.project == "aquarium"));
    assert!(
        app.notes.iter().any(|n| n.slug() == "bicycle/routes.md"),
        "and nothing else"
    );
    assert!(
        chat::filed(&dir, "aquarium").is_empty(),
        "its conversations went with it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_project_arrives_with_a_note_so_it_can_be_seen() {
    let dir = std::env::temp_dir().join(format!("pixui-np-{}", std::process::id()));
    vault_with(&dir, &[("bicycle/routes.md", "# Routes\n")]);
    let mut app = notes::Notes::open(dir.clone());
    let made = app.new_project();
    assert!(!made.is_empty());
    assert!(dir.join(&made).is_dir(), "the folder is there");
    assert_eq!(
        app.notes[app.current].project, made,
        "and the note being read is the one in it - a project with nothing in \
         it has no heading, so it would be a folder you could not see"
    );
    let again = app.new_project();
    assert_ne!(again, made, "a second one does not land on the first");
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------- the web

use notes::web;

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
fn a_tool_that_fails_says_so_rather_than_saying_nothing() {
    // The one answer that reliably makes a model invent is no answer at all:
    // handed an empty result it fills the gap, and it filled one with a
    // llama.cpp version that has never existed.
    let said = notes::tools::run("no-such-tool", "anything");
    assert!(!said.trim().is_empty());
    assert!(said.contains("no tool called"), "{said}");
}

#[test]
fn every_tool_says_when_it_is_for_and_not_only_what_it_is() {
    // Measured: a tool described as what it does was reached for once in four
    // questions that needed it; the same tool described in terms of when it is
    // needed was reached for four times in four.
    for tool in notes::tools::available(true) {
        assert!(
            ["Use ", "use it", "Call this"]
                .iter()
                .any(|said| tool.about.contains(said)),
            "/{} never says when to use it",
            tool.name
        );
        assert!(
            tool.about.len() > 120,
            "/{} is described too thinly",
            tool.name
        );
        assert!(
            !tool.takes.1.is_empty(),
            "/{} does not say what its argument is",
            tool.name
        );
    }
}

#[test]
fn tools_are_declared_the_way_the_model_was_trained_to_read_them() {
    let declared = notes::llm::declare(&notes::tools::available(true));
    // Lifted from the chat template baked into the weights, not invented: the
    // model obeys this shape and argues with any other.
    assert!(declared.starts_with("# Tools\n\nYou have access to the following functions:"));
    assert!(declared.contains("<tools>") && declared.contains("</tools>"));
    assert!(declared.contains("<tool_call>\n<function=example_function_name>"));
    assert!(declared.contains("\"name\": \"weather\""));
    assert!(declared.contains("\"required\": [\"place\"]"));
}

#[test]
fn a_conversation_is_told_about_its_tools_and_an_edit_is_not() {
    let editing = "rewrite the passage and nothing else";
    let chat = notes::llm::Ask {
        turns: vec![notes::llm::Turn {
            mine: true,
            text: "hello".into(),
        }],
        tools: notes::tools::available(true),
        ..Default::default()
    };
    assert!(
        chat.system(editing).starts_with("# Tools"),
        "tools come first, as the template puts them"
    );
    assert!(chat
        .system(editing)
        .contains("You are talking with somebody"));

    let quiet = notes::llm::Ask {
        turns: vec![notes::llm::Turn {
            mine: true,
            text: "hello".into(),
        }],
        ..Default::default()
    };
    assert!(
        !quiet.system(editing).contains("# Tools"),
        "no tools, no mention of tools"
    );

    let rewrite = notes::llm::Ask {
        source: "a line".into(),
        tools: notes::tools::available(true),
        ..Default::default()
    };
    assert_eq!(
        rewrite.system(editing),
        editing,
        "an edit is not a conversation and does not browse"
    );
}

#[test]
fn a_tool_call_is_read_out_of_a_reply() {
    let said = "Let me check.\n\n<tool_call>\n<function=weather>\n<parameter=place>\nBerlin\n</parameter>\n</function>\n</tool_call>";
    assert_eq!(
        notes::llm::called(said),
        Some(("weather".to_string(), "Berlin".to_string()))
    );
    // The trap that cost an afternoon: splitting on `>` first eats the `>` of
    // `</parameter>` and leaves the tag on the end of the value, which fed the
    // tool a broken argument, got nothing back, and had the model inventing a
    // llama.cpp version to fill the gap.
    let (_, arg) = notes::llm::called(said).unwrap();
    assert!(
        !arg.contains("</parameter"),
        "the closing tag is not part of the value"
    );

    assert_eq!(notes::llm::called("just an answer, no call"), None);
    assert_eq!(
        notes::llm::called("<function=weather></function>"),
        None,
        "no argument is no call"
    );
}

#[test]
fn what_was_looked_up_travels_with_the_answer() {
    let used = notes::llm::Used {
        tool: "weather".into(),
        arg: "Berlin".into(),
        result: "Berlin right now: 23C, Overcast".into(),
    };
    let reply = format!("{}It is 23C and overcast in Berlin.", used.written());
    let (prose, looked) = chat::lookups(&reply);
    assert_eq!(
        prose, "It is 23C and overcast in Berlin.",
        "the answer reads as an answer"
    );
    assert_eq!(looked.len(), 1);
    assert_eq!(looked[0].tool, "weather");
    assert_eq!(looked[0].arg, "Berlin");
    assert!(
        looked[0].result.contains("23C"),
        "and what it found is kept with it"
    );
}

#[test]
fn a_reply_can_have_looked_something_up_and_still_propose_a_change() {
    let reply = "<used tool=\"weather\" arg=\"Berlin\">\nBerlin right now: 23C\n</used>\n\nI put it in the note.\n\n<edit file=\"today.md\" lines=\"3-3\">\nBerlin, 23C and overcast.\n</edit>";
    let (rest, looked) = chat::lookups(reply);
    assert_eq!(looked.len(), 1);
    let (prose, changes) = chat::proposals(&rest);
    assert_eq!(prose, "I put it in the note.");
    assert_eq!(changes.len(), 1, "and the change is still a change");
    assert_eq!(changes[0].file.as_deref(), Some("today.md"));
}

#[test]
fn nothing_is_looked_up_in_a_reply_that_looked_nothing_up() {
    let (prose, looked) = chat::lookups("A plain answer with no tools in it.");
    assert!(looked.is_empty());
    assert_eq!(prose, "A plain answer with no tools in it.");
}

#[test]
fn the_web_is_off_until_it_is_turned_on() {
    // The one setting here that is not about taste: with it on, a question can
    // send a place name to somebody else's server.
    assert!(!Settings::default().web, "off by default");
    assert!(Settings::parse("web = on\n").web);
    assert!(!Settings::parse("web = off\n").web);
    assert!(
        !Settings::parse("scheme = NORD\n").web,
        "and a settings file that predates it does not turn it on"
    );
}

// ------------------------------------------------------------------- the sums

use notes::calc;

fn sum(text: &str) -> String {
    calc::evaluate(text).unwrap_or_else(|why| format!("!{why}"))
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
    assert!(sum("").contains("nothing here"));
    assert!(sum("2 & 3").contains("not something this can work out"));
    assert!(
        sum("2 3").contains("left over"),
        "two numbers side by side is a typo"
    );
}

#[test]
fn working_a_sum_out_needs_no_permission_to_leave_the_machine() {
    // The web switch is about sending a place name to somebody else's server.
    // Arithmetic happens here, so it is offered either way.
    let offline: Vec<&str> = notes::tools::available(false)
        .iter()
        .map(|t| t.name)
        .collect();
    assert!(
        offline.contains(&"calc"),
        "with the network off it is still there: {offline:?}"
    );
    assert!(
        !offline.iter().any(|t| *t == "weather" || *t == "fetch"),
        "and nothing that would leave the machine is"
    );

    let online: Vec<&str> = notes::tools::available(true)
        .iter()
        .map(|t| t.name)
        .collect();
    assert!(
        online.starts_with(&["calc"]),
        "and it is still there with the network on"
    );
    assert!(online.contains(&"weather") && online.contains(&"fetch"));
}

// ------------------------------------------------------------------ keeping up

#[test]
fn a_note_that_changed_is_written_down_without_being_asked() {
    let dir = std::env::temp_dir().join(format!("pixui-keep-{}", std::process::id()));
    vault_with(&dir, &[("aquarium/water.md", "# Water\n\nas it was\n")]);
    let mut app = notes::Notes::open(dir.clone());
    let i = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/water.md")
        .unwrap();

    app.notes[i].buffer.checkpoint();
    app.notes[i].buffer.insert_str_at(2, 0, "changed: ");
    app.notes[i].buffer.dirty = true;

    assert_eq!(app.keep_up(), 1, "one note had something to say");
    let on_disk = std::fs::read_to_string(dir.join("aquarium").join("water.md")).unwrap();
    assert!(
        on_disk.contains("changed: as it was"),
        "and it is on the disk: {on_disk:?}"
    );
    assert!(
        !app.notes[i].buffer.dirty,
        "and is not still waiting to be written"
    );
    assert_eq!(app.keep_up(), 0, "nothing to do the second time");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_note_with_no_name_yet_is_left_alone() {
    // It has nowhere to go, and choosing a filename for somebody is not a
    // thing to do behind their back.
    let dir = std::env::temp_dir().join(format!("pixui-noname-{}", std::process::id()));
    vault_with(&dir, &[("aquarium/water.md", "# Water\n")]);
    let mut app = notes::Notes::open(dir.clone());
    let at = app.insert_note(notes::Note::blank("aquarium".into()));
    app.notes[at].buffer.dirty = true;
    assert_eq!(app.keep_up(), 0, "nothing was written");
    assert!(
        app.notes[at].buffer.dirty,
        "and it still knows it is unsaved"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------- backlinks

#[test]
fn a_note_knows_what_points_at_it() {
    let dir = std::env::temp_dir().join(format!("pixui-back-{}", std::process::id()));
    vault_with(
        &dir,
        &[
            ("aquarium/water.md", "# Water\n\nTest on Sundays.\n"),
            (
                "aquarium/stock.md",
                "# Stock\n\nThe shrimp keep dying. See [the water](water.md).\n",
            ),
            (
                "aquarium/plants.md",
                "# Plants\n\nFerts twice a week. Nothing about the other note.\n",
            ),
            (
                "aquarium/notes.md",
                "# Notes\n\nA wiki link to [[water]] as well.\n",
            ),
            (
                "bicycle/routes.md",
                "# Routes\n\nA bare water.md here means this project's own.\n",
            ),
        ],
    );
    let app = notes::Notes::open(dir.clone());
    let water = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/water.md")
        .unwrap();

    let names: Vec<String> = app
        .linked_from(water)
        .iter()
        .map(|&i| app.notes[i].slug())
        .collect();
    assert!(
        names.contains(&"aquarium/stock.md".to_string()),
        "a markdown link counts: {names:?}"
    );
    assert!(
        names.contains(&"aquarium/notes.md".to_string()),
        "and a wiki link counts"
    );
    assert!(
        !names.contains(&"aquarium/plants.md".to_string()),
        "a note that says nothing does not"
    );
    assert!(
        !names.contains(&"bicycle/routes.md".to_string()),
        "and a bare name from another project points at that project's own file"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_link_across_projects_needs_the_path() {
    let dir = std::env::temp_dir().join(format!("pixui-across-{}", std::process::id()));
    vault_with(
        &dir,
        &[
            ("aquarium/water.md", "# Water\n"),
            (
                "bicycle/routes.md",
                "# Routes\n\nSee [the water note](aquarium/water.md).\n",
            ),
        ],
    );
    let app = notes::Notes::open(dir.clone());
    let water = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/water.md")
        .unwrap();
    let names: Vec<String> = app
        .linked_from(water)
        .iter()
        .map(|&i| app.notes[i].slug())
        .collect();
    assert_eq!(names, vec!["bicycle/routes.md"], "spelled out, it counts");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_longer_name_is_not_this_name() {
    let dir = std::env::temp_dir().join(format!("pixui-longer-{}", std::process::id()));
    vault_with(
        &dir,
        &[
            ("aquarium/water.md", "# Water\n"),
            ("aquarium/rainwater.md", "# Rainwater\n"),
            (
                "aquarium/stock.md",
                "# Stock\n\nSee [rain](rainwater.md).\n",
            ),
        ],
    );
    let app = notes::Notes::open(dir.clone());
    let water = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/water.md")
        .unwrap();
    assert!(
        app.linked_from(water).is_empty(),
        "rainwater.md ends in water.md and is not water.md"
    );
    let rain = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/rainwater.md")
        .unwrap();
    assert_eq!(
        app.linked_from(rain).len(),
        1,
        "and the note it does point at knows"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_note_does_not_point_at_itself() {
    let dir = std::env::temp_dir().join(format!("pixui-self-{}", std::process::id()));
    vault_with(
        &dir,
        &[(
            "aquarium/water.md",
            "# Water\n\nA note about water.md itself.\n",
        )],
    );
    let app = notes::Notes::open(dir.clone());
    let water = app
        .notes
        .iter()
        .position(|n| n.slug() == "aquarium/water.md")
        .unwrap();
    assert!(app.linked_from(water).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------- streaming and stopping

/// A backend that writes slowly and does as it is told, so the plumbing around
/// one can be tested without twelve gigabytes of weights.
struct Dawdler;

impl notes::llm::Backend for Dawdler {
    fn name(&self) -> String {
        "DAWDLER".into()
    }
    fn edit(
        &mut self,
        _ask: &notes::llm::Ask,
        watch: &mut dyn notes::llm::Watcher,
    ) -> notes::llm::Reply {
        let mut said = String::new();
        for i in 0..200 {
            if !watch.carry_on() {
                break;
            }
            said.push_str(&format!("word{i} "));
            watch.tick(
                notes::llm::Progress {
                    written: i + 1,
                    ..Default::default()
                },
                said.trim(),
            );
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
        Ok(said.trim().to_string())
    }
}

#[test]
fn the_answer_can_be_watched_as_it_is_written() {
    let mut helper = notes::llm::Assistant::spawn(Box::new(Dawdler));
    assert!(helper.ask(notes::llm::Ask::default()));

    let mut lengths = Vec::new();
    let reply = loop {
        if let Some(r) = helper.poll() {
            break r;
        }
        let n = helper.partial().len();
        if lengths.last() != Some(&n) {
            lengths.push(n);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    .expect("an answer");

    assert!(
        lengths.len() > 5,
        "it arrived in pieces rather than all at once: {lengths:?}"
    );
    assert!(
        lengths.windows(2).all(|w| w[0] <= w[1]),
        "and the pieces only ever grew"
    );
    assert!(reply.ends_with("word199"), "and the whole of it turned up");
    assert!(
        helper.partial().is_empty(),
        "the partial is cleared once it is whole"
    );
}

#[test]
fn a_question_can_be_given_up_on() {
    let mut helper = notes::llm::Assistant::spawn(Box::new(Dawdler));
    assert!(helper.ask(notes::llm::Ask::default()));

    let started = std::time::Instant::now();
    let mut stopped = false;
    let reply = loop {
        if let Some(r) = helper.poll() {
            break r;
        }
        if !stopped && helper.partial().split_whitespace().count() > 5 {
            helper.stop();
            stopped = true;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(started.elapsed().as_secs() < 20, "it never stopped");
    }
    .expect("what it had got to");

    let words = reply.split_whitespace().count();
    assert!(stopped, "it was asked");
    assert!(words < 200, "it did not finish: {words} words");
    assert!(
        words >= 5,
        "and what it had got to came back rather than nothing - half a \
         paragraph you asked it to stop writing is what you were looking at"
    );

    // And the next question is not still trying to stop.
    assert!(helper.ask(notes::llm::Ask::default()));
    let again = loop {
        if let Some(r) = helper.poll() {
            break r;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(started.elapsed().as_secs() < 30);
    }
    .expect("an answer");
    assert!(again.ends_with("word199"), "the flag was put down again");
}

// ----------------------------------------------------------------- the clock

#[test]
fn a_date_says_what_day_it_falls_on() {
    use notes::clock;
    // Checkable against the world: the moon landing was a Sunday, and the
    // first day of 2000 was a Saturday.
    let moon = clock::about("1969-07-20").unwrap();
    assert!(moon.contains("20 July 1969 is a Sunday"), "{moon}");
    assert!(moon.contains("days ago"), "and it says how long ago");
    let y2k = clock::about("2000-01-01").unwrap();
    assert!(y2k.contains("1 January 2000 is a Saturday"), "{y2k}");
    // A leap day is a real day, and 2000 was a leap year while 1900 was not.
    assert!(clock::about("2000-02-29")
        .unwrap()
        .contains("29 February 2000 is a Tuesday"));
}

#[test]
fn today_is_answered_without_being_asked_which_day_it_is() {
    let now = notes::clock::about("today").unwrap();
    assert_eq!(
        notes::clock::about("").unwrap(),
        now,
        "an empty question means today"
    );
    assert_eq!(
        notes::clock::about("NOW").unwrap(),
        now,
        "however it is spelled"
    );
    assert!(
        now.contains("the time is"),
        "and it says the time as well: {now}"
    );
    for day in [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ] {
        if now.starts_with(day) {
            return;
        }
    }
    panic!("it should begin with a day of the week: {now}");
}

#[test]
fn a_day_with_no_year_means_the_next_one_there_is() {
    // Which is what somebody asking how long until Christmas means by it, and
    // what stopped the model reaching for a year out of its training. It says
    // when the one before was as well: a day that comes round is asked about
    // in both directions - "when is the next and how long since the last" is
    // one question - and answering half of it is how the other half came to be
    // counted by hand, and wrongly: 274 days, then 306, then 366, where it was
    // 245.
    let said = notes::clock::about("12-25").unwrap();
    assert!(said.contains("The next 25 December"), "{said}");
    assert!(said.contains("The one before was"), "{said}");
    assert!(said.contains("days from today"), "{said}");
    assert!(said.contains("days ago"), "{said}");
    // And how long that is in the units anybody would use for it.
    assert!(said.contains("months") || said.contains("year"), "{said}");
}

#[test]
fn a_year_that_has_gone_is_answered_and_corrected() {
    // The mistake it kept making: asked how long until Christmas it named a
    // year already past. The answer says where to look instead rather than
    // leaving it to work that out.
    let said = notes::clock::about("2020-12-25").unwrap();
    assert!(
        said.contains("days ago"),
        "it answers what was asked: {said}"
    );
    assert!(
        said.contains("If you meant the next one"),
        "and points at the next one"
    );
    assert!(said.contains("-12-25"), "by name");
}

#[test]
fn something_that_is_not_a_date_is_not_guessed_at() {
    // A date written any other way is ambiguous about which number is the
    // month, and guessing is how a note ends up dated wrongly.
    assert!(notes::clock::about("next tuesday").is_err());
    assert!(
        notes::clock::about("25/12/2026").is_err(),
        "day-first or month-first?"
    );
    assert!(
        notes::clock::about("2026-13-01").is_err(),
        "there is no thirteenth month"
    );
    assert!(notes::clock::about("2026-12-32").is_err());
}

#[test]
fn knowing_the_day_needs_no_permission_to_leave_the_machine() {
    let offline: Vec<&str> = notes::tools::available(false)
        .iter()
        .map(|t| t.name)
        .collect();
    assert!(
        offline.contains(&"date"),
        "the clock is here, not out there: {offline:?}"
    );
    assert!(offline.contains(&"calc"));
    // And reading a note, which is the vault rather than the network.
    assert!(offline.contains(&"read"));
    assert_eq!(
        offline.len(),
        3,
        "these are the three that need nobody's permission: {offline:?}"
    );
}

#[test]
fn a_refusal_says_there_is_a_switch() {
    // The bug this exists for: with looking things up off, the model answered
    // "I don't have access to that", which is true and is also exactly what a
    // broken feature says. One of them has a switch.
    let off = notes::llm::Ask {
        turns: vec![notes::llm::Turn {
            mine: true,
            text: "how hot is it in berlin".into(),
        }],
        tools: notes::tools::available(false),
        web_off: true,
        ..Default::default()
    };
    let said = off.system("editing");
    assert!(said.contains("switched off"), "it is told which it is");
    assert!(said.contains("/web"), "and how to change it");
    assert!(
        !said.contains("\"name\": \"weather\""),
        "without being offered the tool itself"
    );

    let on = notes::llm::Ask {
        tools: notes::tools::available(true),
        web_off: false,
        ..off.clone()
    };
    let said = on.system("editing");
    assert!(
        !said.contains("switched off"),
        "and nothing about switches when it is on"
    );
    assert!(said.contains("\"name\": \"weather\""));
}

#[test]
fn the_switch_can_be_thrown_from_the_conversation() {
    let mut talk = chat::Chat::new("aquarium".into(), "water.md".into());
    assert!(talk.command("/web"), "it is a command");
    assert!(
        !talk
            .notice
            .as_deref()
            .unwrap_or_default()
            .contains("no command"),
        "and one this knows"
    );
    assert!(
        notes::chat::COMMANDS.iter().any(|c| c.name == "web"),
        "so /help says it is there"
    );
}

#[test]
fn nobody_is_shown_the_calling_out() {
    use notes::llm::without_machinery;
    // What leaked: while the answer was being watched as it was written, the
    // first thing to arrive was the block of tags the model writes to reach
    // for a tool, and the panel drew it.
    let mid = "Let me check.\n\n<tool_call>\n<function=weather>\n<parameter=place>\nBerlin";
    assert_eq!(
        without_machinery(mid).trim(),
        "Let me check.",
        "what it said is kept, what it is doing is not"
    );
    // A block half written is a block still being written, so it takes the
    // rest with it rather than being shown in pieces.
    assert!(!without_machinery(mid).contains('<'));

    let done = "Here you go.\n\n<tool_call>\n<function=weather>\n<parameter=place>\nBerlin\n</parameter>\n</function>\n</tool_call>";
    assert_eq!(without_machinery(done).trim(), "Here you go.");

    let plain = "It is 23C in Berlin, overcast.";
    assert_eq!(without_machinery(plain), plain, "an answer is left alone");

    // The other spelling, in case the outer tag never arrives.
    assert_eq!(without_machinery("ok <function=calc>").trim(), "ok");

    // A call in the middle keeps what is on both sides of it. The old rule cut
    // everything from the first tag onwards, so a model that said something,
    // looked something up and then carried on lost the second half.
    let around = "First, the sum.\n<tool_call><function=calc><parameter=expression>2+2</parameter></function></tool_call>\nAnd that is that.";
    let kept = without_machinery(around);
    assert!(kept.contains("First, the sum."), "{kept:?}");
    assert!(kept.contains("And that is that."), "{kept:?}");
    assert!(!kept.contains('<'), "{kept:?}");

    // Other families spell it differently, and a closing tag whose opening
    // never arrived is not language either.
    for machinery in [
        "<|tool_call_start|>[date(when='today')]<|tool_call_end|>",
        "[TOOL_CALL]calc(1+1)[/TOOL_CALL]",
        "</tool_call>",
        "</parameter>\n</function>\n</tool_call>",
    ] {
        let said = format!("Here you go.\n{machinery}");
        let kept = without_machinery(&said);
        assert_eq!(kept, "Here you go.", "left machinery in: {kept:?}");
    }

    // And two calls in one reply take both of themselves away.
    let twice = "<tool_call><function=calc><parameter=expression>1</parameter></function></tool_call>\n<tool_call><function=date><parameter=when>today</parameter></function></tool_call>";
    assert_eq!(without_machinery(twice), "");
}

#[test]
fn the_clock_says_it_gives_the_time_and_not_only_the_day() {
    // The bug this exists for: the tool returned the clock time all along, and
    // the description never mentioned it. Asked whether it was evening yet the
    // model answered "I don't know your location, so I can't tell you what
    // time it is" - and never called the thing that knew.
    let clock = notes::tools::available(false)
        .into_iter()
        .find(|t| t.name == "date")
        .expect("the clock is always offered");
    for word in ["time", "date", "year", "day of the week", "evening"] {
        assert!(
            clock.about.contains(word),
            "the clock never mentions {word}, so it will not be reached for one"
        );
    }
    // And it does give what it claims.
    let now = notes::clock::about("today").unwrap();
    assert!(now.contains("the time is"), "{now}");
}

#[test]
fn what_was_looked_up_reads_as_a_sentence() {
    use notes::chat::Lookup;
    let said = |tool: &str, arg: &str| {
        Lookup {
            tool: tool.into(),
            arg: arg.into(),
            result: String::new(),
        }
        .said()
    };
    // `date` and `today` are what the wiring calls it. Somebody who asked what
    // time it was should not have to read the wiring to see where the answer
    // came from.
    assert_eq!(said("date", "today"), "CHECKED THE DATE AND TIME");
    assert_eq!(said("date", ""), "CHECKED THE DATE AND TIME");
    assert_eq!(said("date", "2026-12-25"), "CHECKED WHEN 2026-12-25 IS");
    assert_eq!(said("calc", "384 * 517"), "WORKED OUT 384 * 517");
    assert_eq!(said("weather", "Berlin"), "CHECKED THE WEATHER IN BERLIN");
    assert_eq!(
        said("release", "ggml-org/llama.cpp"),
        "CHECKED THE LATEST RELEASE OF GGML-ORG/LLAMA.CPP"
    );
    // A page is named by where it is, not by the query string that got there.
    assert_eq!(
        said("fetch", "https://example.com/a/b?q=1&r=2"),
        "READ EXAMPLE.COM"
    );
    // And a tool added after this was written still gets a sentence.
    assert_eq!(said("translate", "hola"), "USED TRANSLATE ON HOLA");
}

#[test]
fn asked_for_today_the_clock_says_not_to_count_from_it() {
    // Asked how long until Christmas, the model called the clock, read today's
    // date off it, and then counted the days in its head: 86, where the answer
    // was 120. The tool could have answered the actual question - give it
    // 12-25 and it says how far off that is - and its description already said
    // so. Saying it again in the answer, a line before the model writes, is
    // what actually landed: 6 out of 6 afterwards, against 4.
    let now = notes::clock::about("today").unwrap();
    assert!(now.contains("the time is"), "{now}");
    assert!(now.contains("Do not work out how far off"), "{now}");
    // With the form spelled out rather than described. An instruction to ask
    // "with the day instead" was followed about as often as it was ignored;
    // naming 12-25 in it is what carried.
    assert!(now.contains("12-25"), "{now}");
    // And not on the answers that are already about another day, which say how
    // far off it is and have nothing to warn against.
    let then = notes::clock::about("12-25").unwrap();
    assert!(!then.contains("Do not work out"), "{then}");
    assert!(then.contains("days from today"), "{then}");
}

#[test]
fn copying_a_turn_gives_what_is_on_the_screen() {
    use notes::chat::{copyable, Chat};
    use notes::llm::Turn;
    // A reply carries the record of what it looked up, and the panel draws
    // that as a sentence rather than as the block it is written in. Pasting
    // the block into a note would hand over wiring nobody asked for.
    let said =
        "<used tool=\"date\" arg=\"today\">\nThursday 27 August 2026\n</used>\n\nIt is Thursday.";
    let theirs = Turn {
        mine: false,
        text: said.into(),
    };
    assert_eq!(copyable(&theirs), "It is Thursday.");
    // What a change offered says is kept, because that is content: it is the
    // lines it wants to put in the file.
    let with_edit = Turn {
        mine: false,
        text: "Here you go.\n\n<edit file=\"notes.md\" lines=\"1\">\nfixed\n</edit>".into(),
    };
    assert!(copyable(&with_edit).contains("<edit file=\"notes.md\""));
    assert!(copyable(&with_edit).contains("fixed"));
    // A question is copied as it was asked.
    let mine = Turn {
        mine: true,
        text: "  what day is it?  ".into(),
    };
    assert_eq!(copyable(&mine), "what day is it?");

    // And the whole conversation reads like the file it is saved as.
    let mut chat = Chat::new("home".into(), "notes.md".into());
    chat.turns = vec![mine, theirs];
    let whole = chat.transcript();
    assert!(whole.starts_with('#'), "{whole}");
    assert!(whole.contains("## you"), "{whole}");
    assert!(whole.contains("## assistant"), "{whole}");
    assert!(whole.contains("what day is it?"), "{whole}");
    assert!(whole.contains("It is Thursday."), "{whole}");
    assert!(!whole.contains("<used"), "no machinery in it: {whole}");
}

#[test]
fn a_question_that_runs_out_of_lookups_still_gets_answered() {
    use notes::llm::{Ask, Assistant, Backend, Progress, Reply, Tool, Watcher};
    /// A model that will not stop reaching for the calendar.
    ///
    /// Which is what a real one did: asked for a table of the last ten
    /// Christmases it went round the same call until the ceiling, and the
    /// person waiting got "I looked several things up and did not get to an
    /// answer" for their trouble.
    struct Greedy {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl Backend for Greedy {
        fn name(&self) -> String {
            "GREEDY".into()
        }
        fn edit(&mut self, ask: &Ask, _w: &mut dyn Watcher) -> Reply {
            // Tools taken away is the signal to answer, and it does.
            if ask.tools.is_empty() {
                return Ok("Here is the table, from what I looked up.".into());
            }
            let n = self
                .calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // A different argument each time, so it is the ceiling that stops
            // this and not the going-round-in-circles check.
            Ok(format!(
                "<tool_call><function=date><parameter=when>202{n}-12-25</parameter></function></tool_call>"
            ))
        }
    }
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut a = Assistant::spawn(Box::new(Greedy {
        calls: calls.clone(),
    }));
    a.ask(Ask {
        turns: vec![notes::llm::Turn {
            mine: true,
            text: "a table of the last ten christmases".into(),
        }],
        tools: vec![Tool {
            name: "date",
            about: "what day it is",
            takes: ("when", "a date"),
        }],
        ..Default::default()
    });
    let said = loop {
        if let Some(r) = a.poll() {
            break r.expect("answers rather than failing");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    // It stopped, it did not give up, and what it looked up on the way is
    // still in the reply.
    assert!(
        said.contains("Here is the table"),
        "answered from what it had: {said}"
    );
    assert!(!said.contains("did not get to an answer"), "{said}");
    assert!(
        said.contains("<used tool=\"date\""),
        "kept the lookups: {said}"
    );
    let n = calls.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        (6..=14).contains(&n),
        "bounded, and not at the old five: {n}"
    );
    let _ = Progress::default();
}

#[test]
fn a_reply_cannot_decide_its_own_change_or_disguise_it_as_a_call() {
    use notes::chat::{proposals, Chat};
    use notes::llm::Turn;

    // Fused: the model wrapped one of this app's change blocks in a tool
    // call's opening tag. It closes two different ways depending on the day.
    for fused in [
        "<tool_call>\n<function=write file=\"kettle.md\">\nbroken.\n</write>\n</tool_call>",
        "<tool_call>\n<function=write file=\"kettle.md\">\nbroken.\n</parameter>\n</function>\n</tool_call>",
    ] {
        let mut chat = Chat::new("home".into(), "notes.md".into());
        chat.answered(Ok(fused.into()), std::path::Path::new("/tmp"));
        let stored = &chat.turns.last().expect("a turn").text;
        let (_, changes) = proposals(stored);
        assert_eq!(changes.len(), 1, "not read as a change: {stored:?}");
        assert_eq!(changes[0].file.as_deref(), Some("kettle.md"), "{stored:?}");
        assert!(changes[0].state.is_none(), "already decided: {stored:?}");
    }

    // Decided: the model copied `state="applied"` off an earlier settled block
    // in its own history. Believing it meant no buttons were offered and the
    // file was never written - the change vanished, quietly.
    let mut chat = Chat::new("home".into(), "notes.md".into());
    chat.answered(
        Ok(
            "here you go.\n\n<write file=\"facts.md\" state=\"applied\">\nsome facts.\n</write>"
                .into(),
        ),
        std::path::Path::new("/tmp"),
    );
    let stored = &chat.turns.last().expect("a turn").text;
    assert!(
        !stored.contains("applied"),
        "kept its own verdict: {stored:?}"
    );
    let (prose, changes) = proposals(stored);
    assert_eq!(changes.len(), 1);
    assert!(
        changes[0].state.is_none(),
        "the one party that does not get a say got one: {stored:?}"
    );
    assert!(prose.contains("here you go"), "{prose:?}");

    // And a reply with nothing untoward in it is left exactly as it was.
    let plain = "just a sentence, no blocks.";
    let mut chat = Chat::new("home".into(), "notes.md".into());
    chat.answered(Ok(plain.into()), std::path::Path::new("/tmp"));
    assert_eq!(chat.turns.last().expect("a turn").text, plain);
    let _ = Turn {
        mine: true,
        text: String::new(),
    };
}

#[test]
fn every_part_of_a_multi_step_answer_is_kept() {
    use notes::llm::{Ask, Assistant, Backend, Reply, Tool, Turn, Watcher};
    /// A model answering a three-part question the way they do: a part, then
    /// a tool, then the next part.
    struct InParts(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl Backend for InParts {
        fn name(&self) -> String {
            "IN PARTS".into()
        }
        fn edit(&mut self, _ask: &Ask, _w: &mut dyn Watcher) -> Reply {
            let n = self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(match n {
                0 => "1234 * 5678 is 7006652.\n<tool_call><function=date><parameter=when>1969-07-20</parameter></function></tool_call>".into(),
                1 => "1969-07-20 was a Sunday.\n<tool_call><function=date><parameter=when>12-25</parameter></function></tool_call>".into(),
                _ => "There are 120 days until the next 25 December.".to_string(),
            })
        }
    }
    let mut a = Assistant::spawn(Box::new(InParts(std::sync::Arc::new(
        std::sync::atomic::AtomicUsize::new(0),
    ))));
    a.ask(Ask {
        turns: vec![Turn {
            mine: true,
            text: "three things, one at a time".into(),
        }],
        tools: vec![Tool {
            name: "date",
            about: "what day it is",
            takes: ("when", "a date"),
        }],
        ..Default::default()
    });
    let said = loop {
        if let Some(r) = a.poll() {
            break r.expect("an answer");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let (prose, looked) = notes::chat::lookups(&said);
    // Every part, not just the last. Before this the conversation showed
    // "There are 120 days until the next 25 December." and nothing else: the
    // first two answers were written, and thrown away with the tool call that
    // followed them.
    assert!(
        prose.contains("7006652"),
        "the first part is gone: {prose:?}"
    );
    assert!(
        prose.contains("Sunday"),
        "the second part is gone: {prose:?}"
    );
    assert!(
        prose.contains("120 days"),
        "the last part is gone: {prose:?}"
    );
    // In the order they were said.
    let (a1, a2) = (
        prose.find("7006652").unwrap(),
        prose.find("Sunday").unwrap(),
    );
    assert!(a1 < a2 && a2 < prose.find("120 days").unwrap(), "{prose:?}");
    assert_eq!(looked.len(), 2, "and what it looked up is still recorded");
}

#[test]
fn a_reply_asking_for_three_things_at_once_gets_all_three() {
    use notes::llm::calls;
    // Models batch them: given three things to find out, all three blocks
    // arrive in one reply. Reading only the first dropped the rest, and since
    // the reply was then all machinery with one call consumed, what showed up
    // in the conversation was the raw tags of the calls that never ran.
    let said = "<tool_call>\n<function=calc>\n<parameter=expression>\n384 * 517\n</parameter>\n</function>\n</tool_call>\n\
                <tool_call>\n<function=date>\n<parameter=when>\n12-25\n</parameter>\n</function>\n</tool_call>";
    let asked = calls(said);
    assert_eq!(
        asked,
        vec![
            ("calc".to_string(), "384 * 517".to_string()),
            ("date".to_string(), "12-25".to_string()),
        ],
        "both of them, in order"
    );
    // One is still one, and the first of many is still the first.
    let single =
        "<tool_call><function=date><parameter=when>today</parameter></function></tool_call>";
    assert_eq!(calls(single).len(), 1);
    assert_eq!(
        notes::llm::called(said),
        Some(("calc".into(), "384 * 517".into()))
    );
    // And a reply with no call in it asks for nothing.
    assert!(calls("just a sentence").is_empty());
}

#[test]
fn a_reply_that_was_only_machinery_says_so_rather_than_nothing() {
    use notes::llm::{Ask, Assistant, Backend, Reply, Tool, Turn, Watcher};
    /// A model whose whole reply is a call it got the shape of wrong.
    struct AllTags;
    impl Backend for AllTags {
        fn name(&self) -> String {
            "ALL TAGS".into()
        }
        fn edit(&mut self, _ask: &Ask, _w: &mut dyn Watcher) -> Reply {
            // No `<parameter=`, so it is not a call, and no change block
            // inside it either: nothing is left once the tags come off.
            Ok("<tool_call>\n<function=date>\n</function>\n</tool_call>".into())
        }
    }
    let mut a = Assistant::spawn(Box::new(AllTags));
    a.ask(Ask {
        turns: vec![Turn {
            mine: true,
            text: "write the sum into a note".into(),
        }],
        tools: vec![Tool {
            name: "calc",
            about: "sums",
            takes: ("expression", "a sum"),
        }],
        ..Default::default()
    });
    let said = loop {
        if let Some(r) = a.poll() {
            break r.expect("an answer");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let (prose, _) = notes::chat::lookups(&said);
    let (prose, changes) = notes::chat::proposals(&prose);
    // Not blank, and not tags. A turn with nothing in it reads as the
    // application having lost the answer, and the tags are the inside of the
    // thing they asked a question of. Something to show means either words or
    // a change: a reply that is only a block is not blank, it is a diff.
    assert!(
        !prose.trim().is_empty() || !changes.is_empty(),
        "a blank turn: {said:?}"
    );
    for bracket in [
        "<tool_call",
        "<function=",
        "</function",
        "</tool_call",
        "<parameter",
    ] {
        assert!(
            !prose.contains(bracket),
            "{bracket} reached the panel: {prose:?}"
        );
    }
}

#[test]
fn a_date_with_its_month_written_out_is_understood() {
    // The rule was ISO and nothing else, because 07/31 and 31/07 are the same
    // six characters meaning two different days. A month with a name is not in
    // doubt, and refusing it cost the answer: asked how many days somebody had
    // been alive from "jul 31 1989", the model could not turn that into a date
    // this would take, did the arithmetic itself, and was out by 819 days.
    let want = notes::clock::about("1989-07-31").unwrap();
    for spelling in [
        "jul 31 1989",
        "31 july 1989",
        "July 31, 1989",
        "31 Jul 1989",
        "  JULY 31 1989  ",
    ] {
        assert_eq!(
            notes::clock::about(spelling).unwrap(),
            want,
            "{spelling:?} should be the same day"
        );
    }
    // May is the awkward one: three letters long to begin with.
    assert!(notes::clock::about("may 1 2000")
        .unwrap()
        .contains("1 May 2000"));
    // Without a year it is a day that comes round, like 12-25.
    let named = notes::clock::about("25 december").unwrap();
    assert_eq!(named, notes::clock::about("12-25").unwrap());
    assert!(named.contains("The next 25 December"), "{named}");
    // And figures alone are still refused, because they are still ambiguous.
    assert!(notes::clock::about("07/31/1989").is_err());
    assert!(notes::clock::about("next tuesday").is_err());
}

#[test]
fn an_enormous_argument_is_cut_before_it_is_drawn() {
    use notes::chat::Lookup;
    // One model answered "how many days have I been alive" by reaching for the
    // calculator with four hundred ones added together. The panel draws what
    // was looked up on one line, and that is not a line.
    let huge = Lookup {
        tool: "calc".into(),
        arg: "1 + ".repeat(200) + "1",
        result: String::new(),
    };
    let said = huge.said();
    assert!(said.len() < 80, "{} characters: {said:?}", said.len());
    assert!(said.ends_with("..."), "{said:?}");
    assert!(said.starts_with("WORKED OUT 1 + 1"), "{said:?}");
    // An ordinary one is untouched.
    let small = Lookup {
        tool: "calc".into(),
        arg: "384 * 517".into(),
        result: String::new(),
    };
    assert_eq!(small.said(), "WORKED OUT 384 * 517");
}

#[test]
fn an_edit_of_no_particular_lines_is_a_write() {
    use notes::chat::proposals;
    // `edit` is the general word for changing something, and a file that does
    // not exist yet has no lines to name. Asked to make a note of four
    // birthdays, a model wrote the whole note inside `<edit file="ages.md">`
    // with no `lines`, and it was dropped for want of one: nothing offered,
    // nothing written, and the block left sitting in the prose.
    let said = "Here you go.\n\n<edit file=\"ages.md\">\n# Ages\n\n- Me: 13,541 days\n</edit>";
    let (prose, changes) = proposals(said);
    assert_eq!(changes.len(), 1, "still dropped: {prose:?}");
    assert_eq!(changes[0].file.as_deref(), Some("ages.md"));
    assert_eq!(
        changes[0].headline("notes.md"),
        "WRITE  ages.md",
        "read as laying the file down"
    );
    assert!(prose.contains("Here you go"), "{prose:?}");
    assert!(
        !prose.contains("<edit"),
        "the block is lifted out: {prose:?}"
    );

    // An edit that does name its lines is still an edit.
    let lined = "<edit file=\"ages.md\" lines=\"2-3\">\nnew text\n</edit>";
    let (_, changes) = proposals(lined);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].headline("notes.md"), "ages.md  LINES 2-3");

    // And a bare edit naming neither lines nor a file is left alone: that
    // would mean replacing the whole of the note in front of you, which is too
    // much to read into something left out.
    let bare = "<edit>\neverything\n</edit>";
    let (prose, changes) = proposals(bare);
    assert!(changes.is_empty(), "{changes:?} from {bare:?}");
    assert!(prose.contains("<edit>"), "left visible instead: {prose:?}");
}

#[test]
fn an_age_is_given_in_years_and_months_not_only_days() {
    // Told a child was 612 days old, the model divided by 365, rounded, and
    // announced she had turned two. She was one. Days are exact and useless
    // past a certain size, and whole calendar years are not something to leave
    // to a model with a division sign.
    //
    // Worked from today rather than from a date written here, so this still
    // means something next year.
    let today = notes::clock::about("today").unwrap();
    let year: i64 = today
        .split_whitespace()
        .filter_map(|w| w.trim_matches(|c: char| !c.is_ascii_digit()).parse().ok())
        .find(|n: &i64| (2000..3000).contains(n))
        .expect("a year in it");
    let rest: Vec<&str> = today.split_whitespace().collect();
    let (day, month) = (rest[1], rest[2]);
    let on = |y: i64| format!("{day} {month} {y}");

    // A year to the day is a year, with no months hanging off it.
    let last = notes::clock::about(&on(year - 1)).unwrap();
    assert!(last.contains("- 1 year."), "{last}");
    assert!(!last.contains("1 year and"), "{last}");

    // And several are several.
    let ago = notes::clock::about(&on(year - 37)).unwrap();
    assert!(ago.contains("- 37 years."), "{ago}");

    // Today and yesterday have no age to give, and do not pretend to.
    let now = notes::clock::about(&on(year)).unwrap();
    assert!(now.contains("which is today"), "{now}");
    assert!(!now.contains(" - 0 "), "{now}");
}

#[test]
fn a_change_wrapped_in_a_call_survives_the_tags_coming_off() {
    use notes::llm::without_machinery;
    // The scrub takes out a whole call, opening tag to closing one - and a
    // change block wearing a call's tags sits inside that span. Asked to add
    // two children to a note, the model looked up both their birthdays, wrote
    // the edit, wrapped the lot in a call, and every word of it was deleted as
    // wiring: two correct lookups and then "it looked that up but did not say
    // anything about it".
    let fused = "<tool_call>\n<function=edit file=\"family.md\" lines=\"1-4\">\n- **Danila**: 11 February 2020\n</edit>\n</tool_call>";
    let kept = without_machinery(fused);
    assert!(
        kept.contains("<edit file=\"family.md\""),
        "lost the block: {kept:?}"
    );
    assert!(kept.contains("Danila"), "lost the body: {kept:?}");
    assert!(!kept.contains("tool_call"), "kept the wrapping: {kept:?}");
    // And the block is then read as the change it is.
    let (_, changes) = notes::chat::proposals(&kept);
    assert_eq!(changes.len(), 1, "{kept:?}");

    // A call with nothing of ours inside it still goes entirely.
    let plain = "Here you go.\n<tool_call>\n<function=date>\n<parameter=when>today</parameter>\n</function>\n</tool_call>";
    assert_eq!(without_machinery(plain), "Here you go.");
}

#[test]
fn a_list_is_corrected_by_the_lines_that_moved() {
    use notes::digest::relisted;
    let was = "- `a.md` \"A\": the first one\n- `b.md` \"B\": the second one\n\
               - `c.md` \"C\": the third one\n- `d.md` \"D\": the fourth one\n\
               - `e.md` \"E\": the fifth one\n- `f.md` \"F\": the sixth one";

    assert!(relisted(was, was).is_none(), "nothing moved");

    // One note edited. The other five are already at the front and saying them
    // again is what put every note in the vault into the prompt twice.
    let now = was.replace("the third one", "the third one, rewritten");
    let said = relisted(was, &now).expect("something moved");
    assert!(said.contains("the third one, rewritten"), "{said}");
    assert!(!said.contains("the sixth one"), "the rest came too: {said}");
    assert_eq!(
        said.matches("- `").count(),
        1,
        "one note moved, so one line: {said}"
    );

    // A note taken away is named, not silently absent from a list that is not
    // being sent.
    let fewer = was.replace("- `b.md` \"B\": the second one\n", "");
    let said = relisted(was, &fewer).expect("something moved");
    assert!(said.contains("`b.md`"), "{said}");
    assert!(said.contains("no longer"), "{said}");

    // And a vault that has changed all over is shown, not itemised.
    let all = "- `x.md` \"X\": something else entirely\n- `y.md` \"Y\": and another";
    let said = relisted(was, all).expect("something moved");
    assert!(said.contains("It is now:"), "{said}");
}

#[test]
fn a_note_read_is_remembered_as_read_and_not_as_its_contents() {
    use notes::chat::without_bodies;
    let long = (1..=40)
        .map(|n| format!("  {n:3} | line {n} of a note that is not short\n"))
        .collect::<String>();
    let said = format!(
        "<used tool=\"read\" arg=\"long.md\">\n`long.md` says, as of now:\n\n{long}</used>\n\nIt is long.",
    );
    let sent = without_bodies(&said);
    assert!(sent.contains("[you read `long.md` here]"), "{sent}");
    assert!(sent.contains("It is long."), "what it said is kept: {sent}");
    assert!(
        !sent.contains("line 20 of a note"),
        "the whole file is still being sent back: {sent}"
    );
    assert!(sent.len() < said.len() / 4, "no smaller: {}", sent.len());

    // A sum or a date is one line and is worth keeping - it was looked up
    // once, it cannot go stale, and what it cost to keep is nothing.
    let sum = "<used tool=\"calc\" arg=\"384 * 517\">\n198528\n</used>\n\nIt is 198528.";
    assert!(without_bodies(sum).contains("198528"));
}

#[test]
fn a_block_nested_in_an_unclosed_one_is_still_found() {
    use notes::chat::{proposals, What};
    // Word for word out of a run: the example tag from the instructions,
    // copied, never closed, with the real change written inside it.
    let said = "<edit file=\"notes.md\" lines=\"12-14\">\n\
                <write file=\"ages.md\"># Ages\n\n- Eva: 613 days\n</write>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 1, "the write was thrown away: {changes:?}");
    assert_eq!(changes[0].file.as_deref(), Some("ages.md"));
    match &changes[0].what {
        What::Write { text } => assert!(text.contains("613 days"), "{text}"),
        other => panic!("read as {other:?}"),
    }

    // And an opener on its own still proposes nothing at all.
    assert!(proposals("<write file=\"a.md\">never finished")
        .1
        .is_empty());
}

#[test]
fn what_a_tool_answered_is_quoted_and_not_proposed() {
    use notes::chat::{proposals, without_bodies};
    // Straight out of a real conversation. The model tried to call `write` as
    // though it were a tool; it was told the shape that does work, and that
    // answer has a block written out in it. Then it wrote the real one.
    let said = "<used tool=\"write\" arg=\"bike.md\">\nwrite is not a tool. Changing a file \
                is not something you call: write a <write> block in your reply instead. \
                Nothing happens to the file until they accept it.\n</used>\n\nMade it.\n\
                <write file=\"bike.md\">The bike is red.</write>";

    // One change, and it is the one it actually proposed.
    let (prose, changes) = proposals(said);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(prose.contains("Made it."), "{prose}");

    // And sending the turn back leaves the tool's answer alone. It used to eat
    // the `</used>` and come back as two changes and half a sentence, so the
    // conversation carried a proposal that was never made.
    let sent = without_bodies(said);
    assert!(sent.contains("</used>"), "the answer was cut into: {sent}");
    assert_eq!(
        sent.matches("write to `bike.md`").count(),
        1,
        "one change was proposed, not two: {sent}"
    );
    assert!(
        !sent.contains("`the note`"),
        "the block inside the answer was read as a change: {sent}"
    );

    // A note that talks about the machinery is quoted the same way. Reading one
    // back must not be a way of getting a change made.
    let read = "<used tool=\"read\" arg=\"how-to.md\">\n`how-to.md` says, as of now:\n\n\
                   1 | Write <delete file=\"wanted.md\"></delete> to remove one.\n</used>\n\nThat is how.";
    assert!(
        proposals(read).1.is_empty(),
        "a note quoted itself into a change"
    );
}

#[test]
fn a_list_that_moves_on_its_own_is_said_at_the_end_and_not_at_the_front() {
    use notes::chat::Chat;
    let file = |n: &str, t: &str| (n.to_string(), t.to_string());
    let big = "a line that is here to take up room\n".repeat(60);
    let files = vec![file("notes.md", &format!("# Notes\n\n{big}"))];
    let mut chat = Chat::new("home".into(), "notes.md".into());

    let (shown, front, moved) = chat.context("- `notes.md` \"Notes\"", &files);
    assert!(moved.is_none(), "nothing has moved yet");

    // A note in another project got its first line changed, so the list of the
    // vault reads differently while nothing in this project moved at all.
    let listed = "- `notes.md` \"Notes\"\n- `other.md` \"Other\"";
    let (again, still, moved) = chat.context(listed, &files);
    assert_eq!(again, shown, "the list at the front is not rewritten");
    assert_eq!(still, front, "and neither is the project");
    let said = moved.expect("the list that moved is said at the end");
    assert!(said.contains("other.md"), "{said}");

    // Never both. Rewriting the front empties the cache, and a front that has
    // been rewritten must not then be called out of date - that was costing
    // the whole prompt again and saying the new list twice.
    assert!(!again.contains("other.md"), "{again}");

    // And said once. The front stays as it was, but the model has been told,
    // and telling it again every turn afterwards is how a conversation ends up
    // reading as a standing instruction that something needs doing about it.
    let (third, _, moved) = chat.context(listed, &files);
    assert_eq!(third, shown, "the front still does not move");
    assert!(moved.is_none(), "it has already been told: {moved:?}");
}

#[test]
fn the_project_is_written_out_once_and_corrected_after() {
    use notes::chat::Chat;
    let file = |n: &str, t: &str| (n.to_string(), t.to_string());
    let big = "a line that is here to take up room\n".repeat(60);
    let one = |tail: &str| {
        vec![
            file("notes.md", &format!("# Notes\n\n{big}{tail}")),
            file("plans.md", "# Plans\n\nBuy a bicycle.\n"),
        ]
    };
    let mut chat = Chat::new("home".into(), "notes.md".into());

    // First time: written out, nothing to correct.
    let (_, first, moved) = chat.context("- `notes.md`", &one("the tap drips\n"));
    assert!(first.contains("# Notes") && first.contains("# Plans"));
    assert!(moved.is_none(), "nothing has moved yet");

    // Nothing changed: the same text, to the byte, so it stays in the cache.
    let (_, again, moved) = chat.context("- `notes.md`", &one("the tap drips\n"));
    assert_eq!(again, first, "the project must not be rewritten");
    assert!(moved.is_none());

    // A line changed - by this panel or by somebody editing the file in
    // another window, which is the same thing from here. The project is still
    // the same text; what moved is said separately.
    let (_, still, moved) = chat.context("- `notes.md`", &one("the tap was fixed\n"));
    assert_eq!(still, first, "the project must still not be rewritten");
    let moved = moved.expect("the change is reported");
    assert!(moved.contains("notes.md"), "{moved}");
    assert!(moved.contains("the tap was fixed"), "{moved}");
    assert!(
        !moved.contains("the tap drips"),
        "the old line is not repeated"
    );
    assert!(
        moved.len() < first.len() / 4,
        "a one line change should be small beside the project: {} vs {}",
        moved.len(),
        first.len()
    );

    // A new file, and a file taken away.
    let (_, _, moved) = chat.context(
        "- `notes.md`",
        &[
            file("notes.md", &format!("# Notes\n\n{big}the tap drips\n")),
            file("later.md", "# Later\n\nSomething else.\n"),
        ],
    );
    let moved = moved.expect("both are reported");
    assert!(moved.contains("`later.md` now contains"), "{moved}");
    assert!(moved.contains("`plans.md` is gone"), "{moved}");

    // And when the corrections grow past being worth it, the project is
    // written out again - but what moved is still said at the end, because
    // that is the part that gets noticed. A file rewritten at the front sits
    // behind every turn of the conversation, and a conversation that has been
    // saying one thing for six turns drowns it.
    let (_, fresh, moved) = chat.context(
        "- `notes.md`",
        &[file("notes.md", "# Notes\n\nAll of it, different.\n")],
    );
    assert!(fresh.contains("All of it, different"), "{fresh}");
    assert!(
        !fresh.contains("Buy a bicycle"),
        "the old project is gone: {fresh}"
    );
    let moved = moved.expect("still told what moved");
    assert!(moved.contains("notes.md"), "{moved}");
    // ...and the new text is now what it compares against, so once it settles
    // there is nothing left to say.
    let (_, _, moved) = chat.context(
        "- `notes.md`",
        &[file("notes.md", "# Notes\n\nAll of it, different.\n")],
    );
    assert!(moved.is_none(), "nothing has moved since the rewrite");
}

#[test]
fn a_conversation_reopened_is_shown_the_project_afresh() {
    use notes::chat::Chat;
    // What a chat remembers about the project is what it sent, and it sends
    // nothing until it is asked to. So one opened tomorrow - or one whose
    // notes were edited while it was closed - writes the whole thing out
    // again, which is right and is the only safe answer: it cannot know what
    // the model was told last time.
    let files = vec![(
        "notes.md".to_string(),
        "# Notes\n\nthe tap drips\n".to_string(),
    )];
    let mut chat = Chat::new("home".into(), "notes.md".into());
    let (_, _, moved) = chat.context("- `notes.md`", &files);
    assert!(moved.is_none());
    let mut reopened = Chat::new("home".into(), "notes.md".into());
    let (_, whole, moved) = reopened.context("- `notes.md`", &files);
    assert!(moved.is_none(), "nothing to correct against nothing shown");
    assert!(whole.contains("the tap drips"), "the whole of it: {whole}");
}

#[test]
fn a_note_changed_by_something_else_is_taken_up() {
    // The vault was read once at startup and never again, so a note edited in
    // another window stayed as it was everywhere it mattered: the editor, the
    // sidebar, and the project the assistant is shown. Asked what colour the
    // bike was, it said red about a file that had said green for ten minutes.
    let dir = std::env::temp_dir().join(format!("notes-outside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a vault");
    std::fs::write(dir.join("bike.md"), "# Bike\n\nThe bike is red.\n").expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    let text = |app: &notes::Notes| {
        app.notes
            .iter()
            .find(|n| n.filename() == "bike.md")
            .map(|n| n.buffer.to_text())
            .unwrap_or_default()
    };
    assert!(text(&app).contains("red"), "{:?}", text(&app));

    // Somebody else writes it. The stamp has to move, and a filesystem that
    // keeps whole seconds would not notice two writes in the same one.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dir.join("bike.md"), "# Bike\n\nThe bike is green.\n").expect("rewritten");
    assert!(app.take_up_changes() > 0, "nothing noticed");
    assert!(text(&app).contains("green"), "still: {:?}", text(&app));

    // A note appearing is picked up, and one taken away is let go of.
    std::fs::write(dir.join("kettle.md"), "# Kettle\n").expect("a new note");
    app.take_up_changes();
    assert!(
        app.notes.iter().any(|n| n.filename() == "kettle.md"),
        "the new one"
    );
    std::fs::remove_file(dir.join("kettle.md")).expect("gone");
    app.take_up_changes();
    assert!(
        !app.notes.iter().any(|n| n.filename() == "kettle.md"),
        "the gone one"
    );

    // Somebody who saves a file meant to, and that holds even over a buffer
    // here that has not been saved yet: it is the same person, and saving the
    // file is the thing they did on purpose. Waiting for the pause before a
    // save is the reason the two can disagree at all.
    let i = app
        .notes
        .iter()
        .position(|n| n.filename() == "bike.md")
        .expect("there");
    app.notes[i].buffer = notes::text::Buffer::from_text("# Bike\n\nmine, unsaved\n");
    app.notes[i].buffer.dirty = true;
    std::thread::sleep(std::time::Duration::from_millis(1100));
    std::fs::write(dir.join("bike.md"), "# Bike\n\ntheirs\n").expect("rewritten again");
    assert!(app.take_up_changes() > 0, "the save was passed over");
    assert!(
        text(&app).contains("theirs"),
        "the file did not win: {:?}",
        text(&app)
    );
    // And nothing is lost by it - only moved one keystroke away.
    let i = app
        .notes
        .iter()
        .position(|n| n.filename() == "bike.md")
        .expect("there");
    app.notes[i].buffer.undo();
    assert!(
        app.notes[i].buffer.to_text().contains("mine, unsaved"),
        "undo should put back what was typed: {:?}",
        app.notes[i].buffer.to_text()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_note_only_just_made_is_not_mistaken_for_one_deleted() {
    // Taking up what changed on disk also lets go of files that have gone -
    // and a note the assistant has just made has not gone, it has not arrived
    // yet. Waiting for the save that follows a pause is the whole of the
    // difference, and for a second or so it looks exactly like a deletion.
    // Every file the assistant created was thrown away in that second.
    let dir = std::env::temp_dir().join(format!("notes-newborn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a vault");
    std::fs::write(dir.join("one.md"), "# One\n").expect("a note");
    let mut app = notes::Notes::open(dir.clone());

    // What accepting a change leaves behind: named, unsaved, not on disk.
    app.apply_change(&notes::chat::Change {
        file: Some("kettle.md".into()),
        what: notes::chat::What::Write {
            text: "# Kettle\n\nThe kettle is broken.\n".into(),
        },
        state: None,
    });
    assert!(
        app.notes.iter().any(|n| n.filename() == "kettle.md"),
        "made"
    );
    assert!(!dir.join("kettle.md").exists(), "and not on disk yet");

    app.take_up_changes();
    assert!(
        app.notes.iter().any(|n| n.filename() == "kettle.md"),
        "the new note was thrown away before it was saved"
    );
    // And it reaches the disk when it is meant to.
    app.before_asking();
    assert!(dir.join("kettle.md").exists(), "still not written");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_named_with_its_folder_still_lands_in_the_folder() {
    // The instructions ask for the name on its own, and models mostly give it.
    // Not always: with a project open, one answered
    // `<write file="new-one/bike.md">` - the project's own name on the front.
    // Joined to the project's folder a second time, that is bound for
    // `new-one/new-one/bike.md`, which does not exist. The write failed with
    // "no such file or directory", quietly, and the note sat in the editor
    // marked unsaved forever, about a file that was nowhere.
    let dir = std::env::temp_dir().join(format!("notes-folded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("new-one")).expect("a project");
    std::fs::write(dir.join("new-one/seed.md"), "# Seed\n").expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    let i = app
        .notes
        .iter()
        .position(|n| n.filename() == "seed.md")
        .expect("there");
    app.current = i;

    app.apply_change(&notes::chat::Change {
        file: Some("new-one/bike.md".into()),
        what: notes::chat::What::Write {
            text: "# Bike\n\nThe bike is red.\n".into(),
        },
        state: None,
    });
    assert_eq!(app.keep_up(), 1, "the write did not happen");
    assert!(
        dir.join("new-one/bike.md").exists(),
        "not where it belongs. vault holds: {:?}",
        std::fs::read_dir(dir.join("new-one"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
    assert!(
        !dir.join("new-one/new-one").exists(),
        "a folder inside itself"
    );
    // And nothing is left claiming to be unsaved.
    assert!(
        !app.notes.iter().any(|n| n.buffer.dirty),
        "still marked unsaved after being written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_change_it_proposed_is_not_sent_back_as_a_copy_of_the_file() {
    use notes::chat::without_bodies;
    // A change block is a copy of a file, and a copy goes stale. Left in the
    // conversation it is worse than stale: the model wrote it, so when the
    // file later says something else it has its own word against a correction
    // and takes its own. Reported exactly so - a change to a note the model
    // itself wrote was never believed, while the same change to a note that
    // was already there was believed at once, and a conversation started
    // fresh got it right, having nothing of its own to argue with.
    let said = "Here you go.\n\n<write file=\"bike.md\" state=\"applied\">\n# Bike\n\nThe bike is red.\n</write>\n\nAnything else?";
    let sent = without_bodies(said);
    assert!(sent.contains("Here you go."), "{sent}");
    assert!(sent.contains("Anything else?"), "{sent}");
    assert!(
        sent.contains("bike.md"),
        "it should still know what it did: {sent}"
    );
    assert!(sent.contains("accepted"), "and what became of it: {sent}");
    // The part that matters: no copy of the file.
    assert!(
        !sent.contains("The bike is red"),
        "the stale copy went too: {sent}"
    );
    assert!(!sent.contains("<write"), "and the block with it: {sent}");

    // A change still waiting, and one turned down, say so.
    let waiting = "<edit file=\"a.md\" lines=\"1\">\nnew text\n</edit>";
    assert!(without_bodies(waiting).contains("waiting"), "{waiting}");
    let turned = "<delete file=\"a.md\" state=\"rejected\"></delete>";
    assert!(without_bodies(turned).contains("turned down"));

    // A reply with no block in it is untouched.
    let plain = "The bike is red.";
    assert_eq!(without_bodies(plain), plain);
}

#[test]
fn a_one_line_change_stays_a_one_line_change() {
    use notes::chat::Chat;
    // A note first mentioned in a correction used to be new every time after
    // that, because corrections were measured against what was written at the
    // front and it was never there. So a one-line change to a long note
    // re-sent the whole note, and went on re-sending it for the rest of the
    // conversation - which is the opposite of the point.
    let long = (1..=400)
        .map(|i| format!("line {i} of a note that is quite long"))
        .collect::<Vec<_>>()
        .join("\n");
    let file = |t: &str| vec![("big.md".to_string(), t.to_string())];
    let mut chat = Chat::new("home".into(), "big.md".into());

    let (_, whole, _) = chat.context("- `big.md`", &file(&long));
    assert!(whole.len() > 10_000, "a long note to start from");

    // One line changes. What is said about it is a line, not a note.
    let once = long.replacen("line 200 of", "line 200 CHANGED of", 1);
    let (_, front, moved) = chat.context("- `big.md`", &file(&once));
    assert_eq!(front, whole, "the front must not move");
    let first = moved.expect("the change is reported");
    assert!(first.contains("CHANGED"), "{first}");
    assert!(
        first.len() < whole.len() / 10,
        "the whole note came back: {} against {}",
        first.len(),
        whole.len()
    );

    // And a second change is measured from the first, not from the front.
    let twice = once.replacen("line 300 of", "line 300 ALSO of", 1);
    let (_, front, moved) = chat.context("- `big.md`", &file(&twice));
    assert_eq!(front, whole, "still must not move");
    let second = moved.expect("the second change is reported");
    assert!(second.contains("ALSO"), "{second}");
    assert!(
        !second.contains("CHANGED"),
        "it repeated a change already told: {second}"
    );
    assert!(second.len() < whole.len() / 10, "{}", second.len());

    // Nothing moving says nothing.
    let (_, _, moved) = chat.context("- `big.md`", &file(&twice));
    assert!(moved.is_none());
}

#[test]
fn asking_to_read_a_note_is_heard_in_either_shape() {
    use notes::llm::calls;
    // Told a file had changed and given a tool to read it with, the model
    // wrote `<read file="bike.md"></read>` - not a tool call, but the shape of
    // the edit and write blocks it had been taught three paragraphs earlier,
    // which is a reasonable thing to conclude from that prompt. Nothing
    // understood it, so it went unanswered and the model fell back on what it
    // already believed. Answered in the shape it asked, it gets it right.
    let block = "I should check.\n<read file=\"bike.md\"></read>";
    assert_eq!(
        calls(block),
        vec![("read".to_string(), "bike.md".to_string())]
    );

    // The proper call still works, and so does one of each.
    let proper =
        "<tool_call><function=read><parameter=file>bike.md</parameter></function></tool_call>";
    assert_eq!(
        calls(proper),
        vec![("read".to_string(), "bike.md".to_string())]
    );
    let both = format!("{proper}\n<read file=\"other.md\"></read>");
    assert_eq!(calls(&both).len(), 2, "{both}");

    // And the asking never reaches the panel as words.
    assert_eq!(notes::llm::without_machinery(block), "I should check.");

    // A note that is not there says so rather than saying nothing.
    let missing = notes::tools::run("read", "nowhere-at-all.md");
    assert!(missing.contains("no note called"), "{missing}");
}

#[test]
fn a_block_whose_attributes_came_as_parameters_is_still_a_block() {
    use notes::chat::{proposals, Chat};
    // The third shape of the same confusion, and the one that turns up once
    // the model has both blocks and tools well in mind: the name and the body
    // are there, wearing a call's tags, and the closing tag has wandered into
    // the middle. It was thrown away, so the change was never offered and
    // nothing was written.
    let said = "<write>\n<parameter=file>\nfacts.md\n</write>\n<parameter=content>\n384 * 517 = 198528\n\nChristmas this year is on a Friday.";
    let mut chat = Chat::new("home".into(), "notes.md".into());
    chat.answered(Ok(said.into()), std::path::Path::new("/tmp"));
    let stored = &chat.turns.last().expect("a turn").text;
    let (_, changes) = proposals(stored);
    assert_eq!(changes.len(), 1, "still dropped: {stored:?}");
    assert_eq!(changes[0].file.as_deref(), Some("facts.md"));
    assert_eq!(changes[0].headline("notes.md"), "WRITE  facts.md");
    match &changes[0].what {
        notes::chat::What::Write { text } => {
            assert!(text.contains("198528"), "{text:?}");
            assert!(text.contains("Friday"), "{text:?}");
            assert!(!text.contains("parameter"), "wiring in the body: {text:?}");
        }
        other => panic!("read as {other:?}"),
    }
}
