//! Reading what a model wrote: the calls out of it, the machinery off it.
//!
//! A reply is prose, tool calls in whichever spelling the model uses, change
//! blocks that may be wearing a call's clothes, and thinking between marks -
//! and only the prose and the blocks are for anybody to see. Everything here
//! is a pure function of the text, and every shape it knows was seen coming
//! out of a model in this application.

/// The marks a model puts its thinking between when asked to think in words.
///
/// Its own thinking channel lost on accuracy; thinking in words won and was a
/// wall of it in the panel before every answer. So the words go between marks
/// of ours, and what is between them is treated the way the channel would
/// have been: the panel says THINKING while it is being written, and neither
/// what is shown nor what is kept has it. The answer is what follows.
pub const THOUGHT_OPEN: &str = "<thinking>";

pub const THOUGHT_CLOSE: &str = "</thinking>";

/// A reply without the thinking it was asked to do in words.
///
/// Finished thoughts go; one still open is left alone, so an answer written
/// inside a thought the model forgot to close is not lost. While it is being
/// written the open one is hidden separately - see `Watching::tick`.
pub fn without_thoughts(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    // A close with no open before it is the tail of a thought begun in an
    // earlier pass: the model thought, reached for a tool mid-thought, and
    // finished the thought once the tool had answered. Everything up to it
    // is thinking.
    if let Some(end) = rest.find(THOUGHT_CLOSE) {
        if !rest[..end].contains(THOUGHT_OPEN) {
            rest = &rest[end + THOUGHT_CLOSE.len()..];
        }
    }
    while let Some(at) = rest.find(THOUGHT_OPEN) {
        out.push_str(&rest[..at]);
        match rest[at..].find(THOUGHT_CLOSE) {
            Some(end) => rest = &rest[at + end + THOUGHT_CLOSE.len()..],
            // Open at the end: the head of a thought the next pass will
            // finish, or one the model forgot to close. Either way it is a
            // thought, and it goes. `assembled` puts the words back, without
            // the marks, if that leaves nothing at all to show.
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    // A close with nothing opened before it, after the thought was done: the
    // model answered, wrote `</thinking>` again, and answered again. The
    // mark is not language, and the answer said twice is the answer.
    let out = out
        .replace(&format!("\n{THOUGHT_CLOSE}"), "")
        .replace(THOUGHT_CLOSE, "");
    let out = out.trim();
    if let Some((first, second)) = out.split_once("\n\n") {
        if first.trim() == second.trim() {
            return first.trim().to_string();
        }
    }
    out.to_string()
}

/// What of a reply is shown: the machinery off, the thinking off - unless
/// the thinking was all there was.
///
/// A reply that had its answer inside a thought it never closed would come
/// out as nothing, and "it did not answer" is worse than the working. So a
/// reply with nothing left once its thoughts are gone shows its thoughts,
/// with the marks off.
pub fn shown(said: &str) -> String {
    let plain = without_machinery(said);
    if plain.is_empty() && said.contains(THOUGHT_OPEN) {
        return without_machinery(&said.replace(THOUGHT_OPEN, "").replace(THOUGHT_CLOSE, ""));
    }
    plain
}

/// A reply with the calling-out taken out of it.
///
/// What the model writes when it reaches for a tool is a block of tags, and
/// nobody asked to see that. It matters most while the answer is being watched
/// as it is written: the first thing to arrive is the machinery, and a panel
/// that shows it is a panel showing somebody the inside of the thing they asked
/// a question of.
///
/// Anything said *before* the call is kept - "let me check" is worth reading -
/// and an unterminated block takes the rest with it, since a block half written
/// is a block still being written.
pub fn without_machinery(said: &str) -> String {
    // A change block dressed as a call is put right before anything is taken
    // away, or it would be taken away with the dressing. See `unfused`.
    let mended = unfused(said);
    let said: &str = &mended;
    let mut out = String::with_capacity(said.len());
    let mut rest = said;
    loop {
        // The earliest thing that starts a call, whichever spelling it is in.
        let Some((at, opener)) = OPENERS
            .iter()
            .filter_map(|o| rest.find(o).map(|i| (i, *o)))
            .min_by_key(|(i, _)| *i)
        else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..at]);
        let after = &rest[at + opener.len()..];
        // Past the end of this call, if it has one. A block half written is a
        // block still being written, and what follows it is nothing: that is
        // the streaming case, where the tags arrive before the answer does.
        let Some(shut) = CLOSERS
            .iter()
            .filter_map(|c| after.find(c).map(|i| (i + c.len(), *c)))
            .min_by_key(|(i, _)| *i)
            .map(|(i, _)| i)
        else {
            break;
        };
        rest = &after[shut..];
    }
    // Whatever is left of the wrapping, for the spellings that arrive without
    // their opening or in an order nobody predicted. None of it is language.
    for stray in STRAYS {
        out = out.replace(stray, "");
    }
    without_thoughts(&without_keyed_calls(&without_bare_calls(&out)))
}

