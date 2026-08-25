//! Entry point. See `lib.rs` for the application itself.

use std::path::PathBuf;

use notes::{config, frame, shots, Notes};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `--shots` renders every screenshot offscreen and exits. It lives in the
    // app rather than in an example because it drives the real application: if
    // it can be driven headlessly, so can a test.
    if std::env::args().any(|a| a == "--shots") {
        shots::run()?;
        return Ok(());
    }

    // `--ask <instruction>` runs one edit against whatever is on stdin and
    // prints the answer. A way to try the model, and to see what it costs,
    // without going through the interface at all.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--ask") {
        let request = args.get(i + 1).cloned().unwrap_or_default();
        let mut source = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut source)?;
        let mut backend = notes::assistant();
        eprintln!("[{}]", backend.name());
        let started = std::time::Instant::now();
        match backend.edit(&notes::llm::Ask { source, request }) {
            Ok(text) => println!("{text}"),
            Err(e) => eprintln!("error: {e}"),
        }
        eprintln!("[{:.1}s]", started.elapsed().as_secs_f32());
        return Ok(());
    }

    // Notes live in ./notes by default; point PIXUI_NOTES_DIR somewhere else to
    // use a real vault.
    let dir = std::env::var("PIXUI_NOTES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("notes"));

    pixui::run(config(), Notes::open(dir), frame)
}
