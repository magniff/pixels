//! How a family of models spells its tools, and what it is told.
//!
//! There is no one way to declare a tool or call one. Each family was trained
//! on its own spelling and argues with any other, so what a model is told is
//! in its own words - read off the chat template baked into the weights - and
//! the prompt that follows the tools is the same for all of them.

use super::Tool;

/// How a family of models spells a tool call.
///
/// There is no one way. Qwen writes `<tool_call><function=x>`, Gemma 4
/// writes `<|tool_call>call:x{a:b}<tool_call|>`, Liquid's models write a
/// Python list between `<|tool_call_start|>` and `<|tool_call_end|>` - and
/// each was trained on its own and argues with any other. Told from the chat
/// template baked into the weights, which is the one place a model says how
/// it wants to be spoken to. Everything a reply is parsed for is tried in
/// every dialect regardless; this only decides what the model is *told*.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dialect {
    #[default]
    Qwen,
    Gemma,
    Liquid,
}

impl Dialect {
    /// From the model's own chat template.
    pub fn of(template: &str) -> Self {
        if template.contains("<|turn>") {
            Self::Gemma
        } else if template.contains("<|tool_list_start|>")
            || template.contains("<|tool_call_start|>")
        {
            Self::Liquid
        } else {
            Self::Qwen
        }
    }
}

/// The tools, written out the way the model's own chat template writes them.
///
/// Copied from the template baked into the weights rather than invented. The
/// template cannot be handed a tool list through the interface this app has -
/// that lives in a part of llama.cpp the bindings do not expose - but it merges
/// the tool block and the system message into one turn, so writing the block
/// out by hand renders exactly what passing tools would have.
pub fn declare(tools: &[Tool], dialect: Dialect) -> String {
    let json = |tool: &Tool| {
        format!(
            "{{\"type\": \"function\", \"function\": {{\"name\": \"{}\", \"description\": \"{}\", \
             \"parameters\": {{\"type\": \"object\", \"properties\": {{\"{}\": {{\"type\": \"string\", \
             \"description\": \"{}\"}}}}, \"required\": [\"{}\"]}}}}}}",
            tool.name, tool.about, tool.takes.0, tool.takes.1, tool.takes.0
        )
    };
    match dialect {
        Dialect::Qwen => {
            let mut out =
                String::from("# Tools\n\nYou have access to the following functions:\n\n<tools>");
            for tool in tools {
                out.push('\n');
                out.push_str(&json(tool));
            }
            // The call format is the one baked into the weights, word for
            // word: the model obeys this shape and argues with any other. The
            // reminders that followed it there are not - four paragraphs of
            // them, saying twice over what the example already shows.
            out.push_str(
                "\n</tools>\n\nIf you choose to call a function ONLY reply in the following format \
                 with NO suffix:\n\n<tool_call>\n<function=example_function_name>\n         <parameter=example_parameter_1>\nvalue_1\n</parameter>\n</function>\n</tool_call>\n\n         Several calls at once means several such blocks. Reasoning may come before a call, \
                 never after. If no function fits, answer normally and do not mention functions.",
            );
            out
        }
        // Gemma 4 declares each tool between `<|tool>` and `<tool|>` in the
        // system turn, and calls one as `<|tool_call>call:name{arg:value}`.
        Dialect::Gemma => {
            let mut out = String::from("You have these tools:\n");
            for tool in tools {
                out.push_str(&format!("<|tool>{}<tool|>\n", json(tool)));
            }
            // Told not to ask. Asked what colour the roses in another note
            // were, this family answered that it could not see that note
            // and offered to read it if asked - with the tool that reads
            // notes right there in front of it. It reads when told to read;
            // it needs telling that it may without being told.
            out.push_str(
                "\nTo use one, write exactly: <|tool_call>call:example_function_name{example_parameter_1:the value}<tool_call|>\n\
                 Several at once means several such calls. Use a tool whenever it would help, \
                 without asking permission first - reading a note is never something to ask about. \
                 If no tool fits, answer normally.",
            );
            out
        }
        // Liquid's models list their tools between `<|tool_list_start|>` and
        // `<|tool_list_end|>` and call them as a Python list.
        Dialect::Liquid => {
            let listed: Vec<String> = tools.iter().map(json).collect();
            format!(
                "List of tools: <|tool_list_start|>[{}]<|tool_list_end|>\n\n\
                 To use one, write exactly: <|tool_call_start|>[example_function_name(example_parameter_1=\"the value\")]<|tool_call_end|>\n\
                 Several at once means several entries in the list. If no tool fits, answer normally.",
                listed.join(", ")
            )
        }
    }
}

