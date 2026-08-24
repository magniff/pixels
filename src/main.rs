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

    // Notes live in ./notes by default; point PIXUI_NOTES_DIR somewhere else to
    // use a real vault.
    let dir = std::env::var("PIXUI_NOTES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("notes"));

    pixui::run(config(), Notes::open(dir), frame)
}
