//! Answering one question, running whatever tools it asks for on the way.
//!
//! The loop lives here rather than in a backend, because it is not about
//! language models: it is about a question that took a few steps. The stub
//! answers in one and never notices there is a loop around it.

use std::sync::mpsc::Sender;

use super::reply::{calls, shown, to_ascii, without_machinery, THOUGHT_OPEN};
use super::{jotted, Ask, Backend, Progress, Reply, Turn, Used, Watcher};

/// As much of a conversation as fits, counted from the newest turn back.
///
/// `measure` says how long a set of turns comes to, in whatever unit `limit`
/// is in; it is asked as few times as the answer allows. The oldest turns go
/// first, two at a time - a question and its answer - so what is left still
/// begins with somebody asking something, which is the only shape the chat
/// templates take. The newest turn is never let go of: a question that does
/// not fit on its own is a different problem, and the caller's.
///
/// Whatever is left is told that it is not the whole story, in a line on the
/// front of its first turn, so the model does not answer "you never asked
/// about that" about something it was asked three hours ago.
pub fn fitted(turns: &[Turn], limit: usize, measure: impl Fn(&[Turn]) -> usize) -> Vec<Turn> {
    if turns.is_empty() || measure(turns) <= limit {
        return turns.to_vec();
    }
    const LEFT_OUT: &str = "[Earlier turns of this conversation have been left out to make room.]";
    // The first turn that could open the kept part: a question, at the
    // earliest, and past the pairs let go of so far.
    let mut from = 0;
    loop {
        // Two at a time, then forward to the next thing somebody asked.
        from = (from + 2).min(turns.len() - 1);
        while from < turns.len() - 1 && !turns[from].mine {
            from += 1;
        }
        let mut kept = turns[from..].to_vec();
        if let Some(first) = kept.first_mut() {
            first.text = format!("{LEFT_OUT}\n\n{}", first.text);
        }
        if from >= turns.len() - 1 || measure(&kept) <= limit {
            return kept;
        }
    }
}

/// The most tools one question may run before it has to answer.
///
/// It was five, which is fewer than some perfectly ordinary questions need: a
/// table of the last ten Christmases is ten calls to the calendar before a word
/// can be written, and asking for one got "I looked several things up and did
/// not get to an answer" - having thrown away all five lookups on the way out.
/// Twelve covers that, and what happens at the ceiling matters more than where
/// the ceiling is: see `finish`.
///
/// A cap rather than a hope. A model that cannot find what it wants will search
/// for it again, and again, and there is somebody waiting at the end of it.
const STEPS: u8 = 12;

/// Answer one question, running whatever tools it asks for on the way.
///
/// The loop lives here rather than in a backend, because it is not about
/// language models: it is about a question that took a few steps. The stub
/// answers in one and never notices there is a loop around it.
/// The worker's end of a question in flight: it reports, and it listens.
struct Watching<'a> {
    beat: &'a Sender<Progress>,
    words: &'a Sender<String>,
    stop: &'a std::sync::atomic::AtomicBool,
    /// Which tool call this is, which the backend knows nothing about.
    step: u8,
    /// Words sent so far, so a stream is not the same sentence over and over.
    sent: usize,
    at: Progress,
}

