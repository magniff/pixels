# pixui

A chunky, warm, pixel-art immediate-mode UI toolkit for Rust, rendered entirely
on the CPU. Cross-platform via winit — macOS, Windows, Linux.

Two applications are built on it, both depending on nothing but `pixui`:

| crate | what it is |
|---|---|
| `pixui-demo` | a widget gallery — buttons, toggles, sliders, tabs, scrolling, theming |
| `pixui-notes` | a markdown note editor with vim keys, a note sidebar, and in-app file dialogs |

## The split

The whole point of this workspace is the boundary between the two crates.

| | `crates/pixui` — the library | the applications |
|---|---|---|
| **Owns** | everything identical in any app | everything that makes it *this* app |
| Windowing, event loop, present | yes | — |
| Rasterising, clipping, dithering | yes | — |
| Bitmap font, text measurement | yes | — |
| Widget look, press springs, focus ring | yes | — |
| Palette and theme definitions | yes | — |
| Screen layout | — | yes |
| Which widgets exist, and where | — | yes |
| What a click *means* | — | yes |
| Which theme is selected | — | yes |
| Scrolling, clipping, hit testing | yes | — |
| Text entry, caret, modality | yes | — |
| Drawn mouse pointer | yes | — |
| Draggable splitters | yes | which pane, and how wide |
| **Dependencies** | `winit`, `softbuffer`, `wgpu` | `pixui` — and nothing else |

That last row is the part you don't have to take on faith. Both apps have
exactly one dependency each. If any toolkit concern had leaked into application
code, they would need a second dependency to express it.

`pixui-notes` is the sharper test of the boundary, because it wants things a
widget gallery never asks for: a modal file browser, a text caret, a keyboard
owned entirely by the application. Those needs split cleanly —

- **into the toolkit** went `text_field`, `Ui::input_blocked` (which is what
  makes a modal a modal in immediate mode), `Ui::capture_keyboard`, a real
  `ctrl` modifier distinct from the platform-primary one, and faux-bold text.
- **stayed in the app** went the vim grammar, the markdown highlighter, line
  wrapping, and every call to `std::fs`.

The dialogs are the illustration: they browse the filesystem, but they are drawn
with the same panels, lists, buttons and scroll areas as everything else, and
`std::fs` appears nowhere in `pixui`.

The same boundary holds inside the library:

```
  your application          ← only ever sees the modules below
  ─────────────────────────────────────────────────────────────
  widgets                   button, toggle, slider, panel, scroll_area, ...
  ui + layout + theme       identity, hit testing, focus, scrolling, look
  canvas + font + color     the software rasteriser
  ─────────────────────────────────────────────────────────────
  app                       ← the only module that names a platform crate
   ├─ app::soft             CPU upscale  → softbuffer
   └─ app::gpu              texture upload → wgpu
```

`pixui::app` is the sole place winit, softbuffer and wgpu appear. Everything
above it speaks `Input` and `Canvas`. Two things prove that rather than assert
it: `examples/snapshot.rs` drives the identical UI with no window and no event
loop, and `tests/gpu.rs` renders through the real GPU pipeline offscreen and
compares it pixel-for-pixel against the CPU path.

Getting a finished canvas onto a screen is the one job that genuinely differs
between platforms and eras, so it sits behind a `Presenter` trait with two
implementations. Implementing that trait is all a new port needs.

## Run it

```sh
cargo run --release -p pixui-notes                     # the note editor
cargo run --release -p pixui-notes --example snapshot   # offscreen render -> snapshots/
cargo run --release -p pixui-demo                      # the widget gallery
cargo run --release -p pixui-demo --example snapshot   # offscreen render -> snapshots/
cargo run --release -p pixui-demo --example bench      # where frame time goes
PIXUI_PROFILE=1 cargo run --release -p pixui-demo      # live frame breakdown
PIXUI_BACKEND=soft ... # force the CPU presenter (default is GPU when compiled in)
PIXUI_VSYNC=0     ... # unblock present, so the profiler measures work not waiting
```

