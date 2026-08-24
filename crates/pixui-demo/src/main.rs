//! Entry point. See `lib.rs` for the actual application.

use pixui::Config;
use pixui_demo::{frame, theme, Demo};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pixui::run(
        Config::new("pixui // control deck", 384, 240)
            .with_scale(3.0)
            .adaptive()
            .with_min_canvas(320, 200)
            .with_theme(theme()),
        Demo::default(),
        frame,
    )
}
