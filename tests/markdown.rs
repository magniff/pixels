//! Markdown as this editor reads it: the parser, the highlighter, wrapping,
//! and what the preview makes of each construct.

use notes::markdown::{self, Tok};
use notes::markdown::{Block, CellAlign, Marker};

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

/// The token each character of a rendered paragraph carries, as one string per
/// span, so a test can say what was emphasised without counting spans.
fn toks(spans: &[notes::markdown::Span]) -> Vec<(String, notes::markdown::Tok, bool)> {
    spans
        .iter()
        .map(|s| (s.text.clone(), s.tok, s.bold))
        .collect()
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
fn markup_inside_a_code_span_is_literal() {
    use notes::markdown::Tok;
    let spans = para("`**not bold**`");
    assert_eq!(toks(&spans), [("**not bold**".into(), Tok::Code, false)]);
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
fn a_hard_break_ends_a_wrapped_row() {
    let rows = notes::markdown::wrap_ranges("ab\ncd", 40);
    assert_eq!(
        rows,
        vec![(0, 2), (3, 5)],
        "and the newline itself is drawn by nobody"
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
