#![cfg(feature = "llm")]
use notes::llm::{Ask, Assistant, Turn};
#[test]
fn trace() {
    let mut a = Assistant::spawn(Box::new(notes::llm::local::Local::new(
        format!("models/{}", std::env::var("MODEL").unwrap()),
        "You are the assistant built into a markdown note app.".into(),
    )));
    let within = "The kitchen tap drips at night and the sound carries. ".repeat(380);
    for q in [
        "say ok",
        "what day of the week is it?",
        "and what time is it?",
    ] {
        eprintln!("\nQ {q}");
        a.ask(Ask {
            turns: vec![Turn {
                mine: true,
                text: q.into(),
            }],
            within: Some(within.clone()),
            tools: notes::tools::available(false),
            ..Default::default()
        });
        let t = std::time::Instant::now();
        loop {
            if a.poll().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        eprintln!("  took {:?}", t.elapsed());
    }
}
