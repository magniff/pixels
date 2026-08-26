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
            &mut |_| {},
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

    // Before a word has come back, what there is to report is the question.
    block.progress = Progress {
        prompt: 412,
        ..Progress::default()
    };
    assert_eq!(block.headline(), "READING 412 TOKENS");

    // A reasoning model is thinking, and says so rather than looking slow.
    block.progress = Progress {
        prompt: 412,
        written: 90,
        elapsed: std::time::Duration::from_secs(4),
        generating: std::time::Duration::from_secs(3),
        deliberating: true,
    };
    assert_eq!(block.headline(), "THINKING - 90 TOKENS AT 30/S");

    block.progress.deliberating = false;
    assert_eq!(block.headline(), "WRITING - 90 TOKENS AT 30/S");
}

#[test]
fn a_question_in_flight_reports_where_it_has_got_to() {
    use notes::llm::{Ask, Backend, Progress};
    let mut stub = notes::llm::Rehearsal;
    let mut seen: Vec<Progress> = Vec::new();
    let _ = stub.edit(
        &Ask {
            source: "teh quick fox".into(),
            request: "fix it".into(),
            ..Default::default()
        },
        &mut |p| seen.push(p),
    );
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
            &mut |_| {},
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
        model: Some("Qwen3-4B-Instruct-2507-Q4_K_M.gguf".into()),
        // A prompt has newlines in it, and the format is one line per setting.
        prompt: "first line\nsecond line\nand a backslash \\ too".into(),
    };
    let text = config.to_text();
    assert_eq!(text.lines().count(), 5, "one line per setting");
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
    let mut talk = chat::Chat::new("welcome.md".into());
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
    let mut talk = chat::Chat::new("n.md".into());
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

    let mut talk = chat::Chat::new("welcome.md".into());
    talk.turns = vec![Turn {
        mine: true,
        text: "anything at all".into(),
    }];
    talk.save(&dir).expect("it saves");

    let path = talk.path.clone().expect("named on the way out");
    assert!(
        path.starts_with(dir.join(".chats").join("welcome")),
        "filed under the note, in a folder the forest does not count: {path:?}"
    );
    assert_eq!(
        notes::read_vault(&dir).len(),
        1,
        "the vault is still one note - a conversation is not one of them"
    );
    let listed = chat::filed(&dir, "welcome.md");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].turns, 1);
    assert_eq!(listed[0].title, "anything at all");
    assert!(
        chat::filed(&dir, "other.md").is_empty(),
        "and they belong to one note"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_conversation_is_not_filed_at_all() {
    let dir = std::env::temp_dir().join(format!("pixui-chat-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut talk = chat::Chat::new("welcome.md".into());
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

    let mut talk = chat::Chat::new("welcome.md".into());
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

    let back = chat::Chat::open(talk.path.as_ref().unwrap(), "welcome.md".into());
    assert_eq!(
        back.title(),
        "wrapping notes",
        "and it is still called that tomorrow"
    );
    assert_eq!(
        chat::filed(&dir, "welcome.md")[0].title,
        "wrapping notes",
        "in the list too"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_line_that_is_not_a_command_is_a_question() {
    let mut talk = chat::Chat::new("n.md".into());
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
        let mut talk = chat::Chat::new("n.md".into());
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
    let mut talk = chat::Chat::new("n.md".into());
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
    let mut talk = chat::Chat::new("welcome.md".into());
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
    let back = chat::Chat::open(talk.path.as_ref().unwrap(), "welcome.md".into());
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
    let mut talk = chat::Chat::new("n.md".into());
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
    let mut talk = chat::Chat::new("n.md".into());
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
    let mut talk = chat::Chat::new("n.md".into());
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