impl Watcher for Watching<'_> {
    fn tick(&mut self, at: Progress, said: &str) {
        self.at = Progress {
            steps: self.step,
            ..at
        };
        // A closed channel means the application has gone; the answer itself
        // will notice on the way out.
        let _ = self.beat.send(self.at);
        // Whole words only. A stream that redraws on every token spends more
        // of the machine on drawing half a word than on writing the next one,
        // and a word appearing is what reads as writing anyway.
        //
        // Counted rather than looked for at the end: what arrives here has
        // been through the tidying that takes the model's markers off, and
        // that trims, so the text never ends in the space that would have said
        // a word had finished.
        let said = without_machinery(said);
        // A thought still being written is not shown, and is said to be
        // thinking rather than writing. What comes after the thought is the
        // answer, and that is shown as it arrives. In any of the spellings a
        // model thinks in: what is being streamed has not been through the
        // tidying that takes a finished thought off.
        let thinking = [
            THOUGHT_OPEN,
            "<think>",
            "<|channel>thought",
            "<|channel|>analysis",
        ]
        .iter()
        .filter_map(|mark| said.find(mark))
        .min();
        let (mut said, thinking) = match thinking {
            Some(at) => (said[..at].trim().to_string(), true),
            None => (said, false),
        };
        // A tag half typed is not a word. Anything from the last `<` with no
        // `>` after it is a tag still arriving, and is held back until it has.
        if let Some(at) = said.rfind('<') {
            if !said[at..].contains('>') {
                said.truncate(at);
            }
        }
        if thinking && !self.at.deliberating {
            self.at.deliberating = true;
            let _ = self.beat.send(self.at);
        }
        let words = said.split_whitespace().count();
        if words > self.sent {
            self.sent = words;
            let _ = self.words.send(said.to_string());
        }
    }

    fn carry_on(&self) -> bool {
        !self.stop.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub(super) fn answer(
    backend: &mut dyn Backend,
    ask: Ask,
    beat: &Sender<Progress>,
    words: &Sender<String>,
    stop: &std::sync::atomic::AtomicBool,
) -> Reply {
    let mut turns = ask.turns.clone();
    // What moved since the project was written out goes in front of the
    // question, once, here - and not in the backend, where it went in front
    // of whichever turn happened to be last. A question that reaches for a
    // tool is asked again with the tool's answer as the last turn, and the
    // correction was moving with it: the model was shown "STOP, the files
    // below have changed" stapled to a tool response, and the question it had
    // been in front of a moment earlier had lost it. Every question of that
    // kind was also read from the cache twice over, because the turn it moved
    // out of no longer matched the turn that had been read.
    if let (Some(moved), Some(last)) = (&ask.since, turns.last_mut()) {
        if last.mine {
            last.text = format!("{moved}\n\n---\n\n{}", last.text);
        }
    }
    let ask = Ask { since: None, ..ask };
    let mut used: Vec<Used> = Vec::new();
    let mut step = 0u8;
    // What it has said so far, across every pass.
    //
    // A question in three parts is answered in three passes, and each pass
    // writes its part and then reaches for the next tool. Only the last pass
    // used to survive: asked for a product, a weekday and a day count, the
    // conversation showed "3. 120 days until the next 25 December" and
    // nothing else. The first two answers had been written and thrown away.
    let mut so_far: Vec<String> = Vec::new();
    loop {
        let mut watch = Watching {
            beat,
            words,
            stop,
            step,
            sent: 0,
            at: Progress {
                steps: step,
                ..Progress::default()
            },
        };
        let asked = Ask {
            turns: turns.clone(),
            ..ask.clone()
        };
        let said = to_ascii(&backend.edit(&asked, &mut watch)?);
        jotted(&format!("REPLY ({} chars)", said.len()), &said);
        let at = watch.at;
        // Stopped: what it had got to is the answer. Half a paragraph you
        // asked it to stop writing is more use than nothing, and it is what
        // you were looking at when you pressed the button.
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(assembled(&used, &so_far, &shown(&said)));
        }

        let asked_for = if ask.tools.is_empty() {
            Vec::new()
        } else {
            calls(&said)
        };
        if asked_for.is_empty() {
            // Whatever it looked up goes in front of what it said, so the
            // answer arrives with its working. Anything that looked like a
            // call and was not one comes out here rather than being read as
            // prose: a half-written block is not something to show anybody.
            return Ok(assembled(&used, &so_far, &shown(&said)));
        }
        // The ones worth running: not already answered, and not asked twice in
        // the same breath. Both happen, and the second one badly - asked for a
        // percentage of a lifetime, a model wrote the same sum forty-four
        // times in one reply, and every one of them ran, because the check for
        // going round in circles only ever looked at the first.
        let mut fresh: Vec<(String, String)> = Vec::new();
        for call in &asked_for {
            let seen = used.iter().any(|u| (&u.tool, &u.arg) == (&call.0, &call.1));
            if !seen && !fresh.contains(call) {
                fresh.push(call.clone());
            }
        }
        // Out of steps, or nothing left to ask that has not been asked. Either
        // way the thing to do is not to give up: it has been looking things up
        // all this time and the answers are sitting in the turns behind it.
        let looping = fresh.is_empty();
        if step >= STEPS || looping {
            keep(&mut so_far, &said);
            turns.push(Turn {
                mine: false,
                text: said,
            });
            return finish(
                backend, &ask, turns, used, so_far, beat, words, stop, looping,
            );
        }
        // What it wrote before reaching for the tool is part of the answer.
        keep(&mut so_far, &said);
        let _ = beat.send(Progress {
            looking: true,
            steps: step.saturating_add(1),
            ..at
        });
        // Every call it made, not only the first, and no more of them than
        // there are steps left. The calls and their answers become two more
        // turns, in the shapes the model's own template expects: what it said,
        // then a user turn holding the responses, which the template reads as
        // tool results rather than as somebody asking something new.
        let mut answers = String::new();
        for (tool, arg) in fresh {
            if step >= STEPS {
                break;
            }
            let result = crate::tools::run(&tool, &arg, &ask.file);
            answers.push_str(&format!("<tool_response>\n{result}\n</tool_response>\n"));
            used.push(Used { tool, arg, result });
            step = step.saturating_add(1);
        }
        turns.push(Turn {
            mine: false,
            text: said,
        });
        turns.push(Turn {
            mine: true,
            text: answers.trim_end().to_string(),
        });
    }
}

