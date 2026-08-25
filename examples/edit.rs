//! One edit, timed. `PIXUI_MODEL` picks the weights.
use notes::llm::Backend;
use std::time::Instant;

fn main() {
    let config = notes::settings::Settings::default();
    let source = std::env::args().nth(1).unwrap_or_else(|| {
        "the meeting was, in my opinion, somewhat productive but we didnt \
         really decide anything concrete and i think we should probably \
         meet again next week to go over it once more."
            .to_string()
    });
    let request = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "tighten this".to_string());

    let started = Instant::now();
    let mut backend = notes::llm::local::Local::new(
        notes::llm::local::default_path(),
        config.prompt.clone(),
        config.context,
    );
    let ask = notes::llm::Ask { source, request };
    let reply = backend.edit(&ask);
    let first = started.elapsed();
    // Putting it down and picking it up again, which is what an idle editor
    // does, and the one path where ggml is most likely to object.
    backend.release();
    let again = Instant::now();
    let _ = backend.edit(&ask);
    println!(
        "--- {} : first {:?} (load included), again {:?}",
        backend.name(),
        first,
        again.elapsed()
    );
    match reply {
        Ok(text) => println!("{text}"),
        Err(why) => println!("failed: {why}"),
    }
}