/// What a model writes to begin reaching for a tool.
///
/// More than one spelling, because more than one family is spoken to here and
/// each was trained on its own. The list is what has actually been seen coming
/// out of a model in this application, not everything that exists.
const OPENERS: &[&str] = &[
    "<tool_call",
    "<function=",
    "<|tool_call_start|>",
    "<|tool_call>",
    "[TOOL_CALL]",
    "<read",
];

/// And what ends one.
const CLOSERS: &[&str] = &[
    "</tool_call>",
    "</function>",
    "<|tool_call_end|>",
    "<tool_call|>",
    "[/TOOL_CALL]",
    "</read>",
];

/// Leftovers: a closing tag whose opening never came, and the parameter
/// wrapping that sits inside a call and sometimes outlives it.
const STRAYS: &[&str] = &[
    "</read>",
    "</tool_call>",
    "<tool_call|>",
    "<|tool_call>",
    "</function>",
    "</parameter>",
    "<|tool_call_end|>",
    "<|tool_call_start|>",
    "[/TOOL_CALL]",
    "[TOOL_CALL]",
];

/// A block whose attributes arrived as a call's parameters.
///
/// The third shape of the same confusion, and the one that turns up once the
/// model has both blocks and tools well in mind:
///
/// ```text
/// <write>
/// <parameter=file>
/// facts.md
/// </write>
/// <parameter=content>
/// what the file should say
/// ```
///
/// The name and the body are both there, wearing the wrong tags, and the
/// closing one has wandered into the middle. Read for what it plainly means -
/// a write of `facts.md` with that text - rather than thrown away, which is
/// what happened: the change was never offered and nothing was written.
fn with_parameters(reply: &str) -> String {
    let mut out = reply.to_string();
    for kind in ["edit", "write", "create", "delete", "merge"] {
        let open = format!("<{kind}>");
        if !out.contains(&open) || !out.contains("<parameter=file>") {
            continue;
        }
        let Some(named) = out
            .split("<parameter=file>")
            .nth(1)
            .and_then(|rest| rest.split('<').next())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
        else {
            continue;
        };
        let body = out
            .split("<parameter=content>")
            .nth(1)
            .map(|rest| rest.split("</parameter>").next().unwrap_or(rest).trim())
            .unwrap_or("")
            .to_string();
        // Everything from the opening tag onwards was the block; what came
        // before it is whatever the model said first.
        let before = out.split(&open).next().unwrap_or("").to_string();
        // A name and no body is a call that was answered "write is not a
        // tool", not a proposal - the model went on to write the block
        // properly in the next pass. Mended into an empty block, it sat in
        // front of the real one as a second write of the same file, and two
        // writes are not a merge: a write and two deletes were offered one
        // at a time, and the deletes went first.
        if body.is_empty() && kind != "delete" {
            out = before;
            continue;
        }
        out = format!("{before}<{kind} file=\"{named}\">\n{body}\n</{kind}>");
    }
    out
}