/// Keep the part of a pass that is prose rather than machinery.
///
/// The machinery off and the thinking off, and not the thinking put back
/// when that leaves nothing: that is for a reply that was nothing but a
/// thought, and a pass that reached for a tool was not one. It thought, and
/// then it called, and the thought was the working for the call - which came
/// out in the chat and was kept in the conversation as if it had been said.
fn keep(so_far: &mut Vec<String>, said: &str) {
    let plain = without_machinery(said);
    // The same sentence twice is one sentence: a model that repeats its
    // working before each call would otherwise say everything n times.
    if !plain.is_empty() && !so_far.contains(&plain) {
        so_far.push(plain);
    }
}

/// Everything looked up, then everything said, in the order it was said.
///
/// `last` has already had the machinery taken off it - the caller does that,
/// because only the caller knows whether what looked like machinery was any.
fn assembled(used: &[Used], so_far: &[String], last: &str) -> String {
    let mut out: String = used.iter().map(Used::written).collect();
    let mut parts: Vec<&str> = so_far.iter().map(String::as_str).collect();
    let last = last.trim();
    if !last.is_empty() && !parts.contains(&last) {
        parts.push(last);
    }
    // A reply that was nothing but a call it got wrong leaves nothing behind
    // once the tags are off it, and a turn with nothing in it reads as the
    // application having lost the answer. Say what happened instead: the
    // lookups are shown above it either way, so this is the only line missing.
    if parts.is_empty() {
        parts.push(if used.is_empty() {
            "It did not answer."
        } else {
            "It looked that up but did not say anything about it."
        });
    }

    out.push_str(&parts.join("\n\n"));
    out
}

/// One more pass, with the tools taken away.
///
/// What used to happen at the ceiling was a sentence saying it had looked
/// several things up and got nowhere, with the lookups discarded - the worst of
/// both, since the answers it needed were already in front of it and the person
/// waiting got neither them nor a reply.
///
/// So it is asked once more without any tools to reach for, which leaves it
/// nothing to do but write. The results are still in the turns, so a table of
/// ten Christmases is written from the ten answers rather than abandoned on the
/// eleventh call.
#[allow(clippy::too_many_arguments)]
fn finish(
    backend: &mut dyn Backend,
    ask: &Ask,
    mut turns: Vec<Turn>,
    used: Vec<Used>,
    so_far: Vec<String>,
    beat: &Sender<Progress>,
    words: &Sender<String>,
    stop: &std::sync::atomic::AtomicBool,
    looping: bool,
) -> Reply {
    turns.push(Turn {
        mine: true,
        text: format!(
            "{} Answer now, from what you already have above, and do not look \
             anything else up - there is nothing left to look up with. If you were \
             asked to write or change a file, write the block for it now, with the \
             values you have. Plain words and the block, nothing else.",
            if looping {
                "You have already looked that up, and the answer is above."
            } else {
                "That is as much looking up as there is time for."
            }
        ),
    });
    let mut watch = Watching {
        beat,
        words,
        stop,
        step: STEPS,
        sent: 0,
        at: Progress::default(),
    };
    let said = to_ascii(&backend.edit(
        &Ask {
            turns,
            // Nothing to reach for, so the only thing left is the answer.
            tools: Vec::new(),
            ..ask.clone()
        },
        &mut watch,
    )?);
    // There were no tools to call on this pass, so anything shaped like a call
    // was not one - it was the model carrying on out of habit. What it wrote
    // is gone with the tags either way, and handing those back instead was
    // showing somebody the inside of the thing they asked a question of.
    // Better to say plainly that there is no answer, when there is none.
    let plain = shown(&said);
    let ending = if plain.is_empty() && so_far.is_empty() {
        "I looked those up but could not put an answer together."
    } else {
        &plain
    };
    Ok(assembled(&used, &so_far, ending))
}
