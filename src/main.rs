//! Entry point. See `lib.rs` for the application itself.

use std::path::PathBuf;

use notes::{config, frame, Notes};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Notes live in ./notes by default; point PIXUI_NOTES_DIR somewhere else to
    // use a real vault.
    let dir = std::env::var("PIXUI_NOTES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("notes"));

    pixui::run(config(), Notes::open(dir), frame)
}
