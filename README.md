# notes

A markdown note editor with vim keys, drawn one pixel at a time.

Every pixel on screen is software-rendered into a small buffer and magnified by
a whole number — the sidebar, the caret, the file dialogs and the mouse pointer
included. Nothing is borrowed from the platform.

![the editor](screenshots/editor.png)

```sh
cargo run --release
```

Notes live in `./notes` and are seeded on first run. Point `PIXUI_NOTES_DIR`
somewhere else to use a real vault.

## Two views of a note

The source view highlights markdown as you type it, and hands fenced code to
a real grammar — [syntect](https://github.com/trishume/syntect)'s, remapped
onto the sixteen colours here rather than dragging its own theme in. The
preview renders the document: paragraphs reflow, markup disappears, tables
become tables. Switch with `cmd-1` and `cmd-2` (Ctrl off macOS), with
`:preview` and `:source`, or by clicking the tabs — `<>` for the source, a
folded page for the rendering. The shortcut takes a modifier because bare
digits are vim's counts, and `2j` has to keep meaning two lines down.

| source | preview |
|---|---|
| ![source](screenshots/visual.png) | ![preview](screenshots/preview.png) |

### What it understands

Both forms of heading, ATX and underlined. Emphasis with either `*` or `_`,
single, double or triple, nesting properly so `**bold with an *italic* word**`
is both. Strikethrough. Code spans, including ones fenced by several backticks
so they can hold a backtick. Backslash escapes. Links inline, by reference
(full, collapsed and shortcut), as autolinks, and as bare URLs written in
running prose. Images. Lists — bullet, ordered, and task — nested, with items
that run over several lines and blank lines between them. Block quotes holding
anything a document can hold, including other quotes, lazily continued. Code
fenced with backticks or tildes, or indented four spaces. Tables with per-
column alignment. Horizontal rules. Hard line breaks, both ways of writing one.

Not supported, and left as plain text rather than pretended at: inline and
block HTML, footnotes and definition lists, character entities, and any script
the 5x7 ASCII font cannot draw.

Tables size to their content, fenced code keeps its own slab and is never
re-wrapped, task lists get real checkboxes, and links lose their targets.
Links in the preview are clickable: one naming another note opens it, and one
with a scheme goes to the desktop to be opened. The seeded **Markdown
showcase** note exercises the lot — open it and flip between the tabs. It is
installed into any vault that has not got it, and it doubles as the parser's
test fixture, so a construct cannot be claimed there without being parsed. Switching dissolves one view into the other: every pixel takes one
side or the other by an ordered dither, because sixteen colours have nothing
in between to fade through. The tabs themselves ride the same spring the
buttons do, so the one you pick rises out of the strip as the other sinks.

![mid-dissolve](screenshots/tab-fade.png)

In insert mode the caret closes toward its own middle and opens back out
instead of switching on and off: at eight pixels tall, a bar that simply
vanishes reads as a dropped frame, where two ends travelling to meet each other
is a blink you can watch happen. Typing restarts it at full height.

Both views are numbered down the same gutter, and both carry the same
scrollbar in the same place, so flipping between them does not shift the page.
The preview's numbers are the *source* lines its blocks were parsed from — a
rendering has no lines of its own to count, since a paragraph is one block
however many rows it wraps into, so the only number that means anything is
where the block came from. Everything that *is* a row somebody typed gets its
own number: every item of a list, every line of a code block, every row of a
table, and everything inside a quote. What cannot is a wrapped row and a blank
line, for the same reason the source view puts a tick rather than a number
beside a continuation.

The preview scrolls with the vim motions that move a page: `j k`,
`Ctrl-d Ctrl-u`, `Ctrl-f Ctrl-b`, `Ctrl-e Ctrl-y`, `gg`, `G`, space, and the
wheel. The keys that move a caret are not taken, because there is no caret
there to move — except `/`, `n` and `N`, which are vim's search and belong to
the note rather than to the view of it. A search finds a line in the source,
scrolls the preview to the block that line was parsed into, and lights up every
hit in the rendered text; the source view lights up the same pattern, so a
search made in either view is answered in both.

![a search, answered in the preview](screenshots/preview-search.png)

Headings have six levels and the font has one size, so the ladder is built out
of everything else: the top three rule themselves off — full width, full width,
then only as wide as the words — the last gives up its weight, and the colour
and the air above step down the whole way.

![the foot of the showcase, reached with G](screenshots/preview-scroll.png)

![the showcase, rendered](screenshots/showcase-preview.png)

## Panes

`cmd-e` puts the keyboard in the editor, `cmd-n` in the note list, `cmd-s` in
the search box. A ring marks the region that just
took the keyboard and fades out again; the list's own outline fades in behind
it. Nothing moves — an arrival cue that travels has to cross whatever chrome
lies between where it starts and where it stops, and that reads as flicker. In the search box, `Down` steps into the
results, and so does `Enter`, which is the key the hand is already on after
typing; both stay put when nothing matched. `Escape` clears the term — a
second one leaves. In the list, `j`
and `k` walk the notes the filter is actually showing, and `Enter` drops back
into the text. Command specifically,
not Control — Control is vim's.

| arriving | settled |
|---|---|
| ![the ring on arrival](screenshots/pane-flare.png) | ![the note list with the keyboard](screenshots/pane-notes.png) |

## Vim

| | |
|---|---|
| **Motions** | `h j k l`, `w b e`, `0 ^ $`, `gg G`, `f F t T` with `;` `,`, counts |
| **Editing** | `i I a A o O`, `x`, `D`, `C`, `p P`, `u`, `Ctrl-r` |
| **Operators** | `d c y` with any motion, doubled for linewise |
| **Text objects** | `iw aw`, `ip ap`, `i" i' i\``, `i( i[ i{ i<` and `a` variants |
| **Visual** | `v` charwise, `V` linewise, `Ctrl-v` blockwise; `I`/`A` on a block type once and apply to every row |
| **Search** | `/` `?`, `n` `N`, `*` for the word under the cursor |
| **Commands** | `:w`, `:e`, `:q`, `:qa`, `:new`, `:preview`, `:source`, `:help` |
| **Indenting** | `Tab` a level in, `Shift-Tab` a level out |
| **Assistant** | `Ctrl-a` on a visual selection, or the mark in the margin |

A new line inherits where it is. Enter in a list opens the next item already
marked up — the same bullet, the next number, an unchecked box for a task, at
the nesting it found — and `o` and `O` do the same. Enter on an item holding
nothing but its marker takes the marker away instead, because a list you cannot
leave is worse than one you have to start twice. A quote carries its bar down,
and a list inside a quote continues as both. Inside a fenced code block the
markdown rules stop applying and the code's do: the indent carries, a line
ending in `:` or an opening bracket takes one more level, and a level is four
spaces rather than two. `Tab` and `Shift-Tab` move the whole line in or out by
one level, taking the caret with them.

![o on a list item, then Tab](screenshots/auto-indent.png)

Normal mode is *parsed*, not switch-cased, because vim's grammar really is one:
`[count] operator [count] motion`. Keystrokes accumulate and are re-parsed on
each key, which is why `3dw`, `d3w`, `dd`, `2dd` and `dG` all fall out of a
single code path — and why `d` on its own is a prefix that waits rather than an
error that beeps.

![blockwise visual](screenshots/visual-block.png)

## The rest of it

The mouse works too: click to place the caret, drag to select, double-click a
note in the drawer to rename it, drag the divider to resize the sidebar.

| | |
|---|---|
| ![filtering the note list](screenshots/filter.png) | ![the open dialog](screenshots/dialog-open.png) |

The file dialogs are not the system's. They browse the filesystem, scroll, take
keyboard navigation and dim what is behind them — and they are built from the
same buttons and lists as everything else.

`Cmd`/`Ctrl` with `+`, `-` and `0` scales the whole UI, in whole pixels.
Resizing the window buys more room rather than bigger pixels.

## Built on pixui

`pixui/` is the toolkit underneath: a software rasteriser, a 5x7 bitmap font, an
immediate-mode widget set, and a swappable presenter with CPU (softbuffer) and
GPU (wgpu) backends. It is a path dependency of this app rather than the point
of the repo, and `pixui/README.md` covers it properly.

The assistant needed one thing the toolkit did not have: **layers**. Immediate
mode resolves interaction as each widget is visited, so a control drawn on top
of another is *painted* over it but asks about the pointer second — and by then
whatever is underneath has already said the click was for it. `ui.layer(rect,
…)` declares that everything inside floats above everything shallower, and
interaction is resolved against the layers the previous frame declared, which
for anything on screen long enough to be clicked is the same answer. It is what
makes the block's own buttons work while sitting inside a text view whose hit
test covers the whole pane.

```sh
cargo run --release -- --shots                 # regenerate screenshots/
cargo test --workspace                         # 281 tests
PIXUI_PROFILE=1 cargo run --release            # live frame breakdown
```

## The menu

The strip along the top is a menu. **Pixels** — a chamfered face with a caret
under the name, so it reads as something that opens rather than as a word that
happens to react to the pointer — opens onto **Settings** and **About**: what the app is and which commit it was built from, and everything
the assistant needs to be told.

| the menu | the settings |
|---|---|
| ![the Pixels menu](screenshots/menu.png) | ![settings](screenshots/settings.png) |

Settings lists the weights the app knows how to fetch, with what each is good
for and what it costs to download. **GET** fetches one — through `curl`, which
is already how this app opens a link, and which saves a TLS stack and a
certificate store for a job done at most twice in a program's life. It writes to
a `.part` file and renames it only once the file is whole, so a half-fetched
download is never mistaken for weights, and an interrupted one resumes where it
stopped. Anything else already sitting in the models folder is listed too,
whether the catalogue knows it or not.

Underneath is the system prompt, which is what the model is told before it is
told anything else. The default is in `settings::DEFAULT_PROMPT`, and every
clause of it was put there by a model getting something wrong. **DEFAULT** puts
it back.

Both settle in `~/.config/pixui-notes/settings.conf` — two keys, written by
hand, because a dependency that can parse anything is a strange price to pay for
that. `PIXUI_CONFIG` moves the file and `PIXUI_MODELS` moves the weights folder;
the screenshots above use both, so what they show does not depend on whose
machine took them.

## The assistant

Select something in visual mode and a mark appears in the right margin, level
with the last line of the selection. Press it — or `Ctrl-a`, for hands that
never left the keyboard — and the text opens up under the selection to make
room for the question.

It sits *in* the note rather than over it. A panel floating beside the
selection covers the words either side of the one thing you are trying to
judge; a block opened between the lines pushes them apart instead, scrolls with
them, and leaves the note reading as a note with a question in it.

| asked | answered |
|---|---|
| ![the block opening under the selection](screenshots/assist-open.png) | ![the diff](screenshots/assist-diff.png) |

The answer comes back as a word-level diff: struck through in red where words
went, green where they arrived, and the note is not touched until **APPLY** is
pressed. **REJECT** throws it away. Asking again refines what is on screen
rather than starting over, so "now make it shorter" means shorter than the
suggestion you are looking at.

![kept](screenshots/assist-applied.png)

A line diff would be the wrong tool: a rewritten paragraph is one line in the
source, so a line diff would say "all of it changed" and leave you reading two
paragraphs to find the three words that moved. Words are the unit the
suggestion is actually made in.

The model runs on a worker thread and is spoken to through two channels.
Nothing waits on it: a request is posted, the frame carries on drawing at sixty
a second, and the answer is collected whenever it turns up.

### Which model

Two backends behind one trait. The default build has the **rehearsal** stub,
which fixes a handful of typos and collapses runs of spaces — enough to build,
test and screenshot the whole interaction without a gigabyte of weights, and
enough that the app still has an assistant when built without one. It says
`REHEARSAL` in the panel, because a stub that does not admit to being one is a
demo pretending to be a feature.

The real one is **Qwen3**, quantised to four bits and run through llama.cpp —
on the machine in front of you, offline, no key and no account. Build it in with
`--features llm`, which compiles llama.cpp from source and so wants `cmake` and
`clang`, and fetch the weights:

```sh
mkdir -p models && curl -L -o models/Qwen3-1.7B-Q4_K_M.gguf \
  https://huggingface.co/ggml-org/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf
```

That is where it looks unless `PIXUI_MODEL` says otherwise; without it the build
falls back to the stub. Or skip the curl line and press **GET** in Settings,
which fetches the same file to the same place.

**1.7B** (1.2 GB) proofreads, tightens and rephrases well. It is poor at style:
"make it goofy" comes back with a comma moved, because at that size a vague
instruction gives it nothing to aim at and its safest guess is what you wrote.
**4B-Instruct** (2.3 GB) is the one to fetch if that matters — same prompt, same
code, and it will happily turn a sentence about bitmap fonts into a pixelated
superhero. Point `PIXUI_MODEL` at it; nothing needs rebuilding.

Which prompt a model gets is asked of the model rather than guessed from its
name: the thinking-tuned Qwen3 builds are told the thinking is already done, by
prefilling an empty `<think>` block, and the instruct-tuned ones — which share a
tokeniser but were never trained on that — are not. Handing one an empty think
block is how a perfectly good model answers with nothing at all. The weights load on the first question rather than at
startup, so opening a note never waits on them. On Apple silicon the whole
model goes to the GPU and an edit takes a second or two.

There is a way to try it without the interface at all, which is also how to see
what it costs:

```sh
echo "some prose with an error in it" | cargo run --features llm -- --ask "fix the grammar"
```

A local model is good at proofreading, tightening and rephrasing — the jobs
where being local matters most, since the note never leaves the machine. All of
them want a *concrete* instruction: "fix the grammar", "split into two
sentences", "say it as a pirate". When one finds nothing to do — because the
instruction was vague, or because the passage was already right — the block says
so and puts the question back in the field to be edited.

Whatever comes back is folded into ASCII on the way in — em dashes become two
hyphens, curly quotes straighten, emoji are dropped — because the font is 5x7
and has no glyph for any of it. A suggestion arriving as a row of missing-glyph
boxes is not a suggestion.

The model is loaded once and stays loaded: a few seconds the first time, a
fraction of one after that, against a tenth of a second to answer. That is also
why quitting the app used to
end in a page of backtrace: llama.cpp's Metal backend asserts, on the way out,
that every buffer has been freed, and nothing frees them — a process on its way
to `exit` does not unwind, and a quit from the macOS menu runs no Rust
destructor at all. The app now arranges to leave first; see
`leave_before_ggml_does`.

It is **not** good at checking facts: it has no way to look anything up, and it will
invent a correction with the same confidence it fixes a comma. That is the
reason for the diff and the two buttons.

## Not implemented

Marks, macros, named registers, regular expressions in search (patterns are
literal), tag objects, inline and block HTML, footnotes, definition lists,
character entities, and any script the built-in ASCII font cannot draw. Images
are parsed but cannot be drawn, so their alt text stands in for them.

## Licence

MIT OR Apache-2.0
