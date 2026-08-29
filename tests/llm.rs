//! Talking to a model: the shapes a request and a reply take, the dialects,
//! the loop that runs tools, and what comes off a reply before anybody sees it.

use notes::chat;

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

#[test]
fn tools_are_declared_the_way_the_model_was_trained_to_read_them() {
    let declared = notes::llm::declare(&notes::tools::available(true), notes::llm::Dialect::Qwen);
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
    // No argument is still a call - to nothing, which the tool will say. It
    // used to be no call at all, and a reply that was only that came out
    // as "it did not answer" with the model never told what was missing.
    assert_eq!(
        notes::llm::called("<function=weather></function>"),
        Some(("weather".to_string(), String::new()))
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
fn the_correction_stays_on_the_question_when_a_tool_is_used() {
    use notes::llm::{Ask, Assistant, Backend, Reply, Tool, Turn, Watcher};
    use std::sync::{Arc, Mutex};
    /// Remembers every conversation it was handed, and reaches for a tool the
    /// first time only.
    struct Noting(Arc<Mutex<Vec<Ask>>>);
    impl Backend for Noting {
        fn name(&self) -> String {
            "NOTING".into()
        }
        fn edit(&mut self, ask: &Ask, _w: &mut dyn Watcher) -> Reply {
            let mut seen = self.0.lock().unwrap();
            seen.push(ask.clone());
            Ok(if seen.len() == 1 {
                "<tool_call><function=date><parameter=when>today</parameter></function></tool_call>"
                    .into()
            } else {
                "Green.".into()
            })
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut a = Assistant::spawn(Box::new(Noting(seen.clone())));
    a.ask(Ask {
        turns: vec![Turn {
            mine: true,
            text: "what colour is the bike".into(),
        }],
        since: Some("STOP. bike.md now says green.".into()),
        tools: vec![Tool {
            name: "date",
            about: "the day",
            takes: ("when", "a date"),
        }],
        ..Default::default()
    });
    loop {
        if let Some(r) = a.poll() {
            r.expect("an answer");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "one call, then the answer");

    // The first pass: the correction is in front of the question.
    let first = &seen[0].turns;
    assert!(first[0].text.starts_with("STOP."), "{}", first[0].text);
    assert!(first[0].text.ends_with("what colour is the bike"));

    // The second: the question still carries it, and the tool's answer -
    // which is now the last turn - does not. It used to move: "STOP, the
    // files have changed" was stapled to a tool response, and the question
    // it had been in front of a moment before had lost it, so the turn that
    // had just been read from the cache no longer matched.
    let second = &seen[1].turns;
    assert_eq!(second[0].text, first[0].text, "the question turn changed");
    let last = second.last().unwrap();
    assert!(
        last.mine && last.text.contains("<tool_response>"),
        "{}",
        last.text
    );
    assert!(
        !last.text.contains("STOP."),
        "moved onto the tool response: {}",
        last.text
    );

    // And no backend is left to place it a second time.
    assert!(seen.iter().all(|a| a.since.is_none()));
}

#[test]
fn a_conversation_that_has_outgrown_the_room_is_read_from_its_newest_turns() {
    use notes::llm::{fitted, Turn};
    let turn = |mine: bool, text: &str| Turn {
        mine,
        text: text.to_string(),
    };
    let turns: Vec<Turn> = (0..10)
        .map(|i| turn(i % 2 == 0, &format!("turn {i} {}", "x".repeat(50))))
        .collect();
    // Measured in characters, which is a stand-in for tokens and as good.
    let size = |turns: &[Turn]| turns.iter().map(|t| t.text.len()).sum::<usize>();

    // Room for all of it: nothing touched, not even a note saying so.
    let kept = fitted(&turns, 10_000, size);
    assert_eq!(kept, turns);

    // Room for about four. The oldest go, two at a time, and what is left
    // still opens with a question - and says that it is not the whole story.
    let kept = fitted(&turns, 330, size);
    assert!(kept.len() < turns.len(), "nothing was let go of");
    assert!(kept[0].mine, "the kept part must open with a question");
    assert!(
        kept[0].text.starts_with("[Earlier turns"),
        "{}",
        kept[0].text
    );
    assert!(kept[0].text.contains("turn 6"), "{}", kept[0].text);
    assert_eq!(
        kept.last(),
        turns.last(),
        "the newest turn is never let go of"
    );
    assert!(size(&kept) <= 330 + 80, "still over: {}", size(&kept));

    // A question that does not fit on its own is still sent, and on its own.
    let kept = fitted(&turns, 10, size);
    assert_eq!(kept.len(), 1);
    assert!(kept[0].text.ends_with(&turns[9].text));
}

#[test]
fn letters_in_any_alphabet_survive_the_fold_to_ascii() {
    use notes::llm::to_ascii;
    // The punctuation with an ASCII spelling gets it, as before.
    assert_eq!(
        to_ascii("well \u{2014} yes \u{201c}so\u{201d}"),
        "well -- yes \"so\""
    );
    // Decoration goes. A party emoji is not something a 5x7 font has words for.
    assert_eq!(
        to_ascii("done \u{1f389} at last \u{2192} next"),
        "done at last next"
    );
    // Letters stay, whatever alphabet they are in. They used to go with the
    // emoji, and a line of a note copied into an edit came back with its
    // name misspelt and was written to disk that way.
    assert_eq!(
        to_ascii("M\u{fc}ller drinks caf\u{e9}"),
        "M\u{fc}ller drinks caf\u{e9}"
    );
    assert_eq!(
        to_ascii("\u{414}\u{430}\u{43d}\u{438}\u{43b}\u{430}"),
        "\u{414}\u{430}\u{43d}\u{438}\u{43b}\u{430}"
    );
}

#[test]
fn an_answer_that_is_code_keeps_its_fence() {
    use notes::llm::{clean_reply, without_thinking};
    let code = "```rust\nfn main() {}\n```";
    // A passage handed back is unwrapped: the fence was never asked for.
    assert_eq!(clean_reply(code), "fn main() {}");
    // An answer in a conversation is not: somebody asked for code, and the
    // fence is what makes it code on the page.
    assert_eq!(without_thinking(code), code);
    // Both take the deliberation off.
    let thought = "<think>\nhmm\n</think>\n\nThe bike is red.";
    assert_eq!(without_thinking(thought), "The bike is red.");
    assert_eq!(clean_reply(thought), "The bike is red.");
}

#[test]
fn the_panel_says_what_is_actually_happening() {
    use notes::chat::doing;
    use notes::llm::Progress;
    let p = Progress::default;
    // Sent, and nothing heard back yet: not reading anything.
    assert_eq!(doing(&p()), "ASKING...");
    // The weights going in are not the notes being read.
    assert_eq!(
        doing(&Progress {
            loading: true,
            ..p()
        }),
        "LOADING THE MODEL..."
    );
    // The first question of a conversation reads the whole thing: the notes.
    let first = Progress {
        prompt: 5000,
        fresh: 5000,
        read: 2500,
        ..p()
    };
    assert_eq!(doing(&first), "READING THE NOTES... 50%");
    // The next question reads its own tail; the notes are still in the
    // cache. It used to say the notes were being read and start at 96%.
    let next = Progress {
        prompt: 5200,
        fresh: 200,
        read: 5100,
        ..p()
    };
    assert_eq!(doing(&next), "READING WHAT'S NEW... 50%");
    // Read, and nothing written: the wait for the first token, not 100%.
    let ready = Progress {
        prompt: 5200,
        fresh: 200,
        read: 5200,
        ..p()
    };
    assert_eq!(doing(&ready), "ABOUT TO ANSWER...");
    // And once it is writing, that.
    let writing = Progress {
        prompt: 5200,
        read: 5200,
        written: 12,
        ..p()
    };
    assert!(doing(&writing).starts_with("WRITING... 12 TOKENS"));
    // With what the thinking cost beside it, once there has been any.
    assert!(!doing(&writing).contains("THOUGHT"));
    let thought = Progress {
        thought: 340,
        ..writing
    };
    assert!(
        doing(&thought).ends_with("THOUGHT FOR 340"),
        "{}",
        doing(&thought)
    );
    let thinking = Progress {
        deliberating: true,
        written: 120,
        ..p()
    };
    assert_eq!(doing(&thinking), "THINKING... 120 TOKENS");
}

#[test]
fn a_read_asked_for_in_every_shape_seen_is_a_read() {
    use notes::llm::calls;
    let one = |s: &str| calls(s);
    // As a block, which is the shape it was first seen in.
    assert_eq!(
        one("<read file=\"bike.md\"></read>"),
        vec![("read".into(), "bike.md".into())]
    );
    // As a call, the way the tools are declared.
    assert_eq!(
        one("<tool_call>\n<function=read>\n<parameter=file>\nbike.md\n</parameter>\n</function>\n</tool_call>"),
        vec![("read".into(), "bike.md".into())]
    );
    // And the two fused: no name on the tag, the name as a parameter under
    // it, a call's closing tags. Word for word from a run, where it came out
    // as "it did not answer".
    assert_eq!(
        one("<read>\n<parameter=file>\nshop.md\n</parameter>\n</function>\n</tool_call>"),
        vec![("read".into(), "shop.md".into())]
    );
    // A word that starts the same way is a word.
    assert!(one("the <reader> was <ready>").is_empty());
}

#[test]
fn a_tool_call_is_heard_in_every_dialect() {
    use notes::llm::{calls, declare, Dialect, Tool};
    // Gemma 4: call:name{arg:value}, quoted or not, one or several.
    let gemma = "Let me check.\n<|tool_call>call:date{when:\"2024-12-23\"}<tool_call|><|tool_call>call:calc{expression:87 / 7}<tool_call|>";
    assert_eq!(
        calls(gemma),
        vec![
            ("date".to_string(), "2024-12-23".to_string()),
            ("calc".to_string(), "87 / 7".to_string())
        ]
    );
    // Quoted with Gemma's own quote token, which it uses as often as a mark.
    let token = "<|tool_call>call:calc{expression:<|\"|>384 * 517<|\"|>}<tool_call|>";
    assert_eq!(
        calls(token),
        vec![("calc".to_string(), "384 * 517".to_string())]
    );
    // Liquid: a Python list of calls.
    let liquid =
        "<|tool_call_start|>[date(when=\"today\"), calc(expression='2 + 2')]<|tool_call_end|>";
    assert_eq!(
        calls(liquid),
        vec![
            ("date".to_string(), "today".to_string()),
            ("calc".to_string(), "2 + 2".to_string())
        ]
    );
    // And the machinery of both comes off what is shown.
    assert_eq!(notes::llm::without_machinery(gemma), "Let me check.");
    assert_eq!(notes::llm::without_machinery(liquid), "");

    // Each family is told in its own words.
    let tool = Tool {
        name: "date",
        about: "the day",
        takes: ("when", "a date"),
    };
    let one = std::slice::from_ref(&tool);
    assert!(declare(one, Dialect::Gemma).contains("<|tool>"));
    assert!(declare(one, Dialect::Gemma).contains("<|tool_call>call:"));
    assert!(declare(one, Dialect::Liquid).contains("<|tool_list_start|>"));
    assert!(declare(one, Dialect::Qwen).contains("<tools>"));

    // Told apart by the template.
    assert_eq!(Dialect::of("{{ '<|turn>' }}..."), Dialect::Gemma);
    assert_eq!(Dialect::of("...<|tool_list_start|>..."), Dialect::Liquid);
    assert_eq!(Dialect::of("<|im_start|>..."), Dialect::Qwen);
}

#[test]
fn thinking_in_words_is_kept_out_of_what_is_shown_and_kept() {
    use notes::llm::{without_machinery, without_thoughts};
    let said = "<thinking>\nThe file has three lines. Line 3 is the milk. I should use after=\"3\".\n</thinking>\n\nAdded.\n<edit file=\"shop.md\" after=\"3\">- bread 1.80</edit>";
    let shown = without_machinery(said);
    assert!(!shown.contains("thinking"), "{shown}");
    assert!(!shown.contains("three lines"), "{shown}");
    assert!(shown.starts_with("Added."), "{shown}");
    assert!(
        shown.contains("<edit file="),
        "the change is still there: {shown}"
    );
    // Two thoughts, both gone.
    assert_eq!(
        without_thoughts("<thinking>a</thinking>x<thinking>b</thinking>y"),
        "xy"
    );

    // A thought that reaches for a tool halfway through comes in two halves,
    // in two passes: an open with no close, then a close with no open. Both
    // halves are thinking, and both leaked - the first as its own text, the
    // second as text with a stray tag on the end.
    let first = "<thinking>\nI need the date for that.\n<tool_call><function=date><parameter=when>today</parameter></function></tool_call>";
    assert_eq!(without_machinery(first), "");
    let second = "So it is a Friday.\n</thinking>\n\nIt is Friday.";
    assert_eq!(without_machinery(second), "It is Friday.");

    // A reply that was thinking to the very end shows its words, with the
    // marks off, rather than "it did not answer".
    let only = "<thinking>\nhmm, the answer is 12.4 weeks";
    assert_eq!(notes::llm::shown(only), "hmm, the answer is 12.4 weeks");
}

#[test]
fn nothing_half_typed_is_shown_while_streaming() {
    use notes::llm::{Ask, Assistant, Backend, Progress, Reply, Turn, Watcher};
    /// Streams a reply a piece at a time, the way the real one arrives.
    struct Dribble;
    impl Backend for Dribble {
        fn name(&self) -> String {
            "DRIBBLE".into()
        }
        fn edit(&mut self, _ask: &Ask, w: &mut dyn Watcher) -> Reply {
            let whole = "<thinking>\nplan\n</thinking>\n\nThe bike is red.\n<tool_call>";
            let mut so_far = String::new();
            for piece in [
                "<thin",
                "king>\nplan\n</thinking>\n\nThe bike ",
                "is red.\n<tool_c",
                "all>",
            ] {
                so_far.push_str(piece);
                w.tick(Progress::default(), &so_far);
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
            assert_eq!(so_far, whole);
            Ok("The bike is red.".into())
        }
    }
    let mut a = Assistant::spawn(Box::new(Dribble));
    a.ask(Ask {
        turns: vec![Turn {
            mine: true,
            text: "colour?".into(),
        }],
        ..Default::default()
    });
    // Everything the panel was shown along the way, and none of it a tag.
    let mut seen = Vec::new();
    loop {
        let partial = a.partial().to_string();
        if !partial.is_empty() && seen.last() != Some(&partial) {
            seen.push(partial);
        }
        if a.poll().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(!seen.is_empty(), "nothing was shown at all");
    for shown in &seen {
        assert!(!shown.contains('<'), "a tag was shown: {shown:?}");
        assert!(!shown.contains("plan"), "a thought was shown: {shown:?}");
    }
}

#[test]
fn a_tool_named_as_a_bare_tag_with_its_parameter_as_a_line_is_a_call() {
    use notes::llm::{calls, without_machinery};
    // Word for word, forty questions into one conversation.
    let said = "<calc>\nexpression: (420 / 1205) * 100\n</calc>";
    assert_eq!(
        calls(said),
        vec![("calc".to_string(), "(420 / 1205) * 100".to_string())]
    );
    assert_eq!(without_machinery(said), "");
    // With an equals sign, and something said around it.
    let said = "Let me see.\n<date>\nwhen = 2026-10-14\n</date>\nOne moment.";
    assert_eq!(
        calls(said),
        vec![("date".to_string(), "2026-10-14".to_string())]
    );
    assert_eq!(without_machinery(said), "Let me see.\n\nOne moment.");
    // The value on its own, for a tag that names a tool there is.
    let said = "<date>2026-10-14</date>";
    assert_eq!(
        calls(said),
        vec![("date".to_string(), "2026-10-14".to_string())]
    );
    assert_eq!(without_machinery(said), "");
    // Not a call: a tag around prose with a colon in it, a block, a thought,
    // a value on its own under a name that is no tool.
    for said in [
        "<b>Note: this is bold</b>",
        "<write file=\"a.md\">key: value</write>",
        "<thinking>\nexpression: 1 + 1\n</thinking>",
        "<b>1 + 1</b>",
    ] {
        assert!(
            calls(said).iter().all(|(n, _)| n != "calc" && n != "b"),
            "{said:?}: {:?}",
            calls(said)
        );
    }
}

#[test]
fn a_tool_named_as_a_bare_tag_with_its_parameter_under_it_is_a_call() {
    use notes::llm::{calls, without_machinery};
    // Word for word: neither the wrapper nor <function=, and it was shown.
    let said = "The share is:\n<calc>\n<parameter=expression>\n1.80 / (4.00 + 3.20 + 2.20 + 1.80) * 100\n</parameter>\n</calc>";
    assert_eq!(
        calls(said),
        vec![(
            "calc".to_string(),
            "1.80 / (4.00 + 3.20 + 2.20 + 1.80) * 100".to_string()
        )]
    );
    assert_eq!(without_machinery(said), "The share is:");
    // Without the closing tags either.
    let open = "<date>\n<parameter=when>today";
    assert_eq!(calls(open), vec![("date".to_string(), "today".to_string())]);
    // A block with a parameter is still a block, not a call to "write".
    let block = "<write>\n<parameter=file>\nfacts.md\n</parameter>\n<parameter=content>\nhello\n</parameter>";
    assert!(
        calls(block).iter().all(|(n, _)| n != "write"),
        "{:?}",
        calls(block)
    );
}

#[test]
fn a_call_with_the_argument_left_out_is_still_a_call() {
    use notes::llm::calls;
    // Word for word: asked for a number of days in weeks, it wrote the call
    // and forgot the sum. It used to come out as "it did not answer".
    let said = "<tool_call>\n<function=calc>\n</function>\n</tool_call>";
    assert_eq!(calls(said), vec![("calc".to_string(), String::new())]);
    // Run with nothing, the tool says what it wanted rather than nothing.
    let told = notes::tools::run("calc", "", "");
    assert!(told.contains("<parameter=expression>"), "{told}");
    // And the shape the instructions show is not read as a call to nothing.
    assert!(calls("<function=example_function_name>\n<parameter=example_parameter_1>\nvalue_1\n</parameter>\n</function>").len() == 1);
    assert!(calls("nothing here").is_empty());
}

#[test]
fn a_call_with_a_name_and_no_body_is_not_an_empty_block() {
    use notes::chat::{proposals, What};
    use notes::llm::unfused;
    // The first pass: write called as a tool, with a name and nothing else.
    let call = "Then I will delete the originals.\n\n<tool_call>\n<function=write>\n<parameter=file>\nweek.md\n</parameter>\n</function>\n</tool_call>";
    let mended = unfused(call);
    assert!(
        !mended.contains("<write"),
        "an empty block was made: {mended}"
    );
    // Kept in front of the real reply, as it is, it must not be a second
    // write - two writes are not a merge, and the deletes went first.
    let turn = format!(
        "{}\n\n<write file=\"week.md\"># Week\n</write>\n<delete file=\"monday.md\"></delete>\n<delete file=\"tuesday.md\"></delete>",
        notes::llm::without_machinery(call)
    );
    let (_, changes) = proposals(&turn);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(matches!(changes[0].what, What::Merge { .. }), "{changes:?}");
    // And even with an empty draft left standing, drafts collapse first.
    let drafts = "<write file=\"week.md\">\n\n</write>\n<write file=\"week.md\"># Week\n</write>\n<delete file=\"monday.md\"></delete>";
    let (_, changes) = proposals(drafts);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(matches!(changes[0].what, What::Merge { .. }), "{changes:?}");
}

#[test]
fn gemma_may_write_the_argument_as_an_attribute_after_the_name() {
    use notes::llm::{calls, without_machinery};
    // Word for word, three at once, dressed like the change blocks.
    let said = "<|tool_call>call:read file=\"barn/hens.md\"<tool_call|><|tool_call>call:read file=\"barn/goats.md\"{}<tool_call|>";
    assert_eq!(
        calls(said),
        vec![
            ("read".to_string(), "barn/hens.md".to_string()),
            ("read".to_string(), "barn/goats.md".to_string()),
        ]
    );
    assert_eq!(without_machinery(said), "");
    // The usual shape still.
    let said = "<|tool_call>call:calc{expression:<|\"|>384 * 517<|\"|>}<tool_call|>";
    assert_eq!(
        calls(said),
        vec![("calc".to_string(), "384 * 517".to_string())]
    );
}

#[test]
fn a_closing_mark_after_the_thought_is_done_is_not_shown() {
    use notes::llm::without_thoughts;
    // Word for word: the thought, the answer, the mark again, the answer again.
    let said = "<thinking>\nIt was 590.\n</thinking>\n\nThe hotel cost 590.\n</thinking>\n\nThe hotel cost 590.";
    assert_eq!(without_thoughts(said), "The hotel cost 590.");
    // Said twice with a difference is said twice.
    let said = "<thinking>\nIt was 590.\n</thinking>\n\nThe hotel cost 590.\n</thinking>\n\nBefore that, 610.";
    assert_eq!(
        without_thoughts(said),
        "The hotel cost 590.\n\nBefore that, 610."
    );
}

#[test]
fn a_reply_that_stopped_after_its_thought_is_carried_on_from_it() {
    use notes::llm::{Ask, Assistant, Backend, Reply, Turn, Watcher};
    /// A model that closed its thought and ended the turn - which happens -
    /// and, handed the thought to carry on from, finishes.
    struct Stalled(std::sync::Arc<std::sync::Mutex<Vec<Option<String>>>>);
    impl Backend for Stalled {
        fn name(&self) -> String {
            "STALLED".into()
        }
        fn edit(&mut self, ask: &Ask, _w: &mut dyn Watcher) -> Reply {
            let mut seen = self.0.lock().unwrap();
            seen.push(ask.prefill.clone());
            Ok(match ask.prefill.as_deref() {
                None => "<thinking>\nRename line 8.\n</thinking>".into(),
                Some(prefill) => format!("{prefill}<edit lines=\"8\">| museum | 45 |</edit>"),
            })
        }
    }
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut a = Assistant::spawn(Box::new(Stalled(seen.clone())));
    a.ask(Ask {
        turns: vec![Turn {
            mine: true,
            text: "rename it".into(),
        }],
        ..Default::default()
    });
    let said = loop {
        if let Some(r) = a.poll() {
            break r.expect("an answer");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "asked once more, carrying on: {seen:?}");
    assert!(
        seen[1]
            .as_deref()
            .is_some_and(|p| p.contains("Rename line 8.") && p.ends_with("\n\n")),
        "{seen:?}"
    );
    assert!(said.contains("<edit lines=\"8\">"), "{said:?}");
    assert!(
        !said.contains("<thinking>"),
        "the thought is not shown: {said:?}"
    );
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
fn the_examples_name_the_note_that_is_open() {
    use notes::llm::{Ask, Turn};
    let asking = |file: &str| Ask {
        file: file.into(),
        vault: "- `new-one/family.md`".into(),
        turns: vec![Turn {
            mine: true,
            text: "change line seven".into(),
        }],
        ..Ask::default()
    };
    let system = asking("new-one/family.md").system("editing");
    // The name in the example is the name it will write, so it is the one that
    // works. It used to say `notes.md` - a fine name for an example until a
    // model copied it into a project that had no notes.md, and the change went
    // nowhere with every other part of the answer right.
    assert!(
        system.contains("<edit file=\"family.md\" lines=\"12-14\">"),
        "{system}"
    );
    assert!(
        !system.contains("file=\"notes.md\""),
        "the example still names a file that is not there: {system}"
    );
    // Without its folder, which is what the rule underneath it asks for.
    assert!(!system.contains("new-one/family.md\" lines"), "{system}");

    // And an editor with nothing open still reads as an example.
    let nothing = asking("");
    assert!(nothing
        .system("editing")
        .contains("<edit file=\"notes.md\""));
}

#[test]
fn nothing_the_model_could_copy_is_left_in_its_own_turn() {
    use notes::chat::as_sent;
    use notes::llm::Turn;
    let turn = |mine: bool, text: &str| Turn {
        mine,
        text: text.to_string(),
    };
    // A conversation with one of everything in it.
    let turns = vec![
        turn(true, "how old is eva"),
        turn(
            false,
            "<used tool=\"date\" arg=\"2024-12-23\">\n23 December 2024 was 613 days ago.\n</used>\n\
             \nEva is 613 days old.",
        ),
        turn(true, "write that down"),
        turn(
            false,
            "<write file=\"ages.md\" state=\"applied\">Eva: 613 days</write>",
        ),
        turn(true, "and read it back"),
        turn(
            false,
            "<used tool=\"read\" arg=\"ages.md\">\n   1 | Eva: 613 days\n</used>\n\nIt says 613.",
        ),
        turn(true, "thanks"),
    ];
    let sent = as_sent(&turns, &[]);

    // Their turns hold what they said and the one block that is still the only
    // copy of a file. Nothing else - no tag, no bracket, no shape that being
    // copied would turn into a lie. It forged the tool tag when it could see
    // one, and forged the bracket that replaced it when it could see that.
    for (i, text) in sent.iter().enumerate() {
        if turns[i].mine {
            continue;
        }
        assert!(!text.contains("<used"), "turn {i}: {text}");
        assert!(!text.contains("[you "), "turn {i}: {text}");
        assert!(!text.contains("[write to"), "turn {i}: {text}");
        assert!(!text.contains("[edit to"), "turn {i}: {text}");
    }
    assert!(sent[1].contains("Eva is 613 days old."), "{}", sent[1]);
    assert!(sent[5].contains("It says 613."), "{}", sent[5]);

    // What was taken out is said in our turns, which it never writes.
    assert!(sent[2].contains("date tool was asked about"), "{}", sent[2]);
    assert!(sent[2].contains("613 days ago"), "{}", sent[2]);
    assert!(sent[6].contains("You read `ages.md`"), "{}", sent[6]);

    // And the question itself is still the last thing in its own turn, because
    // what is nearest the end is what gets answered.
    assert!(
        sent[2].trim_end().ends_with("write that down"),
        "{}",
        sent[2]
    );
}

#[test]
fn an_accepted_block_the_file_has_moved_on_from_loses_its_body() {
    use notes::chat::as_sent;
    use notes::llm::Turn;
    let turn = |mine: bool, text: &str| Turn {
        mine,
        text: text.to_string(),
    };
    let turns = vec![
        turn(true, "make the door green"),
        turn(
            false,
            "<edit file=\"door.md\" lines=\"3\" state=\"applied\">The door is GREEN.</edit>",
        ),
        turn(true, "what colour is it"),
    ];
    // While the file says what the edit said, the edit is a label saying it
    // was accepted: what the file says was told with the next question.
    let green = vec![(
        "door.md".to_string(),
        "# Door\n\nThe door is GREEN.\n".to_string(),
    )];
    let sent = as_sent(&turns, &green);
    assert!(!sent[1].contains("GREEN"), "{}", sent[1]);
    assert!(sent[2].contains("was accepted."), "{}", sent[2]);
    assert!(!sent[2].contains("has changed since"), "{}", sent[2]);

    // Undone by hand: the file says blue, and the edit is a note saying the
    // file has moved on - not a body the model will trust over the page,
    // which it did: shown the project afresh saying blue, it said green.
    let blue = vec![(
        "door.md".to_string(),
        "# Door\n\nThe door is BLUE.\n".to_string(),
    )];
    let sent = as_sent(&turns, &blue);
    assert!(!sent[1].contains("GREEN"), "{}", sent[1]);
    assert!(sent[2].contains("has changed since"), "{}", sent[2]);

    // With nothing known about the files, nothing is second-guessed.
    let sent = as_sent(&turns, &[]);
    assert!(sent[2].contains("was accepted."), "{}", sent[2]);
    assert!(!sent[2].contains("has changed since"), "{}", sent[2]);
}

#[test]
fn an_accepted_change_is_a_label_and_the_file_is_told_elsewhere() {
    use notes::chat::as_sent;
    use notes::llm::Turn;
    let turn = |mine: bool, text: &str| Turn {
        mine,
        text: text.to_string(),
    };
    let turns = vec![
        turn(true, "make a note about the bike"),
        turn(
            false,
            "Made it.\n<write file=\"bike.md\" state=\"applied\">the bike is red</write>",
        ),
        turn(true, "what colour is it"),
        turn(false, "Red."),
    ];
    let sent = as_sent(&turns, &[]);

    // The block it wrote does not go back: what the file says was told with
    // the next question, numbered, and a second copy with no numbers in the
    // margin is the one it would count lines in. What it said around the
    // block stays.
    assert!(!sent[1].contains("the bike is red"), "{}", sent[1]);
    assert!(sent[1].contains("Made it."), "{}", sent[1]);

    // Changed again, and neither carries the text.
    let mut later = turns.clone();
    later.push(turn(true, "make it green"));
    later.push(turn(
        false,
        "<write file=\"bike.md\" state=\"applied\">the bike is green</write>",
    ));
    let sent = as_sent(&later, &[]);
    assert!(
        !sent[1].contains("the bike is red"),
        "kept twice: {}",
        sent[1]
    );
    // And what is left to say about it - that it was proposed, and how it went
    // - is said in the next turn of ours, where the model does not write.
    assert!(
        !sent[1].contains("Your write"),
        "in their turn: {}",
        sent[1]
    );
    assert!(
        sent[2].contains("Your write to `bike.md` was accepted"),
        "{}",
        sent[2]
    );
    assert!(!sent[5].contains("the bike is green"), "{}", sent[5]);

    // A change turned down is a label, whatever else happened. It never became
    // what the file says, so keeping its text would be keeping a file that has
    // never existed.
    let refused = vec![turn(
        false,
        "<write file=\"bike.md\" state=\"rejected\">the bike is blue</write>",
    )];
    assert!(
        !as_sent(&refused, &[])[0].contains("blue"),
        "{:?}",
        as_sent(&refused, &[])
    );
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
    let missing = notes::tools::run("read", "nowhere-at-all.md", "");
    assert!(missing.contains("no note called"), "{missing}");
}
