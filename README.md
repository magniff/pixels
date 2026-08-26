# notes

A markdown note editor with vim keys, drawn one pixel at a time.

Every pixel on screen is software-rendered into a small buffer and magnified by
a whole number — the sidebar, the caret, the file dialogs and the mouse pointer
included. Nothing is borrowed from the platform.

![the editor](screenshots/editor.png)

```sh
cargo run --release
```

Notes live in `./notes` and are seeded on first run; `PIXUI_NOTES_DIR` points
it at a real vault.

## What it does

**Editing** — vim, parsed rather than switch-cased, so `3dw`, `d3w`, `dd` and
`dG` all fall out of one grammar.

- Motions `h j k l`, `w b e`, `0 ^ $`, `gg G`, `f F t T` with `;` `,`, counts
- Operators `d c y` with any motion, doubled for linewise
- Text objects `iw aw`, `ip ap`, `i" i' i\``, `i( i[ i{ i<` and the `a` variants
- Visual charwise, linewise and blockwise; `I`/`A` on a block types once and
  applies to every row
- Search `/` `?`, `n` `N`, `*`, lighting the hits up as the pattern is typed;
  commands `:w :e :q :qa :new :preview :source`
- Undo and redo, a register that knows charwise from linewise from blockwise
- A new line inherits its list marker, its quote, or the indent of the code it
  is in; `Tab` and `Shift-Tab` move a line a level in or out

**Markdown** — two views of the same note, dissolving into each other.

- Headings both ways, emphasis nesting properly, strikethrough, code spans,
  escapes, links inline / by reference / autolinks / bare URLs, images, nested
  lists including tasks, quotes holding anything, fenced and indented code,
  tables with per-column alignment, rules, hard breaks
- Fenced code gets real syntax highlighting, from Sublime grammars
- Links in the preview are clickable; one naming another note opens it
- Both views carry the same gutter and the same scrollbar, numbered with the
  same source lines
- `/` searches the preview and lights up every hit
- The clipboard is the system's: `y` and `d` put text on it, `p` takes whatever
  is on it, and `Cmd-c`/`Cmd-x`/`Cmd-v` are the same three for the hand that
  reaches for those instead

| finding a note | source | preview |
|---|---|---|
| ![the finder](screenshots/finder.png) | ![source](screenshots/showcase-source.png) | ![preview](screenshots/showcase-preview.png) |

| the chats about a note | one of them | a change it offered |
|---|---|---|
| ![the chats about a note](screenshots/chat-picker.png) | ![a conversation](screenshots/chat-talk.png) | ![a proposed change](screenshots/chat-diff.png) |

**Notes** — a drawer of them, filtered as you type, `Ctrl-n`/`Ctrl-p` to walk.
`Cmd-p` opens the whole library to a fuzzy search: `mdsh` finds
`markdown-showcase.md`, and the letters that answered are lit in the row.
`cmd-e`, `cmd-n` and `cmd-s` put the keyboard in the editor, the list or the
search box, and a ring says where it went. The file dialogs are not the
system's — they are built from the same widgets as everything else.

**An assistant, on your machine** — select something, press `Cmd-Enter`, and
ask for a change.

- Qwen3, Qwen3.5 or GPT-OSS 20B through llama.cpp, on the GPU where there is
  one; no key, no account, nothing leaves the machine
- Prompts are built from each model's own chat template, so a family that
  spells its turns differently is spoken to correctly
- The window is sized per request, up to whatever the model was trained to
  read; the weights are put down again after a few minutes' quiet
- `Cmd-Enter` with nothing selected opens a conversation instead: the same
  context, no passage, and every chat is filed under the note it was had about
  so it can be picked up again later. `/rename` names one; the list throws one
  away, after asking
- Asked to change the note, it proposes rather than writes: the reply carries an
  edit against numbered lines, and you get the diff with **ACCEPT** and
  **REJECT** before anything moves
- The question opens *in* the text, between the lines
- The answer arrives as a word-level diff, and the note is not touched until
  you keep it — `Cmd-Enter` again, or **APPLY**
- Asking again refines the suggestion rather than starting over
- Settings fetches the weights for you, and can be switched off entirely

![a suggestion](screenshots/assist-diff.png)

**Colour schemes** — nine, in the toolkit rather than the app, each taken from
its authors' own documentation: Warm, Midnight, Solarized Dark and Light,
Gruvbox Dark and Light, Nord, Dracula, Catppuccin Latte. `j` and `k` walk the
list and wear each one as they go.

**Fonts** — five bitmap faces, all drawn for low resolution: the toolkit's own
hand-drawn 5x7, [Creep2](https://github.com/raymond-w-ko/creep2) at 5x11,
[Tamzen](https://github.com/sunaku/tamzen-font) at 6x12,
[Cozette](https://github.com/the-moonwitch/Cozette) at 7x13, and
[Gohufont](https://github.com/hchargois/gohufont) at 8x14. They are baked
from their BDFs by `tools/bdf2rs.py`, so there is still no font engine — glyphs
are bits, laid out at authoring time and blitted at runtime. Every band, row,
gutter and control is sized from the line height, so changing the face changes
the shape of the whole app rather than overflowing it.

| choosing | worn |
|---|---|
| ![the schemes and faces](screenshots/appearance.png) | ![nord](screenshots/scheme-nord.png) |
| ![Cozette](screenshots/font-cozette.png) | ![Gohufont 14](screenshots/font-gohu.png) |

## Built on pixui

`pixui/` is the toolkit underneath: a software rasteriser, a 5x7 bitmap font,
an immediate-mode widget set, floating layers, and a swappable presenter with
CPU (softbuffer) and GPU (wgpu) backends. It draws a frame when there is a
reason to and not otherwise: a spring in flight or a blinking caret keeps a
clock running, and a window sitting still costs nothing to sit still. It is a path dependency of this app
rather than the point of the repo, and `pixui/README.md` covers it properly.

```sh
cargo run --release -- --shots      # regenerate screenshots/
cargo test --workspace              # 293 tests
PIXUI_PROFILE=1 cargo run --release # live frame breakdown
```

The model is built in by default, which means a first build compiles llama.cpp
and wants `cmake` and `clang`. `--no-default-features` leaves it out, and the
assistant becomes a stub that fixes typos. `PIXUI_MODEL` and `PIXUI_MODELS`
move the weights, `PIXUI_CONFIG` moves the settings file, and
`PIXUI_LLAMA_LOGS` lets llama.cpp narrate when something has gone wrong.

```sh
echo "teh meeting was ok" | cargo run --release -- --ask "fix the typos"
```

## Not implemented

Marks, macros, named registers (the unnamed one is the system clipboard),
regular expressions in search (patterns are literal), tag objects, inline and block HTML, footnotes, definition lists,
character entities, and any script the built-in ASCII font cannot draw. Images
are parsed but cannot be drawn, so their alt text stands in for them.

Why any of it is built the way it is lives in the code, next to the code.

## Licence

MIT OR Apache-2.0
