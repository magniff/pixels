//! Conversations: what a reply proposes, what is kept, what the model is
//! shown next time, and what the panel says while it waits.

use notes::chat;
use notes::llm::Turn;

/// One file, as the panel would see the project it is in.
fn folder_of<'a>(name: &str, lines: &'a [String]) -> chat::Folder<'a> {
    chat::Folder {
        project: String::new(),
        here: name.to_string(),
        files: vec![(name.to_string(), lines)],
    }
}

fn vault_with(dir: &std::path::Path, files: &[(&str, &str)]) {
    let _ = std::fs::remove_dir_all(dir);
    for (path, text) in files {
        let at = dir.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }
}

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
    let first = chat::proposals(reply).1.remove(0);
    let settled = chat::settle(reply, &first, true);
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

    let rejected = chat::settle(&settled, &first, false);
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
    let second = chat::proposals(two).1.remove(1);
    let (_, edits) = chat::proposals(&chat::settle(two, &second, true));
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
        project: String::new(),
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

    let offered = chat::proposals(&talk.turns[1].text).1.remove(0);
    talk.turns[1].text = chat::settle(&talk.turns[1].text, &offered, false);
    assert!(!talk.pending(&folder), "rejecting is an answer too");
}

#[test]
fn a_change_that_can_no_longer_be_made_holds_nothing() {
    // Otherwise a block whose lines have gone is a conversation nobody can get
    // out of: it cannot be accepted, cannot be rejected, and blocks the field.
    let note: Vec<String> = vec!["only one line".to_string()];
    let folder = chat::Folder {
        project: String::new(),
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
        project: String::new(),
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
    // The edit and the write together; the delete on its own, because a
    // write with a delete beside it is read as the merge it is - see
    // `a_write_with_deletes_beside_it_is_the_merge_it_is`.
    let reply = "Three things.\n\n<edit file=\"a.md\" lines=\"2-3\">two\n</edit>\n\n<write file=\"b.md\">all of it</write>";
    let (prose, mut changes) = chat::proposals(reply);
    assert_eq!(
        prose, "Three things.",
        "and none of the machinery is left in it"
    );
    changes.extend(chat::proposals("<delete file=\"c.md\"></delete>").1);
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
        project: String::new(),
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
        project: String::new(),
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
        project: String::new(),
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
fn the_last_block_aimed_at_a_place_stands_for_the_earlier_ones() {
    use notes::chat::{proposals, What};
    // A model thinking as it writes: a pair of edits, "wait", another pair.
    let said = "<edit file=\"prices.md\" lines=\"7\">| Eggs | 0.75 |</edit>\n\
                <edit file=\"summary.md\" lines=\"5-7\">- wrong\n- wrong\n- wrong</edit>\n\n\
                Wait, I need to recalculate.\n\n\
                <edit file=\"prices.md\" lines=\"7\">| Eggs | 0.75 |</edit>\n\
                <edit file=\"summary.md\" lines=\"5-7\">- right\n- right\n- right</edit>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 2, "{changes:?}");
    let summary = changes
        .iter()
        .find(|c| c.file.as_deref() == Some("summary.md"))
        .unwrap();
    match &summary.what {
        What::Edit { text, .. } => assert!(text.contains("right"), "{text}"),
        other => panic!("{other:?}"),
    }
    // Two edits to different lines of one file are two edits.
    let said = "<edit file=\"a.md\" lines=\"2\">x</edit>\n<edit file=\"a.md\" lines=\"9\">y</edit>";
    assert_eq!(proposals(said).1.len(), 2);
}

#[test]
fn a_merge_applied_at_the_top_of_the_vault_takes_both_parts_away() {
    use notes::chat::{Change, What};
    let dir = std::env::temp_dir().join(format!("notes-mergetop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a vault");
    std::fs::write(dir.join("monday.md"), "# Monday\n\n- Swim\n").expect("a note");
    std::fs::write(dir.join("tuesday.md"), "# Tuesday\n\n- Climb\n").expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.filename() == "monday.md")
        .expect("there");
    app.apply_change(&Change {
        file: Some("week.md".into()),
        what: What::Merge {
            from: vec!["monday.md".into(), "tuesday.md".into()],
            text: "# Week\n\n- Swim\n- Climb\n".into(),
        },
        state: Some(true),
    });
    assert!(!dir.join("monday.md").exists(), "monday is still there");
    assert!(!dir.join("tuesday.md").exists(), "tuesday is still there");
    let week = app
        .notes
        .iter()
        .find(|n| n.filename() == "week.md")
        .expect("the week was made");
    assert!(week.buffer.to_text().contains("Climb"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deciding_a_change_marks_every_block_it_came_from() {
    use notes::chat::{proposals, settle};
    // Drafts: three blocks for one place, one change. Deciding it must mark
    // all three, or the one not marked is offered again.
    let drafts =
        "<edit file=\"a.md\" lines=\"2\">x</edit>\nwait\n<edit file=\"a.md\" lines=\"2\">y</edit>";
    let change = proposals(drafts).1.remove(0);
    let settled = settle(drafts, &change, true);
    assert_eq!(settled.matches("state=\"applied\"").count(), 2, "{settled}");
    assert!(proposals(&settled).1.iter().all(|c| c.state == Some(true)));

    // A merge folded from a write and two deletes marks all three blocks.
    let merge = "<write file=\"week.md\">w</write>\n<delete file=\"monday.md\"></delete>\n<delete file=\"tuesday.md\"></delete>";
    let change = proposals(merge).1.remove(0);
    let settled = settle(merge, &change, true);
    assert_eq!(settled.matches("state=\"applied\"").count(), 3, "{settled}");
    assert!(proposals(&settled).1.iter().all(|c| c.state == Some(true)));
}

#[test]
fn a_write_with_deletes_beside_it_is_the_merge_it_is() {
    use notes::chat::{proposals, What};
    // Word for word, after the tool it tried first had been answered.
    let said =
        "<used tool=\"write\" arg=\"week.md\">\nwrite is not a tool. Changing a file is not \
                something you call: write a <write> block in your reply instead.\n</used>\n\n\
                <write file=\"week.md\"># Monday\n\n- Swim\n\n# Tuesday\n\n- Climb\n</write>\n\
                <delete file=\"monday.md\"></delete>\n<delete file=\"tuesday.md\"></delete>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].file.as_deref(), Some("week.md"));
    match &changes[0].what {
        What::Merge { from, text } => {
            assert_eq!(from, &["monday.md".to_string(), "tuesday.md".to_string()]);
            assert!(text.contains("Swim") && text.contains("Climb"), "{text}");
        }
        other => panic!("read as {other:?}"),
    }
    // A write on its own is a write; a delete on its own is a delete.
    assert!(matches!(
        proposals("<write file=\"a.md\">x</write>").1[0].what,
        What::Write { .. }
    ));
    assert!(matches!(
        proposals("<delete file=\"a.md\"></delete>").1[0].what,
        What::Delete
    ));
}

#[test]
fn a_delete_a_merge_already_covers_is_dropped() {
    use notes::chat::{proposals, What};
    // Word for word: a merge, and then the deletes the merge already does.
    // Taken in the wrong order the days are gone before the week is made.
    let said = "<merge into=\"week.md\" from=\"monday.md, tuesday.md\"># Week\n</merge>\n\
                <delete file=\"monday.md\"></delete>\n<delete file=\"tuesday.md\"></delete>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert!(matches!(changes[0].what, What::Merge { .. }));
    // A delete of something else in the same reply is still a delete.
    let said = "<merge into=\"week.md\" from=\"monday.md\"># Week\n</merge>\n<delete file=\"old.md\"></delete>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 2, "{changes:?}");
}

#[test]
fn a_tag_that_forgot_its_closing_bracket_still_opens_a_block() {
    use notes::chat::{proposals, What};
    let said = "<write file=\"shop.md\"\n# Shop\n\n- milk: 2.50\n</write>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].file.as_deref(), Some("shop.md"));
    match &changes[0].what {
        What::Write { text } => assert_eq!(text, "# Shop\n\n- milk: 2.50"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_delete_written_as_a_lone_tag_is_a_delete() {
    use notes::chat::{proposals, What};
    for said in [
        "<delete file=\"scratch.md\">",
        "<delete file=\"scratch.md\"/>",
        "Gone.\n<delete file=\"scratch.md\">\nThat was it.",
        "<delete file=\"scratch.md\"></delete>",
    ] {
        let (prose, changes) = proposals(said);
        assert_eq!(changes.len(), 1, "{said:?}: {changes:?}");
        assert_eq!(changes[0].what, What::Delete, "{said:?}");
        assert_eq!(changes[0].file.as_deref(), Some("scratch.md"));
        assert!(!prose.contains("<delete"), "{said:?}: {prose:?}");
    }
}

#[test]
fn an_edit_to_a_file_the_model_made_is_answered_with_the_whole_file() {
    use notes::chat::Chat;
    let mut chat = Chat::new("trip".into(), "zzqqtrip.md".into());
    let front = vec![("zzqqtrip.md".to_string(), "# Trip\n".to_string())];
    chat.context("- `zzqqtrip.md`", &front);

    // The model wrote a note and then put a row into it. The note is not at
    // the front, so what it has of it is its own write, with no numbers in
    // the margin. A diff on top of that had it editing the wrong line; the
    // whole file, numbered, is what it needs.
    let wrote = "# Budget\n\n| Item | Cost |\n|---|---|\n| flights | 420 |\n| hotel | 610 |\n";
    chat.wrote("budget.md", Some(wrote));
    let edited = "# Budget\n\n| Item | Cost |\n|---|---|\n| food | 150 |\n| flights | 420 |\n| hotel | 610 |\n";
    chat.did("edit", "budget.md", wrote, edited);
    chat.wrote("budget.md", Some(edited));
    let mut now = front.clone();
    now.push(("budget.md".to_string(), edited.to_string()));
    let (_, _, moved) = chat.context("- `zzqqtrip.md`\n- `budget.md`", &now);
    let said = moved.expect("what the edit did is said");
    assert!(
        said.contains("Your edit to `budget.md` was applied"),
        "{said}"
    );
    assert!(said.contains("in full"), "{said}");
    assert!(said.contains("   7 | | hotel | 610 |"), "numbered: {said}");
    assert!(!said.contains("@@"), "not a diff: {said}");

    // A file at the front still gets the diff: it has numbers already. Long
    // enough that the diff is a diff and not the file.
    let room = "a line about the journey\n".repeat(45);
    let (was, is) = (
        format!("# Trip\n\n{room}"),
        format!("# Trip\n\n{room}Off we go.\n"),
    );
    chat.wrote("zzqqtrip.md", Some(&was));
    chat.did("edit", "zzqqtrip.md", &was, &is);
    now[0].1 = is;
    chat.wrote("zzqqtrip.md", Some(&now[0].1));
    let (_, _, moved) = chat.context("- `zzqqtrip.md`\n- `budget.md`", &now);
    let said = moved.expect("said");
    assert!(said.contains("@@"), "{said}");
    assert!(!said.contains("in full"), "{said}");
}

#[test]
fn a_note_made_with_the_name_of_one_in_another_project_is_known_as_itself() {
    use notes::chat::{Change, Chat, What};
    let dir = std::env::temp_dir().join(format!("notes-twin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("trip")).expect("a vault");
    std::fs::write(dir.join("ideas.md"), "# Ideas\n\n- [ ] a preview pane\n").expect("a note");
    std::fs::write(dir.join("trip").join("plan.md"), "# Plan\n").expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.filename() == "plan.md")
        .expect("there");
    let mut chat = Chat::new("trip".into(), "plan.md".into());
    chat.context(
        "- `plan.md`",
        &[("plan.md".to_string(), "# Plan\n".to_string())],
    );
    let write = Change {
        file: Some("ideas.md".into()),
        what: What::Write {
            text: "book the museum early".into(),
        },
        state: None,
    };
    app.apply_change(&write);
    app.took_up_for_test(&write, &mut chat);
    // Known as what was written into trip/, not as the checklist at the top.
    assert_eq!(chat.knows("ideas.md"), Some("book the museum early"));
    // And told, numbered, with the next question: the block it wrote has no
    // numbers in the margin and does not go back.
    let now = vec![
        ("plan.md".to_string(), "# Plan\n".to_string()),
        ("ideas.md".to_string(), "book the museum early".to_string()),
    ];
    let (_, _, moved) = chat.context("- `plan.md`\n- `ideas.md`", &now);
    let said = moved.expect("the new file is told");
    assert!(
        said.contains("Your write to `ideas.md` was accepted"),
        "{said}"
    );
    assert!(said.contains("   1 | book the museum early"), "{said}");
    assert!(!said.contains("STOP"), "its own file is not news: {said}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edit_undone_by_hand_is_seen_as_a_change() {
    use notes::chat::Chat;
    let file = |t: &str| vec![("door.md".to_string(), t.to_string())];
    // Long enough that what changed is said as a diff rather than the file.
    let room = "a line about the hallway\n".repeat(30);
    let blue = &format!("# Door\n\n{room}The door is BLUE.\n");
    let green = &format!("# Door\n\n{room}The door is GREEN.\n");
    let mut chat = Chat::new("home".into(), "door.md".into());
    chat.context("- `door.md`", &file(blue));

    // The model's edit, applied: what it did is said once, as its own doing.
    let before = chat.knows("door.md").unwrap().to_string();
    chat.did("edit", "door.md", &before, green);
    chat.wrote("door.md", Some(green));
    let (_, _, moved) = chat.context("- `door.md`", &file(green));
    let said = moved.expect("what the edit did is said");
    assert!(
        said.contains("Your edit to `door.md` was applied"),
        "{said}"
    );
    assert!(said.contains("+The door is GREEN."), "{said}");
    assert!(
        !said.contains("STOP"),
        "its own edit is not news from outside: {said}"
    );

    // Once. Asked again with nothing moved, nothing is said.
    let (_, _, moved) = chat.context("- `door.md`", &file(green));
    assert!(moved.is_none(), "{moved:?}");

    // Undone by hand: back to blue, which is a change from what it knows.
    // It used to look like nothing, because the file was still known as it
    // was before the edit - and the model went on saying green.
    let (_, _, moved) = chat.context("- `door.md`", &file(blue));
    let said = moved.expect("the undo is a change");
    assert!(said.contains("STOP"), "{said}");
    assert!(said.contains("+The door is BLUE."), "{said}");
}

#[test]
fn a_line_is_added_below_another_without_replacing_it() {
    use notes::chat::{proposals, Change, Folder, What};
    let said = "<edit file=\"shop.md\" after=\"3\">- bread 1.80</edit>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].what,
        What::Insert {
            after: 3,
            text: "- bread 1.80".into()
        }
    );

    let lines: Vec<String> = vec!["# Shop".into(), String::new(), "- milk 2.50".into()];
    let folder = Folder {
        project: String::new(),
        here: "shop.md".into(),
        files: vec![("shop.md".to_string(), &lines[..])],
    };
    // Replaces nothing, and is offered - below the last line, or at the top.
    assert_eq!(changes[0].replacing(&folder).as_deref(), Some(""));
    let top = Change {
        what: What::Insert {
            after: 0,
            text: "- first".into(),
        },
        ..changes[0].clone()
    };
    assert_eq!(top.replacing(&folder).as_deref(), Some(""));
    assert!(top.headline("shop.md").contains("AT THE TOP"));
    // Below a line that is not there is nowhere.
    let past = Change {
        what: What::Insert {
            after: 9,
            text: "- x".into(),
        },
        ..changes[0].clone()
    };
    assert!(past.replacing(&folder).is_none());

    // Applied: the milk is still there, once, and the bread is under it.
    let dir = std::env::temp_dir().join(format!("notes-insert-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a vault");
    std::fs::write(dir.join("shop.md"), "# Shop\n\n- milk 2.50").expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.filename() == "shop.md")
        .expect("there");
    app.apply_change(&changes[0]);
    app.apply_change(&top);
    let text = app.notes[app.current].buffer.to_text();
    assert_eq!(
        text.trim_end(),
        "- first\n# Shop\n\n- milk 2.50\n- bread 1.80",
        "{text:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edit_one_past_the_end_adds_a_line() {
    use notes::chat::{Change, Folder, What};
    let lines: Vec<String> = vec!["# Shop".into(), String::new(), "- Milk - 2.50".into()];
    let folder = Folder {
        project: String::new(),
        here: "shop.md".into(),
        files: vec![("shop.md".to_string(), &lines[..])],
    };
    let edit = |from: usize| Change {
        file: Some("shop.md".into()),
        what: What::Edit {
            from,
            to: from,
            text: "- Bread - 1.80".into(),
        },
        state: None,
    };
    // Line four of a three-line file: the one that is not there yet, which
    // is where something added goes. Offered, replacing nothing.
    assert_eq!(edit(4).replacing(&folder).as_deref(), Some(""));
    // Line five is not the next line; it is nowhere.
    assert!(edit(5).replacing(&folder).is_none());
    // Below line four is the same place, and offered; below five is nowhere.
    let below = |after: usize| Change {
        file: Some("shop.md".into()),
        what: What::Insert {
            after,
            text: "- Bread - 1.80".into(),
        },
        state: None,
    };
    assert_eq!(below(4).replacing(&folder).as_deref(), Some(""));
    assert!(below(5).replacing(&folder).is_none());

    // And applied, it lands after the last line.
    let dir = std::env::temp_dir().join(format!("notes-append-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a vault");
    std::fs::write(dir.join("shop.md"), "# Shop\n\n- Milk - 2.50").expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.filename() == "shop.md")
        .expect("there");
    app.apply_change(&edit(4));
    let text = app.notes[app.current].buffer.to_text();
    assert_eq!(
        text.trim_end(),
        "# Shop\n\n- Milk - 2.50\n- Bread - 1.80",
        "{text:?}"
    );
    // And below the line that is not there yet, the same.
    app.apply_change(&Change {
        file: Some("shop.md".into()),
        what: What::Insert {
            after: 5,
            text: "- Eggs - 3.00".into(),
        },
        state: None,
    });
    let text = app.notes[app.current].buffer.to_text();
    assert!(
        text.trim_end().ends_with("- Bread - 1.80\n- Eggs - 3.00"),
        "{text:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
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
    // Only for a file that is not there yet. Asked to rename one word in a
    // nine-line budget, a model wrote `<edit file="budget.md">` with the one
    // line in it, and read as a write it laid that line over the budget.
    use notes::chat::Folder;
    let budget: Vec<String> = "# Budget\n\n| food | 150 |"
        .lines()
        .map(String::from)
        .collect();
    let there = Folder {
        project: "trip".into(),
        here: "zzqqtrip.md".into(),
        files: vec![("ages.md".to_string(), &budget[..])],
    };
    assert!(
        changes[0].replacing(&there).is_none(),
        "offered over a file that is there"
    );
    let not = Folder {
        project: "trip".into(),
        here: "zzqqtrip.md".into(),
        files: vec![("budget.md".to_string(), &budget[..])],
    };
    assert_eq!(changes[0].replacing(&not).as_deref(), Some(""));
    // And the model is told why nothing came of it, or it writes the same
    // block again when asked what became of it.
    use notes::chat::as_sent;
    use notes::llm::Turn;
    let turns = vec![
        Turn {
            mine: true,
            text: "rename it".into(),
        },
        Turn {
            mine: false,
            text: said.to_string(),
        },
        Turn {
            mine: true,
            text: "and?".into(),
        },
    ];
    let sent = as_sent(&turns, &[("ages.md".to_string(), "# Ages\n".to_string())]);
    assert!(sent[2].contains("named no lines"), "{}", sent[2]);
    assert!(sent[2].contains("Read the file"), "{}", sent[2]);
    // For a file not there, it is simply not answered yet.
    let sent = as_sent(&turns, &[]);
    assert!(
        sent[2].contains("not answered either way yet"),
        "{}",
        sent[2]
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
fn a_change_naming_this_project_is_a_change_here_and_one_naming_another_is_not() {
    use notes::chat::{elsewhere, Change, Folder, What};
    let lines: Vec<String> = "# Bikes\n\n- Alice's bike is green.\n- Bob's bike is red.\n\
                              - Magniff's bike is white.\n"
        .split('\n')
        .map(str::to_string)
        .collect();
    let folder = Folder {
        project: "new-one".into(),
        here: "bikes.md".into(),
        files: vec![("bikes.md".to_string(), &lines[..])],
    };
    let edit = |file: &str| Change {
        file: Some(file.into()),
        what: What::Edit {
            from: 5,
            to: 5,
            text: "- Magniff's bike is silver.".into(),
        },
        state: None,
    };

    // The folder written in, and it is this one: the same change as with the
    // folder left off, which is what the rule asks for. The panel used to
    // refuse this while the application would have applied it.
    let here = edit("new-one/bikes.md");
    assert_eq!(
        here.replacing(&folder).as_deref(),
        Some("- Magniff's bike is white.")
    );
    assert_eq!(here.replacing(&folder), edit("bikes.md").replacing(&folder));
    assert!(here.misplaced(&folder).is_none());

    // A name nothing is filed under is a file that is not there. It used to
    // mean the note in front of you, and a model asked to make bike.md wrote
    // an edit to line 1 of it - and line 1 of the open note became "RED".
    assert!(edit("other.md").replacing(&folder).is_none());
    // No name at all is still the note in front of you.
    let unnamed = Change {
        file: None,
        ..edit("x")
    };
    assert_eq!(unnamed.replacing(&folder), here.replacing(&folder));

    // A folder that is some other project is a change this conversation
    // cannot make, and says which project. It used to fall through to the
    // note in front of you: an edit meant for typography/bikes.md offered
    // against new-one/bikes.md, line for line.
    let away = edit("typography/bikes.md");
    assert_eq!(away.misplaced(&folder).as_deref(), Some("typography"));
    assert!(
        away.replacing(&folder).is_none(),
        "offered against this project"
    );

    // A merge that reaches out of the project by any of its names is too.
    let merge = Change {
        file: Some("bikes.md".into()),
        what: What::Merge {
            from: vec!["bikes.md".into(), "aquarium/stock.md".into()],
            text: String::new(),
        },
        state: None,
    };
    assert_eq!(merge.misplaced(&folder).as_deref(), Some("aquarium"));

    // The rule itself, on its own.
    assert_eq!(
        elsewhere("aquarium/stock.md", "new-one").as_deref(),
        Some("aquarium")
    );
    assert_eq!(elsewhere("new-one/stock.md", "new-one"), None);
    assert_eq!(elsewhere("stock.md", "new-one"), None);
    assert_eq!(elsewhere("stock.md", ""), None);
    assert_eq!(
        elsewhere("aquarium/stock.md", "").as_deref(),
        Some("aquarium")
    );
    assert_eq!(
        elsewhere("/aquarium/stock.md", "").as_deref(),
        Some("aquarium")
    );

    // Lines that are not there are still not there, whatever the file.
    let past = Change {
        file: Some("bikes.md".into()),
        what: What::Edit {
            from: 90,
            to: 90,
            text: "- nowhere.".into(),
        },
        state: None,
    };
    assert!(past.replacing(&folder).is_none());
}

#[test]
fn applying_a_change_aimed_at_another_project_changes_nothing() {
    use notes::chat::{Change, What};
    let dir = std::env::temp_dir().join(format!("notes-elsewhere-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (project, text) in [
        ("new-one", "# Family\n\nDanila.\n"),
        ("aquarium", "# Stock\n\n12 tetras\n"),
    ] {
        std::fs::create_dir_all(dir.join(project)).expect("a project");
        let name = if project == "new-one" {
            "family.md"
        } else {
            "stock.md"
        };
        std::fs::write(dir.join(project).join(name), text).expect("a note");
    }
    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.filename() == "family.md")
        .expect("there");
    let before: Vec<String> = app.notes.iter().map(|n| n.buffer.to_text()).collect();
    let count = app.notes.len();

    // An edit, a write and a delete, each naming the other project.
    for what in [
        What::Edit {
            from: 3,
            to: 3,
            text: "13 tetras".into(),
        },
        What::Write {
            text: "# Stock\n\n13 tetras\n".into(),
        },
        What::Delete,
    ] {
        app.apply_change(&Change {
            file: Some("aquarium/stock.md".into()),
            what,
            state: Some(true),
        });
        assert!(app.status.contains("AQUARIUM"), "{}", app.status);
    }
    let after: Vec<String> = app.notes.iter().map(|n| n.buffer.to_text()).collect();
    assert_eq!(before, after, "something in the vault moved");
    assert_eq!(app.notes.len(), count, "a file was made somewhere");
    assert!(
        !dir.join("new-one/stock.md").exists(),
        "made in this project instead"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_edit_that_landed_somewhere_else_is_reported_back() {
    use notes::chat::{Change, What};
    use notes::text::Buffer;
    let dir = std::env::temp_dir().join(format!("notes-wrongline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a vault");
    let file = dir.join("bike.md");
    // Long enough that saying what changed beats saying the file, which is the
    // path worth testing: the two lines it touched, and nothing else.
    let filler = (1..=40)
        .map(|n| format!("Some other note about bikes, number {n}.\n"))
        .collect::<String>();
    let started = format!(
        "# Bikes\n\n- Alice's bike is green.\n- Bob's bike is blue.\n\
         - Magniff's bike is orange.\n{filler}"
    );
    let started = started.as_str();
    std::fs::write(&file, started).expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    let i = app
        .notes
        .iter()
        .position(|n| n.filename() == "bike.md")
        .expect("there");
    app.current = i;
    let mut chat = notes::chat::Chat::new(String::new(), "bike.md".into());
    let index = "- `bike.md` \"Bikes\"";
    let now = |app: &notes::Notes| -> Vec<(String, String)> {
        app.notes
            .iter()
            .map(|n| (n.filename(), n.buffer.to_text()))
            .collect()
    };
    chat.context(index, &now(&app));

    // Word for word what happened: asked to change the line about Magniff,
    // which is the fifth, it wrote the third. The third was Alice's.
    let change = Change {
        file: Some("bike.md".into()),
        what: What::Edit {
            from: 3,
            to: 3,
            text: "- Magniff's bike is black.".into(),
        },
        state: Some(true),
    };
    app.apply_change(&change);
    app.took_up_for_test(&change, &mut chat);

    let text = app.notes[i].buffer.to_text();
    assert!(text.contains("Magniff's bike is black"), "{text}");
    assert!(
        !text.contains("Alice"),
        "the app applied it as asked: {text}"
    );

    // And the next question says what the edit actually did, so the model can
    // see that it took out a line it never meant to.
    let (_, _, moved) = chat.context(index, &now(&app));
    let said = moved.expect("an edit that did something else must come back");
    assert!(said.contains("-- Alice's bike is green."), "{said}");
    assert!(said.contains("+- Magniff's bike is black."), "{said}");

    // A whole file it wrote itself is read back to it as well, as its own
    // doing and not as news: the block it wrote has no numbers in the
    // margin, and the next edit is made against numbers it counted itself.
    let wrote = Change {
        file: Some("bike.md".into()),
        what: What::Write {
            text: "# Bikes\n\n- one bike.\n".into(),
        },
        state: Some(true),
    };
    app.apply_change(&wrote);
    app.took_up_for_test(&wrote, &mut chat);
    let (_, _, moved) = chat.context(index, &now(&app));
    let said = moved.expect("what it wrote is read back, numbered");
    assert!(
        said.contains("Your write to `bike.md` was applied"),
        "{said}"
    );
    assert!(!said.contains("STOP"), "its own write is not news: {said}");
    // Once: asked again with nothing moved, nothing is said.
    app.notes[i].buffer = Buffer::from_text("# Bikes\n\n- one bike.\n");
    app.took_up_for_test(&wrote, &mut chat);
    let (_, _, moved) = chat.context(index, &now(&app));
    assert!(moved.is_none(), "told twice: {moved:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_has_not_moved_is_not_sent_again() {
    use notes::chat::Chat;
    use notes::digest::fingerprint;
    let big = |seed: &str| {
        format!(
            "# Big\n\n{}\n{seed}\n",
            "a long line of prose\n".repeat(400)
        )
    };
    let files = |tail: &str| {
        vec![
            ("one.md".to_string(), big("one")),
            ("two.md".to_string(), big("two")),
            ("three.md".to_string(), big(tail)),
        ]
    };
    let index = "- `one.md`\n- `two.md`\n- `three.md`";
    let mut chat = Chat::new("home".into(), "one.md".into());
    chat.context(index, &files("three"));

    // Asked again, ten turns running, with nothing touched. Not a word of any
    // of the three may come back, and the project at the front must be the
    // same bytes it was - that is what keeps it out of the reading.
    let (_, first, _) = chat.context(index, &files("three"));
    for _ in 0..10 {
        let (_, project, moved) = chat.context(index, &files("three"));
        assert_eq!(project, first, "the project moved on its own");
        assert!(moved.is_none(), "an unmoved file was reported: {moved:?}");
    }

    // One of the three touched, and only that one is spoken about.
    let (_, project, moved) = chat.context(index, &files("three, changed"));
    assert_eq!(project, first, "the project must still not be rewritten");
    let said = moved.expect("the one that moved is reported");
    assert!(said.contains("three.md"), "{said}");
    assert!(
        !said.contains("one.md") && !said.contains("two.md"),
        "{said}"
    );
    assert!(said.contains("+three, changed"), "{said}");
    assert!(
        !said.contains("a long line of prose\na long"),
        "the file came: {said}"
    );

    // And the number that decides it is a number, not the file.
    assert_eq!(fingerprint(&big("three")), fingerprint(&big("three")));
    assert_ne!(fingerprint(&big("three")), fingerprint(&big("four")));
}

#[test]
fn two_changes_far_apart_are_two_hunks_and_not_the_whole_file() {
    use notes::chat::Chat;
    let body = |a: &str, b: &str| {
        let mut lines: Vec<String> = (1..=200).map(|n| format!("line {n}")).collect();
        lines[4] = a.to_string();
        lines[194] = b.to_string();
        lines.join("\n") + "\n"
    };
    let before = body("line 5", "line 195");
    let after = body("line 5 CHANGED", "line 195 CHANGED");
    let file = |t: &str| vec![("notes.md".to_string(), t.to_string())];
    let mut chat = Chat::new("home".into(), "notes.md".into());
    chat.context("- `notes.md`", &file(&before));

    let (_, _, moved) = chat.context("- `notes.md`", &file(&after));
    let said = moved.expect("the change is reported");

    // Both changes, both marked, both with the line they replaced.
    assert!(said.contains("-line 5\n"), "{said}");
    assert!(said.contains("+line 5 CHANGED"), "{said}");
    assert!(said.contains("-line 195\n"), "{said}");
    assert!(said.contains("+line 195 CHANGED"), "{said}");

    // Two hunks, and the hundred and eighty lines between them left alone.
    // This is the whole point: it used to send all of them, to say two.
    assert_eq!(said.matches("@@ -").count(), 2, "{said}");
    assert!(!said.contains("line 100"), "the middle came too: {said}");
    // Ten lines of file travel - two changed, two removed and added, and the
    // context around each - where the whole stretch between them used to.
    let carried = said
        .lines()
        .filter(|l| l.starts_with(['+', '-', ' ']) && !l.starts_with("@@"))
        .count();
    assert!(carried <= 12, "{carried} lines of file came: {said}");

    // Where in the file, in the numbers the margin shows and an edit takes.
    assert!(said.contains("@@ -3,5 +3,5 @@"), "{said}");
    assert!(said.contains("@@ -193,5 +193,5 @@"), "{said}");
}

#[test]
fn a_lookup_goes_back_as_something_it_was_told() {
    use notes::chat::without_bodies;
    // A date it really did ask for. What must not go back is the shape.
    let said = "<used tool=\"date\" arg=\"2024-12-23\">\n23 December 2024 is a Monday, which was \
                613 days ago - 1 year and 8 months.\n</used>\n\nEva is 613 days old.";
    let sent = without_bodies(said);

    // The answer survives: a date is a line and cannot go stale.
    assert!(sent.contains("613 days ago"), "{sent}");
    assert!(sent.contains("Eva is 613 days old."), "{sent}");
    assert!(
        sent.contains("The date tool was asked about 2024-12-23"),
        "{sent}"
    );

    // The tag does not. An assistant turn holding `<used tool=...>` teaches
    // that writing one is a thing an assistant does - and one then wrote its
    // own, with a Tuesday for a Monday and 595 days for 613, having asked
    // nothing and been told nothing.
    assert!(!sent.contains("<used"), "{sent}");
    assert!(!sent.contains("</used>"), "{sent}");
}

#[test]
fn what_a_question_was_told_stays_in_front_of_it() {
    use notes::chat::{as_sent, copyable, told, Chat};
    let mut chat = Chat::new("trip".into(), "zzqqtrip.md".into());
    chat.draft = "which notes are there?".into();
    chat.commit();
    chat.tell("Your write to `budget.md` was accepted. It now says, in full:\n\n   1 | # Budget");
    // Once: told again for the same question - a tool's answer asks it again
    // - nothing is added.
    chat.tell("something else");
    let turn = chat.turns.last().unwrap();
    let (what, question) = told(&turn.text);
    assert_eq!(
        what.map(|w| w.contains("# Budget")),
        Some(true),
        "{}",
        turn.text
    );
    assert!(!what.unwrap().contains("something else"), "{}", turn.text);
    assert_eq!(question, "which notes are there?");
    // Not on the screen, and not in the clipboard.
    assert_eq!(copyable(turn), "which notes are there?");
    // But in front of the question every time it goes to the model, in the
    // shape a correction has always arrived in.
    let sent = as_sent(&chat.turns, &[]);
    assert!(
        sent[0].starts_with("Your write to `budget.md` was accepted"),
        "{}",
        sent[0]
    );
    assert!(
        sent[0].contains("\n\n---\n\nwhich notes are there?"),
        "{}",
        sent[0]
    );
    // And it survives the file the conversation is kept in.
    let read = notes::chat::parse(&chat.to_text());
    assert_eq!(read.len(), 1);
    assert_eq!(told(&read[0].text).0, what, "{}", read[0].text);
}

#[test]
fn a_second_edit_in_the_same_reply_is_moved_along_by_the_first() {
    use notes::chat::{proposals, rebased, settle, Chat};
    // Word for word: a row put in below line 5, and line 7 changed - both
    // against the file as the model saw it. Applied one after the other, the
    // second landed on what had been line 6.
    let said = "<edit file=\"invoice.md\" after=\"5\">| 2026-05-02 | newsletter | 180 |</edit>\n<edit file=\"invoice.md\" lines=\"7\">**Total: 1380**</edit>";
    let (_, changes) = proposals(said);
    assert_eq!(changes.len(), 2);
    // The first is applied: everything below line 5 is one line further down.
    let settled = settle(said, &changes[0], true);
    let moved = rebased(&settled, "invoice.md", 5, 1);
    assert!(moved.contains("lines=\"8-8\""), "{moved}");
    // The one already applied is left as it was, and one above the change
    // would be too.
    assert!(moved.contains("after=\"5\" state=\"applied\""), "{moved}");
    let above = "<edit file=\"invoice.md\" lines=\"3\">x</edit><edit file=\"other.md\" lines=\"9\">y</edit>";
    assert_eq!(rebased(above, "invoice.md", 5, 1), above);
    // Through the conversation, as the panel does it.
    let dir = std::env::temp_dir().join(format!("notes-rebase-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a vault");
    std::fs::write(
        dir.join("invoice.md"),
        "# Invoice\n\n| Date | Item | Amount |\n| --- | --- | --- |\n| 2026-02-03 | website | 1200 |\n\n**Total: 1200**\n",
    )
    .expect("a note");
    let mut app = notes::Notes::open(dir.clone());
    app.current = app
        .notes
        .iter()
        .position(|n| n.filename() == "invoice.md")
        .expect("there");
    let mut chat = Chat::new(String::new(), "invoice.md".into());
    chat.context(
        "- `invoice.md`",
        &[(
            "invoice.md".to_string(),
            app.notes[app.current].buffer.to_text(),
        )],
    );
    chat.answered(Ok(said.into()), &dir);
    for _ in 0..2 {
        let i = chat.turns.len() - 1;
        let (_, changes) = proposals(&chat.turns[i].text);
        let next = changes
            .into_iter()
            .find(|c| c.state.is_none())
            .expect("one waiting");
        chat.turns[i].text = settle(&chat.turns[i].text, &next, true);
        app.apply_change(&next);
        app.took_up_for_test(&next, &mut chat);
    }
    let text = app.notes[app.current].buffer.to_text();
    assert!(text.contains("| newsletter | 180 |"), "{text:?}");
    assert!(text.contains("**Total: 1380**"), "{text:?}");
    assert!(
        !text.contains("**Total: 1200**"),
        "the old total is still there: {text:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_short_file_whose_lines_moved_is_told_whole() {
    use notes::chat::Chat;
    let file = |t: &str| vec![("ledger.md".to_string(), t.to_string())];
    let before = "# Ledger\n\n| a |\n| b |\n| hosting |\n| c |\n";
    let after = "# Ledger\n\n| a |\n| b |\n| c |\n";
    let mut chat = Chat::new(String::new(), "ledger.md".into());
    chat.context("- `ledger.md`", &file(before));
    chat.did("edit", "ledger.md", before, after);
    chat.wrote("ledger.md", Some(after));
    let (_, _, moved) = chat.context("- `ledger.md`", &file(after));
    let said = moved.expect("said");
    // A row taken out of a short file at the front: the whole file, numbered,
    // so the next edit counts from the right numbers.
    assert!(said.contains("in full"), "{said}");
    assert!(said.contains("   5 | | c |"), "{said}");
    // The same lines changed in place: a diff, as before.
    let changed = "# Ledger\n\n| a |\n| B |\n| c |\n";
    chat.did("edit", "ledger.md", after, changed);
    chat.wrote("ledger.md", Some(changed));
    let (_, _, moved) = chat.context("- `ledger.md`", &file(changed));
    let said = moved.expect("said");
    assert!(!said.contains("It now says"), "{said}");
}

#[test]
fn a_grep_whose_lines_have_moved_loses_its_numbers() {
    use notes::chat::as_sent;
    use notes::llm::Turn;
    let turns = vec![
        Turn {
            mine: true,
            text: "which notes mention the museum?".into(),
        },
        Turn {
            mine: false,
            text: "<used tool=\"grep\" arg=\"museum\">\n2 lines say that:\ntrip/budget.md:8: | museum tickets | 45 |\ntrip/days.md:4: Day 3: museum.\n</used>\n\nTwo notes do.".into(),
        },
        Turn {
            mine: true,
            text: "thanks".into(),
        },
    ];
    let budget = |rows: &str| format!("# Budget\n\n| Item | Cost |\n|---|---|\n{rows}");
    let days =
        "Dates: 14 to 16 October 2026\nDay 1: fly.\nDay 2: hike.\nDay 3: museum.\n".to_string();
    // As it was: line 8 is the tickets, and the answer goes back whole.
    let then = vec![
        (
            "budget.md".to_string(),
            budget("| food | 150 |\n| flights | 420 |\n| hotel | 600 |\n| museum tickets | 45 |\n"),
        ),
        ("days.md".to_string(), days.clone()),
    ];
    let sent = as_sent(&turns, &then);
    assert!(
        sent[2].contains("trip/budget.md:8: | museum tickets | 45 |"),
        "{}",
        sent[2]
    );
    // A row put in above them: line 8 is something else now, and the
    // numbers do not go back - a model renamed the food from them.
    let now = vec![
        ("budget.md".to_string(), budget("| food | 150 |\n| souvenirs | 60 |\n| flights | 420 |\n| hotel | 600 |\n| museum tickets | 45 |\n")),
        ("days.md".to_string(), days),
    ];
    let sent = as_sent(&turns, &now);
    assert!(!sent[2].contains("budget.md:8"), "{}", sent[2]);
    assert!(
        sent[2].contains("was asked about museum at that point"),
        "{}",
        sent[2]
    );
    assert!(sent[2].contains("out of date"), "{}", sent[2]);
    assert!(sent[1].contains("Two notes do."), "{}", sent[1]);
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
    assert!(sent.contains("You read `long.md` at that point."), "{sent}");
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
fn a_file_the_model_has_only_seen_in_pieces_is_shown_whole_when_it_moves() {
    use notes::chat::Chat;
    let file = |n: &str, t: &str| (n.to_string(), t.to_string());
    let big = "a line that is here to take up room\n".repeat(60);
    let start = vec![file("notes.md", &format!("# Notes\n\n{big}"))];
    let mut chat = Chat::new("home".into(), "notes.md".into());
    chat.context("- `notes.md`", &start);

    // Made by the model mid-conversation and accepted: known, not at the
    // front. Long enough that a diff would be the smaller thing to send.
    let list = format!(
        "# Shop\n\n{}",
        (1..=30)
            .map(|i| format!("- item {i}\n"))
            .collect::<String>()
    );
    chat.wrote("shop.md", Some(&list));
    let mut both = start.clone();
    both.push(file("shop.md", &list));
    let (_, _, moved) = chat.context("- `notes.md`\n- `shop.md`", &both);
    // The list of notes has a new line, and that is said; the file itself is
    // not, because the model wrote it.
    assert!(
        !moved.as_deref().unwrap_or("").contains("STOP"),
        "its own file was reported as news: {moved:?}"
    );

    // Changed by somebody else, in one line. A file at the front would get a
    // diff; this one gets the whole thing, because the model has never had
    // the whole thing in front of it.
    both[1].1 = list.replace("- item 7\n", "- item 7, changed\n");
    let (_, _, moved) = chat.context("- `notes.md`\n- `shop.md`", &both);
    let said = moved.expect("the change is reported");
    assert!(said.contains("now contains, in full"), "{said}");
    assert!(
        said.contains("item 1\n") && said.contains("item 30"),
        "{said}"
    );

    // A file that is at the front still gets the diff it always got.
    both[0].1 = both[0].1.replace("# Notes", "# Notes, renamed");
    let (_, _, moved) = chat.context("- `notes.md`\n- `shop.md`", &both);
    let said = moved.expect("the change is reported");
    assert!(said.contains("@@ "), "{said}");
    assert!(
        !said.contains("take up room\n a line"),
        "the project came whole: {said}"
    );
}

#[test]
fn a_change_we_made_is_not_reported_as_one_we_found() {
    use notes::chat::Chat;
    let file = |n: &str, t: &str| (n.to_string(), t.to_string());
    let big = "a line that is here to take up room\n".repeat(60);
    let start = vec![file("notes.md", &format!("# Notes\n\n{big}"))];
    let mut chat = Chat::new("home".into(), "notes.md".into());
    let index = "- `notes.md` \"Notes\"";
    chat.context(index, &start);

    // The model proposed a file, it was accepted, and the application says so.
    let after = {
        let mut v = start.clone();
        v.push(file("bike.md", "the bike is red\n"));
        v
    };
    chat.wrote("bike.md", Some("the bike is red\n"));
    let (_, _, moved) = chat.context(
        "- `bike.md` \"the bike is red\"\n- `notes.md` \"Notes\"",
        &after,
    );
    let said = moved.unwrap_or_default();
    assert!(
        !said.contains("STOP"),
        "a change it proposed was broken to it as news: {said}"
    );
    assert!(
        !said.contains("the bike is red\n   "),
        "and the whole file came back with it: {said}"
    );

    // Somebody else editing that same file still arrives, and arrives as the
    // line that moved rather than as the file.
    let outside = {
        let mut v = start.clone();
        v.push(file("bike.md", "the bike is green\n"));
        v
    };
    let (_, _, moved) = chat.context(
        "- `bike.md` \"the bike is green\"\n- `notes.md` \"Notes\"",
        &outside,
    );
    let said = moved.expect("an edit from outside is still reported");
    assert!(said.contains("STOP"), "{said}");
    assert!(said.contains("the bike is green"), "{said}");
    assert!(
        !said.contains("a line that is here"),
        "the project came too: {said}"
    );
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
    assert!(
        sent.contains("write is not a tool"),
        "the answer was cut into: {sent}"
    );
    // And it goes back as something it was told, not as a tag it could write.
    assert!(!sent.contains("<used"), "{sent}");
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
    assert!(moved.contains("+the tap was fixed"), "{moved}");
    // The line that went is shown as gone rather than left to be inferred.
    // That is what a diff is, and it is a line, not the file.
    assert!(moved.contains("-the tap drips"), "{moved}");
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
    assert!(
        without_bodies(waiting).contains("not answered either way"),
        "{}",
        without_bodies(waiting)
    );
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