/// Put right a change block written as if it were a tool call.
///
/// Applied before the machinery is taken off, and again as a reply is stored.
/// Before, because the scrub removes a whole call from opening tag to closing
/// one - and a change block wearing a call's tags is inside that span. Asked
/// to add two children to a note, the model looked both their birthdays up,
/// wrote the edit, wrapped it in a call, and the whole thing was deleted as
/// wiring: two correct lookups followed by "it looked that up but did not say
/// anything about it".
///
/// The two are told apart by nothing but their tags, and a model handed both in
/// one system prompt sometimes fuses them. Asked to make a file, Qwen3.5 wrote
///
/// ```text
/// <tool_call>
/// <function=write file="kettle.md">
/// the kettle is broken.
/// </write>
/// </tool_call>
/// ```
///
/// which is this app's own write block wearing a call's opening tag. The intent
/// is not in doubt - the closing tag says which block was meant - so it is read
/// as the block it plainly is rather than thrown away, which is what happened
/// before: the reply came out empty and the conversation showed a blank turn.
///
/// Only the four kinds that exist, and only when the matching close is there,
/// so a genuine call to a tool that happens to share a name is left alone.
pub fn unfused(reply: &str) -> std::borrow::Cow<'_, str> {
    if !reply.contains("<function=") {
        // The bare shape needs no unfusing, only its parameters read.
        if reply.contains("<parameter=file>") {
            return std::borrow::Cow::Owned(with_parameters(reply));
        }
        return std::borrow::Cow::Borrowed(reply);
    }
    let mut out = reply.to_string();
    let mut mended = false;
    for kind in ["edit", "write", "create", "delete", "merge"] {
        let opened = format!("<function={kind}");
        if !out.contains(&opened) {
            continue;
        }
        out = out.replace(&opened, &format!("<{kind}"));
        mended = true;
        // The close is fused in more than one way - `</write>` one time,
        // `</parameter></function>` the next - so whichever wrapper closes
        // first stands in for the one that was meant.
        if !out.contains(&format!("</{kind}>")) {
            for wrapper in ["</parameter>", "</function>", "</tool_call>"] {
                if out.contains(wrapper) {
                    out = out.replacen(wrapper, &format!("</{kind}>"), 1);
                    break;
                }
            }
        }
    }
    if !mended {
        return std::borrow::Cow::Borrowed(reply);
    }
    out = with_parameters(&out);
    for wrapper in ["<tool_call>", "</tool_call>", "</function>", "</parameter>"] {
        out = out.replace(wrapper, "");
    }
    std::borrow::Cow::Owned(out)
}

/// The tool call a reply is making, if it is making one.
///
/// The model's own format, which is why the parsing is this short: the tag and
/// one parameter, exactly as the chat template told it to write them.
pub fn called(reply: &str) -> Option<(String, String)> {
    calls(reply).into_iter().next()
}

/// Every call a reply is making, in the order it made them.
///
/// More than one, because models ask for more than one at a time: given three
/// things to find out, they write all three blocks in a single reply. Reading
/// only the first meant the rest were dropped on the floor - and since the
/// reply was then all machinery with the first call consumed, what the person
/// waiting saw was the raw tags of the calls that never ran.
pub fn calls(reply: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = reply
        .split("<function=")
        .skip(1)
        .filter_map(one_call)
        .collect();
    out.extend(gemma_calls(reply));
    out.extend(liquid_calls(reply));
    out.extend(bare_calls(reply));
    out.extend(asked_to_read(reply));
    out
}

/// A fifth spelling: the tool's name as a tag of its own, with the parameter
/// under it and nothing around it - `<calc>\n<parameter=expression>1 + 1`.
///
/// Neither the call wrapper nor `<function=`, so nothing heard it; the sum
/// was never worked out, and the tag was shown in the chat as the answer.
/// A tag immediately followed by a parameter is a call to whatever the tag
/// names. Only a bare tag: `<function=x>` and the block kinds are heard
/// elsewhere, and a tag with attributes is a block.
fn bare_calls(reply: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, value) in bare_spans(reply) {
        out.push((name, value));
    }
    for (name, value, _, _) in keyed_spans(reply) {
        out.push((name, value));
    }
    out
}

