#![cfg(feature = "llm")]
//! One model, one battery. `MODEL=<file> cargo test --release --test tmp_bench -- --nocapture`
use notes::llm::{Ask, Assistant, Progress, Turn};
use std::time::Instant;

fn project() -> String {
    // Something the size of a real project, so prefill is the real cost.
    format!(
        "## notes.md\n\n{}\n\n## plans.md\n\nBuy a bicycle. The blue one from the shop on Almirante Reis.\n",
        "The kitchen tap drips at night and the sound carries. ".repeat(380)
    )
}

struct Bench {
    a: Assistant,
    tools: Vec<notes::llm::Tool>,
}

impl Bench {
    fn ask(&mut self, turns: Vec<Turn>, with_tools: bool) -> (String, Vec<String>, f32, u128) {
        self.a.ask(Ask {
            turns,
            within: Some(project()),
            vault: "notes.md - rough notes\nplans.md - things to buy\n".into(),
            file: "notes.md".into(),
            tools: if with_tools {
                self.tools.clone()
            } else {
                Vec::new()
            },
            ..Default::default()
        });
        let t = Instant::now();
        let mut last = Progress::default();
        let reply = loop {
            let p = self.a.progress();
            if p != Progress::default() {
                last = p;
            }
            if let Some(r) = self.a.poll() {
                break r;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        let ms = t.elapsed().as_millis();
        let said = reply.unwrap_or_else(|e| format!("<failed: {e}>"));
        let (prose, looked) = notes::chat::lookups(&said);
        let used = looked
            .iter()
            .map(|l| format!("{}({})", l.tool, l.arg))
            .collect();
        (prose, used, last.rate(), ms)
    }
}

fn one(text: &str) -> Vec<Turn> {
    vec![Turn {
        mine: true,
        text: text.into(),
    }]
}

#[test]
fn battery() {
    let file = std::env::var("MODEL").unwrap();
    let t = Instant::now();
    let mut b = Bench {
        a: Assistant::spawn(Box::new(notes::llm::local::Local::new(
            format!("models/{file}"),
            "You are the assistant built into a markdown note app.".into(),
        ))),
        tools: notes::tools::available(false),
    };
    // First question carries the load and the first full read.
    let (_, _, _, cold) = b.ask(one("say ok"), true);
    println!("\n#### {file}");
    println!(
        "  load + first question   {cold} ms   (wall {:?})",
        t.elapsed()
    );

    // Second question over the same project: does the cache come back?
    let (_, _, rate, warm) = b.ask(one("say ok again"), true);
    println!("  second question         {warm} ms   writing {rate:.1} tok/s");

    // ---- things with a right answer ----
    let checks: &[(&str, &[&str])] = &[
        ("what is 384 * 517?", &["198528", "198,528"]),
        ("what is (12.5 + 3) / 4?", &["3.875"]),
        ("what day of the week is it?", &["Thursday"]),
        ("how many days until christmas?", &["120"]),
        (
            "what is the date today?",
            &["27 August 2026", "August 27, 2026", "2026-08-27"],
        ),
        (
            "what colour is the bicycle in my notes, and where is the shop?",
            &["blue"],
        ),
    ];
    let mut right = 0;
    for (q, want) in checks {
        let (said, used, _, ms) = b.ask(one(q), true);
        let ok = want
            .iter()
            .any(|w| said.to_lowercase().contains(&w.to_lowercase()));
        right += usize::from(ok);
        println!(
            "  [{}] {:>6}ms {q}\n        {used:?}  -> {}",
            if ok { "ok" } else { "NO" },
            ms,
            said.replace('\n', " ")
                .chars()
                .take(100)
                .collect::<String>()
        );
    }
    println!("  ---- {right}/{} with the right answer", checks.len());

    // ---- things it cannot know, where the answer is saying so ----
    for q in [
        "who wrote the notes I am looking at?",
        "what is the newest release of llama.cpp?",
    ] {
        let (said, used, _, _) = b.ask(one(q), true);
        println!(
            "  [??] {q}\n        {used:?}  -> {}",
            said.replace('\n', " ")
                .chars()
                .take(130)
                .collect::<String>()
        );
    }

    // ---- the job this app actually asks of it ----
    let (said, _, _, _) = b.ask(
        one("in notes.md, change the first line so it says the tap was fixed. propose the edit."),
        false,
    );
    let shaped = said.contains("<edit") || said.contains("<write");
    println!(
        "  [{}] proposes a change in the block format\n        {}",
        if shaped { "ok" } else { "NO" },
        said.replace('\n', " ")
            .chars()
            .take(130)
            .collect::<String>()
    );
}
