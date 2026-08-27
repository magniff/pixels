//! Entry point. See `lib.rs` for the application itself.

use notes::{config, frame, shots, Notes};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `--shots` renders every screenshot offscreen and exits. It lives in the
    // app rather than in an example because it drives the real application: if
    // it can be driven headlessly, so can a test.
    if std::env::args().any(|a| a == "--shots") {
        shots::run()?;
        return Ok(());
    }

    // `--e2e` drives the whole application against the real model and checks
    // what it did to the vault. See `tools/e2e.sh`, which makes the sandbox it
    // expects to be run in.
    if std::env::args().any(|a| a == "--e2e") {
        let failed = notes::e2e::run()?;
        std::process::exit(i32::from(failed > 0));
    }

    // `--ask <instruction>` runs one edit against whatever is on stdin and
    // prints the answer. A way to try the model, and to see what it costs,
    // without going through the interface at all.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--ask") {
        let request = args.get(i + 1).cloned().unwrap_or_default();
        let mut source = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut source)?;
        let mut backend = notes::assistant(&notes::settings::Settings::load());
        eprintln!("[{}]", backend.name());
        let started = std::time::Instant::now();
        // The same numbers the block shows, on the line it is working on, so
        // a run from the shell says what it is doing too.
        // One line, rewritten in place, which is what a terminal has instead
        // of a panel.
        struct Line;
        impl notes::llm::Watcher for Line {
            fn tick(&mut self, p: notes::llm::Progress, _said: &str) {
                eprint!(
                    "\r[{} {} tokens, {:.0}/s, {:.1}s]   ",
                    if p.deliberating {
                        "thinking"
                    } else {
                        "writing"
                    },
                    p.written,
                    p.rate(),
                    p.elapsed.as_secs_f32()
                );
            }

            fn carry_on(&self) -> bool {
                true
            }
        }
        let mut tick = Line;
        // The same surroundings the editor sends: every note in the vault as
        // one line each, and - when the passage can be found in one of them -
        // that note with the passage marked where it sits. Found rather than
        // named, so a run from the shell asks exactly what a selection asks.
        let vault_dir = notes::notes_dir();
        let library = notes::read_vault(&vault_dir);
        let mut ask = notes::llm::Ask {
            request,
            vault: notes::digest::vault(&library),
            ..Default::default()
        };
        let needle = source.trim();
        for note in &library {
            let whole = note.buffer.to_text();
            if let Some(at) = whole.find(needle).filter(|_| !needle.is_empty()) {
                ask.file = note.filename();
                ask.within = notes::digest::marked(&whole, at, at + needle.len());
                break;
            }
        }
        ask.source = source;
        eprintln!(
            "[{} notes in the vault{}]",
            library.len(),
            match &ask.file {
                f if f.is_empty() => String::new(),
                f => format!(", the passage is in {f}"),
            }
        );
        let reply = backend.edit(&ask, &mut tick);
        eprintln!();
        match reply {
            // Folded the same way the editor folds it, so this prints what a
            // note would actually get rather than what the model said.
            Ok(text) => println!("{}", notes::llm::to_ascii(&text)),
            Err(e) => eprintln!("error: {e}"),
        }
        eprintln!("[{:.1}s]", started.elapsed().as_secs_f32());
        return Ok(());
    }

    pixui::run(config(), Notes::open(notes::notes_dir()), frame)
}
