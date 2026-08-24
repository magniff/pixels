//! Tests for the parts of the notes app that are pure logic: the vim grammar,
//! the markdown highlighter, and line wrapping.
//!
//! None of this needs a window, which is the point — the editing model was kept
//! separate from the drawing so that `dw` can be asserted on rather than
//! eyeballed.

use pixui::{Key, Mods};
use pixui_notes::markdown::{self, Tok};
use pixui_notes::text::{Buffer, Cursor};
use pixui_notes::vim::{self, Mode, Selection, Vim, VimEvent, VisualKind};

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
    assert_eq!(markdown::derive_title(&lines), "Real Title");

    let lines: Vec<String> = vec!["just prose".into()];
    assert_eq!(markdown::derive_title(&lines), "just prose");
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
    let config = pixui_notes::config();
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
