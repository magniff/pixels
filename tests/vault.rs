//! The vault on disk and in memory, the settings, and the finder and
//! dialogs that reach into it.

use notes::settings::Settings;
use notes::text::Buffer;
use notes::vim::{Vim, VimEvent};
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

fn vault_with(dir: &std::path::Path, files: &[(&str, &str)]) {
    let _ = std::fs::remove_dir_all(dir);
    for (path, text) in files {
        let at = dir.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }
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
