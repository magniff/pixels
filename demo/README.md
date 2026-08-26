# Trying the wider context

`editor-notes.md` is an ordinary note — rough, written fast, the kind that has
vague back-references in it because the person writing it knew what they meant.
That is what makes it a test. Four of its lines cannot be rewritten correctly
from the line alone.

Copy it into your vault and open it:

```sh
cp demo/editor-notes.md notes/
```

Select each passage, press `Cmd-Enter`, and give the instruction. Everything
below was checked against Qwen3.5 9B.

| select | ask | needs |
| --- | --- | --- |
| `Anyway it all works and looks fine.` | say what "it all" means here | the whole note |
| `Cut some of the colour schemes…` (both lines) | say how many there are | a count three sections up |
| `I keep forgetting which one.` | name the note it means, by filename | **another note entirely** |
| `That is the reason the whole thing can be done this way.` | say what "this way" means | the paragraph above |

The third row is the one that proves the most. The filename it answers with
appears **nowhere in this note** — the only place it exists is the one-line
summary of every note in the vault, so a correct answer cannot have come from
anywhere else. Before this change the same question got a plausible invention.

The same four can be run without the interface, which is how they were checked:

```sh
printf 'I keep forgetting which one.' \
  | cargo run --release -- --ask "name the note it means, by filename"
```

`--ask` finds the passage in the vault by searching for it, so it sends exactly
what a selection sends: the note around it, marked in place, and the vault list.
It prints which note it found the passage in, or nothing if it found none.

## What a failure looks like

The risk in handing a model a whole note and asking it to improve one line is
that it improves the note. If a reply comes back with more than the passage —
a heading, the paragraph after it, one of the `<selection>` markers — that is
worth reporting ahead of any wrong name.
