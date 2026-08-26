//! The few things the model can look up that it cannot know.
//!
//! A model's memory of the world stops when its training did, and it does not
//! know that it has stopped. Asked for the newest release of a project it will
//! give you a version number that sounds right and is invented; asked what the
//! weather is doing it will tell you about the climate. These are the calls
//! that answer such questions with a fact.
//!
//! Not a search engine. There is no way to search the web from a program like
//! this without either a browser or somebody's API key: Google serves a page
//! that only runs under JavaScript, DuckDuckGo's HTML endpoint stops answering
//! after the first query, and Bing hands a non-browser client whatever it feels
//! like - the same question returned the right pages once and Austrian auction
//! listings the next time. So this does not pretend. What it has instead is a
//! handful of real APIs, published to be called, that answer the questions
//! people actually ask: what is the weather, what is this thing, what is the
//! latest release, and what does that page say.
//!
//! `curl` again, as everywhere else here: it is how the weights are fetched and
//! how a link is opened, it is on every machine this runs on, and it costs one
//! process against a TLS stack and a certificate store.

use std::process::Command;

/// How long to wait for any one call, in seconds. A tool that has not answered
/// by then is a tool that is not going to, and somebody is watching a panel.
const PATIENCE: &str = "20";

/// The most of a page to keep, in characters.
///
/// Prefill is what this costs: measured on the model this ships against, 2,400
/// tokens of prompt come back in 2.4 seconds and 34,000 take ninety-eight. A
/// page is worth a few seconds and not a minute and a half, and the whole of
/// Wikipedia's article on anything is past that line.
const ROOM: usize = 24_000;

/// What every call says it is, since a program that fetches has to say so.
const AGENT: &str = "pixui-notes (a local markdown editor)";

/// Fetch a URL and hand back what it says, with the markup taken out.
pub fn fetch(url: &str) -> Result<String, String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("{url} is not a web address"));
    }
    let page = get(url)?;
    let text = readable(&page);
    if text.trim().is_empty() {
        // Said rather than returned empty. An empty tool result is the one
        // thing that reliably makes a model invent: handed nothing at all it
        // fills the gap, and handed a sentence saying there was nothing it
        // says so.
        return Err(format!("{url} came back with nothing that could be read"));
    }
    Ok(clip(&text))
}

/// What the weather is doing somewhere, right now.
pub fn weather(place: &str) -> Result<String, String> {
    let body = get(&format!(
        "https://wttr.in/{}?format=j1",
        encode(place.trim())
    ))?;
    let field = |key: &str| pick(&body, &format!("\"{key}\": \""));
    let (Some(now), Some(feels)) = (field("temp_C"), field("FeelsLikeC")) else {
        return Err(format!("no weather came back for {place}"));
    };
    let sky = body
        .split("\"weatherDesc\": [")
        .nth(1)
        .and_then(|rest| pick(rest, "\"value\": \""))
        .unwrap_or_else(|| "unclear".into());
    Ok(format!(
        "{place} right now: {now}C, {}, feels like {feels}C, wind {} km/h, humidity {}%",
        sky.trim(),
        field("windspeedKmph").unwrap_or_else(|| "?".into()),
        field("humidity").unwrap_or_else(|| "?".into()),
    ))
}

/// What an encyclopaedia has on something.
///
/// Wikipedia's own search rather than a search engine's: it is a published API
/// with no key and no rate limit worth worrying about, and on the questions
/// where DuckDuckGo's keyless endpoint answered one in six it answered five in
/// five.
pub fn wikipedia(about: &str) -> Result<String, String> {
    let body = get(&format!(
        "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit=4",
        encode(about.trim())
    ))?;
    let mut out = String::new();
    for hit in body.split("{\"ns\":0,").skip(1) {
        let Some(title) = pick(hit, "\"title\":\"") else {
            continue;
        };
        let gist = pick(hit, "\"snippet\":\"").unwrap_or_default();
        out.push_str(&format!(
            "- {title}\n  https://en.wikipedia.org/wiki/{}\n  {}\n",
            title.replace(' ', "_"),
            strip(&gist).trim()
        ));
    }
    if out.is_empty() {
        return Err(format!("wikipedia has nothing on {about}"));
    }
    Ok(out)
}