/// What the model is told it is doing when it is being talked to rather than
/// asked to rewrite something.
///
/// Not a setting, unlike the editing prompt. That one is worth exposing because
/// how you want your prose handled is personal; this one only has to describe
/// the situation, and a situation is not a preference.
pub const CHAT_PROMPT: &str = "You are talking with somebody about their own \
notes, in the editor they keep them in. Notes are organised into projects; a project \
is a folder of files, and the notes with no folder are a project too - the top of the \
vault. You can see one line about every note in the vault - its title and first line, \
which is not the note - and the whole of every file in the project they are looking \
at, with the lines numbered.

Be direct and brief. This is a side panel, not an essay. No preamble.

To change the project - and only when asked to - write one block, at the top level, \
outside any code fence:

<edit file=\"{note}\" lines=\"12-14\">the text those lines become</edit>
<edit file=\"{note}\" after=\"14\">new lines to go in below line 14</edit>
<write file=\"{note}\">everything the file says from now on</write>
<delete file=\"old.md\"></delete>
<merge into=\"kept.md\" from=\"one.md, two.md\">what the one file says</merge>

Rules for them:

- The file is named without its folder. Leave the name off an edit to mean the file \
they are looking at.
- Only the project they are looking at can be changed, and a new note is made there, \
with write. Any note in the vault can be read with the read tool; to change one in \
another project, say they should open it.
- Lines are inclusive and count from one, as in the margin. Write the replacement \
without the numbers.
- Change as few lines as the request needs. To add something, use after: the new \
lines go in below that line and nothing else moves, and after=\"0\" puts them at \
the top. Not the whole file: rewriting a file to make one addition means copying out \
every line you were not asked to touch, and what gets copied wrong is the part nobody \
was looking at.
- Use write to lay down a whole file, whether or not it is there yet. Use merge to \
fold files into one and take the rest away, in a single step - never a write plus \
deletes, because those are accepted separately and half a merge loses a note.
- One block per file. A change that reaches into several files is one block for each, \
all in the same reply - a price that appears in two notes is changed in both at once, \
not one now and one later. A sentence outside the blocks says what changed.
- Nothing happens until they accept it. Propose the change; do not announce you have \
made it.
- Asked to change nothing, write no block.";

/// A line put on the end of every question, for the models it helps.
///
/// Measured, twice, on twenty-seven scenes. Qwen3.6 with this line: twenty-six,
/// from twenty-four, and the one scene it had never passed - a list sorted
/// after somebody else changed it - passed both times. Gemma 4 with it:
/// twenty-three, from twenty-five, reasoning its way to a year of 366 days
/// and a wrong share it had both numbers for. The same line, opposite
/// effects; so it goes on one family's questions and not the other's, and
/// the backend that knows which family it is talking to is the one that puts
/// it there. On every question, not only the newest, so the conversation
/// reads the same from one prompt to the next and stays in the cache.
///
/// Its own thinking channel was tried as well and lost on both: Qwen
/// overran the room it was given and came back twenty of twenty-seven;
/// Gemma held its score and took two to four times as long.
pub const THINK_FIRST: &str =
    "Think it through carefully first, between <thinking> and </thinking>, \
then reply.";

impl Dialect {
    /// Whether this family answers better for being asked to think first.
    pub fn thinks_first(self) -> bool {
        matches!(self, Dialect::Qwen)
    }
}