/// The names a bare tag cannot be a call to: the wrapping, and the blocks.
const NOT_A_TOOL: &[&str] = &[
    "tool_call",
    "function",
    "edit",
    "write",
    "create",
    "delete",
    "merge",
    "read",
    "parameter",
    "thinking",
    "think",
];

/// A tag on its own, with the parameter written as a line under it -
/// `<calc>\nexpression: (420 / 1205) * 100\n</calc>` - and where it starts
/// and ends.
///
/// A sixth spelling, seen forty questions into one conversation: the name
/// of the parameter and a colon, with the `<parameter=` dressing gone the way
/// the `<function=` dressing went before it. Nothing heard it, the share was
/// never worked out, and the tag was the answer. Only a bare tag with a
/// closing one to match, and only a body that is one `name: value` or
/// `name = value` - anything else in the same shape is a thing being said.
fn keyed_spans(reply: &str) -> Vec<(String, String, usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = reply[from..].find('<') {
        let at = from + at;
        from = at + 1;
        let Some(close) = reply[at..].find('>') else {
            break;
        };
        let name = &reply[at + 1..at + close];
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || NOT_A_TOOL.contains(&name)
        {
            continue;
        }
        let body_at = at + close + 1;
        let shut = format!("</{name}>");
        let Some(len) = reply[body_at..].find(&shut) else {
            continue;
        };
        let raw = &reply[body_at..body_at + len];
        // On lines of its own under the tag, the way a call is laid out.
        // `<b>Note: this is bold</b>` is a sentence with a colon in it.
        if !raw.starts_with('\n') && !is_tool(name) {
            continue;
        }
        let body = raw.trim();
        // One key, one value, on as many lines as the value takes - or, for
        // a tag that is the name of a tool there is, the value on its own:
        // `<date>2026-10-14</date>`, which is how the same conversation put
        // it ten questions later, dressed like a block.
        let keyed = body
            .split_once([':', '='])
            .map(|(k, v)| (k.trim(), v.trim()))
            .filter(|(k, _)| {
                !k.is_empty()
                    && !k.contains(char::is_whitespace)
                    && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        let value = match keyed {
            Some((_, value)) => value,
            None if is_tool(name) => body,
            None => continue,
        };
        if value.is_empty() || value.contains('<') {
            continue;
        }
        let end = body_at + len + shut.len();
        out.push((name.to_string(), value.to_string(), at, end));
        from = end;
    }
    out
}

/// Whether a tag is the name of a tool that exists.
fn is_tool(name: &str) -> bool {
    crate::tools::available(true).iter().any(|t| t.name == name)
}

/// A keyed call taken off what is shown, tag to closing tag.
fn without_keyed_calls(text: &str) -> String {
    let spans = keyed_spans(text);
    if spans.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for (_, _, at, end) in spans {
        out.push_str(&text[from..at]);
        from = end;
    }
    out.push_str(&text[from..]);
    out
}

/// Every bare call in a reply: its name, its value, and where it starts and
/// ends, so it can be run and taken off what is shown.
fn bare_spans(reply: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = reply;
    while let Some(at) = rest.find("<parameter=") {
        let before = rest[..at].trim_end();
        let tag = before.rsplit('<').next().unwrap_or("");
        let name = tag.trim_end_matches('>').trim();
        let bare = before.ends_with('>')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && ![
                "tool_call",
                "function",
                "edit",
                "write",
                "create",
                "delete",
                "merge",
                "read",
                "parameter",
            ]
            .contains(&name);
        let after = &rest[at..];
        let value = after
            .split_once('>')
            .map(|(_, v)| v)
            .unwrap_or("")
            .split("</parameter>")
            .next()
            .unwrap_or("")
            .split(&format!("</{name}>"))
            .next()
            .unwrap_or("")
            .trim();
        if bare && !value.is_empty() {
            out.push((name.to_string(), value.to_string()));
        }
        rest = &after["<parameter=".len()..];
    }
    out
}

/// A bare call taken off what is shown: from its tag to the end of its
/// parameter, and the closing tag after that if there is one.
fn without_bare_calls(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("<parameter=") {
        let before = rest[..at].trim_end();
        let Some(tag_at) = before.rfind('<') else {
            out.push_str(&rest[..at + "<parameter=".len()]);
            rest = &rest[at + "<parameter=".len()..];
            continue;
        };
        let name = before[tag_at + 1..].trim_end_matches('>').trim();
        let bare = before.ends_with('>')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && ![
                "tool_call",
                "function",
                "edit",
                "write",
                "create",
                "delete",
                "merge",
                "read",
                "parameter",
            ]
            .contains(&name);
        if !bare {
            out.push_str(&rest[..at + "<parameter=".len()]);
            rest = &rest[at + "<parameter=".len()..];
            continue;
        }
        out.push_str(&rest[..tag_at]);
        let after = &rest[at..];
        let mut end = after
            .find("</parameter>")
            .map(|i| i + "</parameter>".len())
            .unwrap_or(after.len());
        let shut = format!("</{name}>");
        if after[end..].trim_start().starts_with(&shut) {
            end += after[end..].find(&shut).unwrap_or(0) + shut.len();
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// Gemma 4's spelling: `<|tool_call>call:name{arg:value}<tool_call|>`.
///
/// One argument, so everything between the first brace and the last is the
/// argument, and what follows its first colon is the value. Quoted or not -
/// the model writes `{when:"2024-12-23"}` as readily as `{when:2024-12-23}`.
fn gemma_calls(reply: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for piece in reply.split("<|tool_call>").skip(1) {
        let body = piece.split("<tool_call|>").next().unwrap_or(piece).trim();
        let body = body.strip_prefix("call:").unwrap_or(body);
        // The name, then the arguments in braces - or, dressed like one of
        // the change blocks it has in front of it, as an attribute after the
        // name: `call:read file="barn/hens.md"{}`, and `{}` or nothing after.
        // Three notes were asked for that way and none was read, because
        // "read file="barn/hens.md"" is no tool.
        let brace = body.find('{').unwrap_or(body.len());
        let (head, rest) = body.split_at(brace);
        let (name, attrs) = head.split_once(char::is_whitespace).unwrap_or((head, ""));
        let rest = rest.strip_prefix('{').unwrap_or(rest);
        let inner = rest.rsplit_once('}').map(|(i, _)| i).unwrap_or(rest);
        let inner = if inner.trim().is_empty() {
            attrs
        } else {
            inner
        };
        // `{when:"today"}` one time and `{file="garden/harvest.md"}` the next,
        // from the same model in the same run. Whichever comes first is the
        // one dividing the name from the value.
        // And quoted with a token of its own as often as with a quote mark:
        // `{expression:<|"|>384 * 517<|"|>}`. The calculator was handed the
        // token and said `<` was not something it could work out.
        let value = inner
            .split_once([':', '='])
            .map(|(_, v)| v)
            .unwrap_or("")
            .replace("<|\"|>", "\"")
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();
        let name = name.trim();
        if !name.is_empty() {
            out.push((name.to_string(), value));
        }
    }
    out
}

/// Liquid's spelling: `<|tool_call_start|>[name(arg="value")]<|tool_call_end|>`.
///
/// A Python call. The value is whatever sits between the first `=` and the
/// close of the call, unquoted; a list may hold several.
fn liquid_calls(reply: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for piece in reply.split("<|tool_call_start|>").skip(1) {
        let body = piece.split("<|tool_call_end|>").next().unwrap_or(piece);
        let body = body.trim().trim_start_matches('[').trim_end_matches(']');
        let mut rest = body;
        while let Some(open) = rest.find('(') {
            let name = rest[..open]
                .trim()
                .trim_start_matches(',')
                .trim()
                .to_string();
            let after = &rest[open + 1..];
            let Some(close) = after.find(')') else {
                break;
            };
            let args = &after[..close];
            let value = args
                .split_once('=')
                .map(|(_, v)| v)
                .unwrap_or(args)
                .trim()
                .trim_matches(|c| c == '"' || c == '\'');
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                out.push((name, value.to_string()));
            }
            rest = &after[close + 1..];
        }
    }
    out
}

/// Reading a note, asked for the way this application's own blocks are written.
///
/// Told a file had changed and given a tool to read it with, the model wrote
/// `<read file="bike.md"></read>` - not a tool call, but the shape of the edit
/// and write blocks it had just been taught three paragraphs earlier. Which is
/// a reasonable thing to conclude from that prompt. Nothing understood it, so
/// it went unanswered and the model fell back on what it already believed.
///
/// Answered in the shape it asked, it gets it right. So both shapes are heard.
///
/// And a fourth shape, once the model has blocks and tools both well in mind:
/// `<read>` with no name on it, the name as a call's parameter underneath,
/// and a call's closing tags. Asked to add a line to a note it had just made,
/// it wrote that to look at the note first - which was the right thing to
/// want - and nothing understood it, so the reply came out as nothing.
fn asked_to_read(reply: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = reply;
    while let Some(at) = rest.find("<read") {
        let after = &rest[at + 5..];
        let Some((head, tail)) = after.split_once('>') else {
            break;
        };
        // Only the tag itself: `<reader>` or `<ready` is a word, not a call.
        if !head.is_empty() && !head.starts_with(char::is_whitespace) {
            rest = tail;
            continue;
        }
        let quoted = head
            .split('"')
            .nth(1)
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let named = quoted.map(str::to_string).or_else(|| {
            // The name written as a parameter, up to the parameter's closing
            // tag or the next tag of any kind, whichever comes first.
            tail.split("<parameter=")
                .nth(1)
                .and_then(|p| p.split_once('>'))
                .map(|(_, v)| v.split('<').next().unwrap_or("").trim().to_string())
                .filter(|n| !n.is_empty())
        });
        if let Some(named) = named {
            out.push(("read".to_string(), named));
        }
        rest = tail;
    }
    out
}

/// One call, from the text just after its `<function=`.
fn one_call(after: &str) -> Option<(String, String)> {
    let name = after.split('>').next()?.trim().to_string();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    // Only up to the end of this call: a reply making three of them must not
    // read the second one's argument as the first one's.
    let after = after.split("</function>").next().unwrap_or(after);
    // Everything past the parameter's own `>`, up to its closing tag. Splitting
    // on `>` first would eat the `>` of `</parameter>` and leave the tag in the
    // value - which fed the tools an argument with a tag on the end, got an
    // empty result back, and had the model inventing to fill the gap.
    //
    // A call with the argument left out is still a call. It used to be
    // nothing: not a call, so stripped off as machinery, so a reply that was
    // only `<function=calc></function>` came out as "it did not answer". Run
    // with nothing, the tool says what it wanted, and the model gets to write
    // the call again with it in - which is what it does.
    let value = after
        .split("<parameter=")
        .nth(1)
        .and_then(|p| p.split_once('>'))
        .and_then(|(_, v)| v.split("</parameter>").next())
        .map(str::trim)
        .unwrap_or("");
    Some((name, value.to_string()))
}

/// A reply with the deliberation taken off, and nothing else.
///
/// Reasoning models answer in channels: the deliberation first, the answer
/// last, both in the same stream. Only the last one was asked for. The thinking
/// is dropped rather than shown — an editor that pauses to explain itself for
/// four hundred words before touching your sentence is a worse editor, however
/// interesting the four hundred words are.
///
/// This is the whole of what a *conversation* gets tidied by. The rest of
/// [`clean_reply`] is for a passage handed back - fences and quotes it was not
/// asked to wrap the passage in - and applied to a reply in a conversation it
/// took a code block off the front of an answer that was code because somebody
/// had asked for code.
pub fn without_thinking(text: &str) -> String {
    let mut out = text.trim();
    if let Some(start) = out.rfind("<|channel|>final<|message|>") {
        out = &out[start + "<|channel|>final<|message|>".len()..];
    }
    for tail in ["<|return|>", "<|end|>", "<|call|>"] {
        if let Some(cut) = out.find(tail) {
            out = &out[..cut];
        }
    }
    if let Some(start) = out.rfind("</think>") {
        out = &out[start + "</think>".len()..];
    }
    // Gemma 4 thinks between `<|channel>thought` and `<channel|>`.
    if let Some(start) = out.rfind("<channel|>") {
        out = &out[start + "<channel|>".len()..];
    }
    out.trim().to_string()
}

/// Take the answer out of whatever a model wrapped it in.
///
/// Small models announce themselves — "Here is the proofread version:" — and
/// fence things they were not asked to fence. The prompt asks for the passage
/// between `<text>` and `</text>` precisely so this can be exact rather than a
/// guess about which opening sentence is a preamble and which is the text.
/// Everything else here is a fallback for a model that ignored that.
pub fn clean_reply(text: &str) -> String {
    let out = without_thinking(text);
    let mut out = out.as_str();

    // The delimiters, when they are there. A missing closing tag still tells
    // us where the answer began, which is the half that matters.
    if let Some(start) = out.find("<text>") {
        out = &out[start + "<text>".len()..];
        if let Some(end) = out.find("</text>") {
            out = &out[..end];
        }
    }
    out = out.trim();

    // A code fence around prose, which no instruction ever quite stops.
    if let Some(rest) = out.strip_prefix("```") {
        let rest = rest.split_once('\n').map_or(rest, |(_, r)| r);
        out = rest.strip_suffix("```").unwrap_or(rest).trim();
    }

    // And a whole answer in quotes, which is not the same as an answer that
    // happens to contain them.
    if out.len() > 1
        && out.starts_with('"')
        && out.ends_with('"')
        && !out[1..out.len() - 1].contains('"')
    {
        out = &out[1..out.len() - 1];
    }
    out.trim().to_string()
}

/// Fold a reply into the alphabet the editor can actually draw.
///
/// The font is 5x7 and ASCII. A model that answers with em dashes, curly
/// quotes and a couple of party emoji is not wrong — it is writing for a
/// different screen — but every one of those lands in a note as a missing-glyph
/// box. The common punctuation has an obvious ASCII spelling and gets it; the
/// decoration - emoji, arrows, the symbols - is dropped rather than drawn as a
/// box.
///
/// Letters are kept, whatever alphabet they are in. This used to drop them
/// with the emoji, and it is applied to the whole reply, change blocks and
/// all: a note with a name like Müller or a line of Cyrillic in it, copied by
/// the model into an edit, came back with those characters gone and was then
/// written to disk that way. The editor draws a letter it has no glyph for as
/// a question mark, which is what a person's own typing gets, and a question
/// mark on screen is a different thing from a letter missing from the file.
///
/// Applied to every backend's answer, because the limit belongs to the editor
/// rather than to whichever model happened to answer.
pub fn to_ascii(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            c if c.is_ascii() || c.is_alphanumeric() => out.push(c),
            // An em dash is two hyphens in ASCII prose; a single one reads as a
            // hyphenated word and joins the two clauses it was separating.
            '\u{2014}' | '\u{2015}' => out.push_str("--"),
            '\u{2013}' | '\u{2012}' | '\u{2212}' => out.push('-'),
            '\u{2026}' => out.push_str("..."),
            '\u{2018}' | '\u{2019}' | '\u{201b}' => out.push('\''),
            '\u{201c}' | '\u{201d}' | '\u{201f}' => out.push('"'),
            '\u{00a0}' | '\u{2007}' | '\u{202f}' => out.push(' '),
            '\u{2022}' | '\u{00b7}' => out.push('-'),
            '\u{00d7}' => out.push('x'),
            _ => {}
        }
    }
    // Dropping a character can leave the space that was holding it up. Tidied
    // line by line, and past the indent: the lines are the passage's shape and
    // the indent is a list's nesting, and neither is whitespace to tidy away.
    out.lines()
        .map(|line| {
            let indent = &line[..line.len() - line.trim_start().len()];
            let words: Vec<&str> = line.split_whitespace().collect();
            format!("{indent}{}", words.join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}
