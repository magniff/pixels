//! The tools a conversation can reach for, and the digest of the vault it is
//! handed: the clock, the calculator, the diff.

use notes::digest;
use notes::text::{Buffer, Cursor};

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
fn a_tool_that_fails_says_so_rather_than_saying_nothing() {
    // The one answer that reliably makes a model invent is no answer at all:
    // handed an empty result it fills the gap, and it filled one with a
    // llama.cpp version that has never existed.
    let said = notes::tools::run("no-such-tool", "anything", "");
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
    // And the three that look through the vault without changing it.
    for want in ["find", "grep", "diff"] {
        assert!(offline.contains(&want), "{offline:?}");
    }
    assert_eq!(
        offline.len(),
        6,
        "these are the six that need nobody's permission: {offline:?}"
    );
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
fn reading_a_note_by_name_prefers_the_project_in_question() {
    let dir = std::env::temp_dir().join(format!("notes-readhere-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (project, text) in [
        ("one", "# One\n\nfrom one\n"),
        ("two", "# Two\n\nfrom two\n"),
    ] {
        std::fs::create_dir_all(dir.join(project)).expect("a project");
        std::fs::write(dir.join(project).join("notes.md"), text).expect("a note");
    }
    // The tool reads the vault the application is pointed at.
    let was = std::env::var_os("PIXUI_NOTES_DIR");
    std::env::set_var("PIXUI_NOTES_DIR", &dir);
    let from_one = notes::tools::run("read", "notes.md", "one/other.md");
    let from_two = notes::tools::run("read", "notes.md", "two/other.md");
    match was {
        Some(v) => std::env::set_var("PIXUI_NOTES_DIR", v),
        None => std::env::remove_var("PIXUI_NOTES_DIR"),
    }
    let _ = std::fs::remove_dir_all(&dir);
    // Two notes of one name, and the one read is the one in the project the
    // question is about. It used to be whichever the vault listed first.
    assert!(from_one.contains("from one"), "{from_one}");
    assert!(from_two.contains("from two"), "{from_two}");
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
fn the_vault_can_be_searched_read_only() {
    use notes::tools::{diff_in, find_in, grep_in};
    let dir = std::env::temp_dir().join(format!("notes-vaulttools-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("aquarium")).expect("a project");
    std::fs::write(
        dir.join("aquarium/stock.md"),
        "# Stock\n\n- 12 ember tetras\n- 3 otocinclus\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("aquarium/water.md"),
        "# Water\n\nNitrate under 20.\n",
    )
    .unwrap();
    // Long enough that what differs is said as a diff rather than the file.
    let room = "A line about the tank that does not change.\n".repeat(20);
    std::fs::write(
        dir.join("draft.md"),
        format!("# Plan\n\n{room}Buy tetras.\nFeed them.\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("final.md"),
        format!("# Plan\n\n{room}Buy tetras.\nFeed them twice.\n"),
    )
    .unwrap();

    // find: by part of a name or a folder, without .md, case not minded.
    let found = find_in(&dir, "STOCK").unwrap();
    assert!(found.contains("`aquarium/stock.md` \"Stock\""), "{found}");
    assert!(!found.contains("water"), "{found}");
    let folder = find_in(&dir, "aquarium/").unwrap();
    assert!(
        folder.contains("stock.md") && folder.contains("water.md"),
        "{folder}"
    );
    assert!(find_in(&dir, "nothing-like-this")
        .unwrap()
        .starts_with("no note"));

    // grep: every line, with where it is, and a word off the first line found.
    let hits = grep_in(&dir, "tetras").unwrap();
    assert!(
        hits.contains("aquarium/stock.md:3: - 12 ember tetras"),
        "{hits}"
    );
    assert!(hits.contains("draft.md:23: Buy tetras."), "{hits}");
    assert!(hits.starts_with("3 lines say that"), "{hits}");
    assert!(grep_in(&dir, "zebra")
        .unwrap()
        .starts_with("nothing in the vault"));
    assert!(grep_in(&dir, "  ").is_err());

    // diff: the lines that differ, as a diff, first to second.
    let d = diff_in(&dir, "draft.md final.md").unwrap();
    assert!(
        d.contains("-Feed them.") && d.contains("+Feed them twice."),
        "{d}"
    );
    assert!(!d.contains("# Plan"), "the whole note came: {d}");
    let same = diff_in(&dir, "draft.md, draft.md").unwrap();
    assert!(same.contains("say the same thing"), "{same}");
    assert!(diff_in(&dir, "draft.md").is_err());
    assert!(diff_in(&dir, "draft.md missing.md").is_err());

    // And they are on offer, always: nothing here leaves the machine.
    let names: Vec<&str> = notes::tools::available(false)
        .iter()
        .map(|t| t.name)
        .collect();
    for want in ["find", "grep", "diff"] {
        assert!(names.contains(&want), "{names:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