Cargo features: `soft` (default, softbuffer) and `gpu` (wgpu). The demo enables
both so the two can be compared at runtime. A library user who wants neither the
wgpu build time nor the GPU path can just take the default.

## Using it

```rust
use pixui::{Config, Tone, Ui};

struct State { clicks: u32, loud: bool }

pixui::run(
    Config::new("hello", 240, 140).with_scale(3.0),
    State { clicks: 0, loud: false },
    |ui: &mut Ui, state: &mut State| {
        let screen = ui.canvas.bounds().inset(8);
        let inner = ui.panel(screen, "HELLO");
        ui.column(inner, 4, |ui| {
            ui.label(&format!("clicks: {}", state.clicks));
            if ui.button_tone("PRESS ME", Tone::Accent).clicked {
                state.clicks += 1;
            }
            ui.toggle("loud", &mut state.loud);
        });
    },
).unwrap();
```

## pixui-notes

A markdown editor with a modal (vim-style) editing engine.

| | |
|---|---|
| **Motions** | `h j k l`, `w b e`, `0 ^ $`, `gg G`, counts (`5j`, `d2w`) |
| **Editing** | `i I a A o O`, `x`, `D`, `C`, `p P`, `u`, `Ctrl-r` |
| **Operators** | `d c y` with any motion, doubled for linewise (`dd`, `cc`, `yy`) |
| **Text objects** | `iw aw`, `ip ap`, `i" i' i\``, `i( i[ i{ i<` and `a` variants — so `diw`, `ciw`, `ci"`, `da(`, `dip` |
| **Visual** | `v` charwise, `V` linewise, `Ctrl-v` blockwise; then `d y c s`, `o` to swap ends |
| **Blockwise** | `I` and `A` type once and replicate to every row; `y`/`p` re-form the rectangle |
| **Commands** | `:w`, `:w name`, `:e`, `:e name`, `:q`, `:qa`, `:new`, `:help` |
| **Other** | `Ctrl-n` / `Ctrl-p` between notes, `Ctrl-d` / `Ctrl-u` half-page |
| **Find** | `f F t T` to a character on the line, `;` `,` to repeat |
| **Search** | `/` and `?`, `n` `N` to repeat, `*` for the word under the cursor |
| **Zoom** | `Cmd`/`Ctrl` with `+`, `-`, `0` — handled by the toolkit, not the app |

Normal mode is *parsed*, not switch-cased, because vim's grammar really is a
grammar: `[count] operator [count] motion`. Keystrokes accumulate in a pending
buffer that is re-parsed on each key, which is why `3dw`, `d3w`, `dd`, `2dd` and
`dG` all fall out of one code path — and why `d` on its own is a prefix that
waits rather than an error that beeps.

Text objects are checked before motions after an operator, since `i` and `a` can
only mean an object there. Bracket objects search across lines with depth
counting, so `di(` works on a call split over several lines; quote objects are
line-scoped, as they are in vim, because a quote is far more often unbalanced
across lines than a bracket.

Notes live in `./notes` (override with `PIXUI_NOTES_DIR`) and are seeded on
first run. Lines soft-wrap at the pane width; wrapping is computed from the raw
text alone, so the caret and the syntax highlighting can both be mapped onto the
same visual rows without either knowing about the other.

All three visual shapes reduce to a column range per line, so charwise,
linewise and blockwise selections draw through a single path in the editor
rather than three. A block reports its columns even on lines too short to reach
them, which is the only way to see what a blockwise append is about to pad out.

Search uses vim's *smartcase*: a lower-case pattern matches either case, and
the moment it contains a capital it means it. Matches stay highlighted until
Escape, so the highlight cannot outstay its welcome the way vim's `hlsearch`
famously does.

**Not implemented:** marks, macros, named registers, regular expressions in
search (patterns are literal), tag objects (`it`/`at`), and a rendered preview
pane.

## Design notes