/// The newest release of a project on GitHub, named `owner/repo`.
pub fn release(repo: &str) -> Result<String, String> {
    let repo = repo
        .trim()
        .trim_start_matches("https://github.com/")
        .trim_matches('/');
    if repo.split('/').count() != 2 {
        return Err(format!("{repo} is not an owner/name pair"));
    }
    let body = get(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))?;
    let Some(tag) = pick(&body, "\"tag_name\": \"").or_else(|| pick(&body, "\"tag_name\":\""))
    else {
        return Err(format!("no releases found for {repo}"));
    };
    let when = pick(&body, "\"published_at\": \"")
        .or_else(|| pick(&body, "\"published_at\":\""))
        .unwrap_or_default();
    let notes = pick(&body, "\"body\": \"")
        .or_else(|| pick(&body, "\"body\":\""))
        .unwrap_or_default();
    // Release notes are written for a web page and padded like one; the
    // trailing acre of spaces on every line costs tokens and says nothing.
    let notes: String = tidy(&unescape(&notes)).chars().take(2000).collect();
    Ok(format!(
        "{repo} latest release: {tag}, published {when}\n\n{notes}"
    ))
}

// --------------------------------------------------------------------- plumbing

/// Ask curl for a URL, and hand back what came.
fn get(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args([
            "-sL",
            "--max-time",
            PATIENCE,
            "--max-filesize",
            "8000000",
            "-A",
            AGENT,
            "-H",
            "Accept-Language: en",
            url,
        ])
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("{url} could not be reached"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The value of `"key": "` in a lump of json, without a json parser.
///
/// These are four known APIs answering in shapes that do not move, and a
/// dependency that can parse anything is a strange price for reading four
/// fields out of them.
fn pick(body: &str, key: &str) -> Option<String> {
    let rest = body.split(key).nth(1)?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('/') => out.push('/'),
                // Carriage returns come back from anything written on a web
                // page and are one more character of nothing.
                Some('r') | Some('t') => {}
                // `\u00e9` and friends. Wikipedia's snippets are full of them,
                // and left alone they read as `u00e9` in the middle of a word.
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(c) => out.push(c),
                        None => out.push_str(&hex),
                    }
                }
                Some(other) => out.push(other),
                None => break,
            },
            c => out.push(c),
        }
    }
    Some(out)
}

/// Percent-encode the parts of a query that would otherwise end it.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The part of a page a reader came for, if it is marked.
fn main_of(html: &str) -> Option<&str> {
    for (open, shut) in [("<main", "</main>"), ("<article", "</article>")] {
        if let (Some(a), Some(b)) = (html.find(open), html.rfind(shut)) {
            if b > a {
                return Some(&html[a..b]);
            }
        }
    }
    None
}