**Why software rendering.** A normal toolkit spends most of its complexity on
resolution independence: antialiasing, hinting, subpixel positioning, vector
rasterisation. pixui opts out of all of it. A frame is 384x240 — 92k pixels,
about 0.08 ms to draw — so there are no shaders and no render graph. The
pixel-art look is not a costume on a conventional toolkit; it is what makes the
toolkit small.

**Integer scaling, always.** The virtual canvas is magnified by a whole number.
Fractional scaling is what makes pixel art look like a bad JPEG — some source
pixels get two output pixels and their neighbours get three. Refusing to do that
is the entire point.

**Two things a window resize can mean**, so `Config` makes you pick:

- `Scaling::Fixed` keeps the canvas the same size and magnifies it by the
  largest whole number that fits, letterboxing the rest. Right for a composed,
  fixed layout, where a bigger window should mean bigger pixels.
- `Scaling::Adaptive` pins the magnification and grows the canvas instead, so a
  bigger window means *more room* at the same pixel size. Right for anything
  with content in it. `with_min_canvas` sets the smallest layout you support,
  and the window gets a matching minimum size. Both demos use this.

The difference in feel is the step size. Under `Fixed` the canvas only changes
when a whole extra multiple fits — for a 576-wide canvas that is a 576 pixel
step, with up to 500 pixels of letterbox in between. Under `Adaptive` the step
is the magnification itself: one canvas pixel per 4 physical pixels at a 4x
scale, which reads as continuous.

**The UI scale is a runtime property, not a startup constant.**
`Ui::request_pixel_scale` changes it at any time, and the primary modifier with
`+`, `-` and `0` does so out of the box (`without_zoom_shortcuts` if you would
rather wire your own). Zoom keys are *consumed* by the backend, so an
application never has to filter them out of its own keyboard handling.
`with_scale_range` bounds the zoom.

**Zoom steps are coarse, and that is not a bug.** The live value is the
*physical* magnification, which must be a whole number or the pixels stop being
pixels. From 2 the next stop is 3 — 50% larger, not 30%. There is nothing in
between and there cannot be.

`Config::with_scale` takes a **float** for a related reason. It is a
density-independent opening size in logical points, and on a 2x display an
integer there could only ever produce an even magnification: 1 gives 2, 2 gives
4, and 3 is unreachable. `1.5` names it.

Under `Adaptive`, halving the scale doubles the canvas: the same window, every
piece of chrome half the size, twice as much content. That also means a layout
built from magic pixel constants stops working once the user can zoom — the note
app derives its sidebar width from the canvas for exactly this reason.

An adaptive canvas is pinned to the top-left rather than centred. Centring would
split the sub-scale remainder — always fewer than `scale` pixels — across both
edges, so every few pixels of a drag would shunt the whole UI sideways by one.
That shunt is far more noticeable than the remainder, which is painted in the
background colour and so is invisible.

Under `Adaptive` the scale is pinned in *logical points*, not physical pixels,
so one virtual pixel stays the same physical size whatever the display density —
and moving the window to a different-density display re-derives it.

**Glyphs are 5 wide on a 7 pixel advance**, so there are two columns of
tracking. One column is enough to keep letters technically apart, but at this
size `mmm` and `www` read as a single blob — and it leaves nothing for the bold
weight, which is a double-strike one pixel to the right.

Two measurements follow from that, and confusing them is a real bug:

- `text_width` is the **ink extent**: `(n-1) * ADVANCE + GLYPH_W`, with no
  trailing tracking. This is what centring wants.
- `advance_width` is `n * ADVANCE`: where the next glyph starts. This is what
  anything laying out consecutive runs wants.

Returning the ink extent from a drawing call makes every caller that accumulates
widths creep a pixel left per run. Styled markdown puts a dozen runs on a line,
so the text walks off the character grid the caret is drawn on. The editor now
positions each run by its character offset instead, which cannot drift at all.

Bold deliberately does **not** change the advance. It has to sit on the same
grid as everything else, or a bold run mid-line shunts the rest of it out of
step with the caret.

Anything that boxes a character — a block caret, a selection — should be
`GLYPH_W + 2` wide starting one pixel before the glyph. Sizing it from the
advance instead collects all the tracking on one side, and reads as shunted.

**Physical pixels throughout.** Hit testing and scaling work in physical pixels,
so a HiDPI display just yields a larger integer scale. There is no DPI factor
anywhere in the widget code.

**Springs, not lerps.** The satisfying part of a pixel button is the small
overshoot on release. `anim::Spring` is under-damped on purpose, and the press
offset is allowed to reach -1px, so the button really does pop up past its
resting position.

**Post-frame passes belong to the toolkit.** `Ui::finish` applies the scanline
overlay and draws the pointer, rather than leaving them to whoever is driving
the frame. When the backend owned those passes, the snapshot harnesses had to
reimplement them — two drivers that could disagree about whether the pointer is
drawn over the scanlines or under them. Anything driving the lifecycle by hand
now sets `Input::draw_pointer` and gets the same result.

**The pointer is drawn, not borrowed.** The system pointer is rendered by the
compositor at the display's real resolution, so beside chunky upscaled pixels it
looks like it belongs to a different program. `pixui` hides it and draws its own
into the canvas as the last thing each frame, which puts it on the same grid as
everything else and lets it re-colour with the theme. Sprites are written as
text — `X` outline, `#` fill, `.` gap — because a pointer is a drawing, and that
is a format you can edit a drawing in. `Config::without_pixel_cursor` opts out.

**Resize has to be answered inside the resize event.** The window has already
changed size by the time `Resized` arrives, so until a new frame is presented
the compositor stretches the previous one to fit. Waiting even a single frame
for the normal redraw tick is visible as the UI distorting during a drag and
snapping back afterwards, so the backend draws and presents synchronously there.

**Chamfers, not radii.** Cutting one or two pixels off each corner reads as
"soft" at this scale, costs nothing, and never produces a half-lit edge pixel.

**Scrolling measures, it does not plan.** In immediate mode there is no tree to
ask for a height, so `scroll_area` lays the content out and reads how far the
layout cursor got. That answer is one frame stale, which is invisible — but it
means the scrollbar gutter is reserved whether or not a bar is showing. If the
gutter came and went, content that reflows on width would oscillate forever.

**Clipping is hit testing.** `Ui::interact` tests the clip rect as well as the
widget rect, so a button scrolled halfway out of view stops responding at
exactly the line where it stops being drawn. There is a test for it.

## Performance

Measured on the demo at 2304x1440 (a 6x integer scale on a Retina display):

| stage | ms/frame | share |
|---|---|---|
| build UI + rasterise 384x240 | 0.08 | 1% |
| upscale blit to 3.3 Mpx | 0.23 | 3% |
| **softbuffer present** | **~7.8** | **96%** |

The CPU rasteriser is not the bottleneck and is nowhere close to being one.
Essentially the entire frame cost is softbuffer's macOS present path, which
copies 3.3M pixels into a `CGImage` every frame.

The fix, if it matters, is a GPU present: upload the 384x240 canvas as a texture
and draw one nearest-neighbour quad. That is **36x less data** across the bus,
and would take the frame well under a millisecond. It is a backend swap — one
new module beside `pixui::app` — and touches no widget code. See *Not done yet*.

## Not done yet

Stated plainly, because these are the things that separate this from a toolkit
you could ship a product on:

- **ASCII only.** The 5x7 bitmap font covers printable ASCII. Scripts that need
  shaping, and IME composition, are out of scope for it.
- **No accessibility tree.** To a screen reader this is an opaque rectangle. A
  real product would need to publish an `accesskit` tree alongside the pixels.
- **One window, no OS chrome.** No menus, no dialogs, no multi-window. On macOS
  the system menu bar is the OS and cannot be pixelated — it is the one
  unavoidable break in the illusion.
- **Scrolling is vertical only.** A horizontal axis would double the chrome for
  a case pixel-art UIs almost never want.

## Licence

MIT OR Apache-2.0