/// A tag and everything inside it, gone.
///
/// The name has to end where the tag does. `<head` is the beginning of
/// `<header` too, and taking one for the other deletes from the top of the
/// document to the end of the first header - which on an encyclopaedia page is
/// the article.
fn without(html: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let open = format!("<{tag}");
    let shut = format!("</{tag}>");
    while let Some(at) = rest.find(&open) {
        let ends = rest[at + open.len()..]
            .chars()
            .next()
            .is_none_or(|c| c == '>' || c == '/' || c.is_whitespace());
        if !ends {
            out.push_str(&rest[..at + open.len()]);
            rest = &rest[at + open.len()..];
            continue;
        }
        out.push_str(&rest[..at]);
        match rest[at..].find(&shut) {
            Some(end) => rest = &rest[at + end + shut.len()..],
            // An unclosed one takes the remainder with it, which is what an
            // unclosed script tag means in practice anyway.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Everything between `<` and `>` taken out, and the entities that matter put
/// back.
fn strip(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    unescape(&out)
}

fn unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&nbsp;", " ")
}

/// A page as prose: the furniture dropped, the headings kept.
///
/// Not a parser. It is a page reduced to the parts a reader would read, which
/// is what the model is being handed - the navigation, the cookie notice and
/// the script tags are none of that, and they cost tokens like everything else.
pub fn readable(html: &str) -> String {
    // The article, when the page says where it is. Half of what a page weighs
    // is the furniture around what you came for, and the two marks that say
    // which is which are honoured almost everywhere.
    let mut body = main_of(html).unwrap_or(html).to_string();
    for tag in [
        "script", "style", "noscript", "svg", "head", "nav", "footer",
    ] {
        body = without(&body, tag);
    }
    // Structure worth keeping, turned into the markdown the model reads all
    // day: headings as hashes, list items as dashes, blocks as line breaks.
    for level in 1..=6 {
        body = body.replace(
            &format!("<h{level}"),
            &format!("\n\n{} <h{level}", "#".repeat(level)),
        );
        body = body.replace(&format!("</h{level}>"), &format!("</h{level}>\n"));
    }
    for open in ["<li", "<p", "<br", "<tr", "<div", "<section", "<article"] {
        body = body.replace(open, &format!("\n{open}"));
    }
    body = body.replace("<li", "\n- <li");
    let text = strip(&body);

    tidy(&text)
}

/// Whitespace collapsed: a page turned into text is mostly the blank lines its
/// markup left behind, and every one of them is a token.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank += 1;
            if blank < 2 {
                out.push('\n');
            }
            continue;
        }
        blank = 0;
        out.push_str(&line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Cut to what a request can afford, saying so where it was cut.
fn clip(text: &str) -> String {
    if text.chars().count() <= ROOM {
        return text.to_string();
    }
    let kept: String = text.chars().take(ROOM).collect();
    format!("{kept}\n\n[...the rest of this page was not read: it is longer than one question can afford...]")
}

// ------------------------------------------------------------------- the tools

/// What the model is told it can reach for.
///
/// The wording of `about` is the whole of it. Measured on the model this ships
/// against: a search tool described as "search the web and return a list of
/// results" was reached for once in four questions that needed it, and the same
/// tool described in terms of *when* it is needed - and saying plainly that the
/// model's own memory of such things is wrong - was reached for four times in
/// four, with no false alarms on the two that did not need it.
pub fn tools() -> Vec<crate::llm::Tool> {
    use crate::llm::Tool;
    vec![
        Tool {
            name: "weather",
            about: "The weather right now, anywhere in the world. Use this whenever somebody asks \
                    what it is like outside, or what the weather is doing, in any place. You have \
                    no idea what the weather is; this does. Ask once per place.",
            takes: ("place", "A town, city or region, as somebody would say it."),
        },
        Tool {
            name: "wikipedia",
            about: "Look something up in an encyclopaedia and get back a few articles with their \
                    addresses. Use it for a person, a place, a company, a piece of software, an \
                    event - anything you would want to be sure about rather than remember. Follow \
                    it with fetch to read one of the articles.",
            takes: ("about", "What to look up, in a few words."),
        },
        Tool {
            name: "release",
            about: "The newest release of a project on GitHub, with its version, its date and its \
                    notes. Use this for any question about what version something is on or what \
                    changed recently. Your own memory of version numbers is out of date and \
                    inventing one is worse than saying you looked.",
            takes: (
                "repo",
                "The project as owner/name, for example ggml-org/llama.cpp.",
            ),
        },
        Tool {
            name: "fetch",
            about: "Read one web page and get back its text. Use it for a link somebody gave you, \
                    or one that came back from another tool. It reads the page as it is now.",
            takes: ("url", "The full address, starting with https://"),
        },
    ]
}

/// Run one call, and say what came back.
///
/// A tool that fails says so in a sentence rather than returning nothing.
/// Nothing at all is the one answer that reliably makes a model invent: handed
/// an empty result it fills the gap, and handed "that page could not be read"
/// it says so.
pub fn run(name: &str, arg: &str) -> String {
    let done = match name {
        "weather" => weather(arg),
        "wikipedia" => wikipedia(arg),
        "release" => release(arg),
        "fetch" => fetch(arg),
        other => Err(format!("there is no tool called {other}")),
    };
    match done {
        Ok(text) => text,
        Err(why) => format!("That did not work: {why}."),
    }
}
