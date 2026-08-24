# pixui

A chunky, warm, pixel-art immediate-mode UI toolkit for Rust, rendered entirely
on the CPU. Cross-platform via winit — macOS, Windows, Linux.

This is the toolkit underneath [`notes`](../README.md), the editor in the root
of this repo. It is a path dependency of that app rather than a published
crate.

![the editor it draws](../screenshots/editor.png)

## Layering

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

`pixui::app` is the sole place winit, softbuffer and wgpu appear. Two things
prove that rather than assert it: `notes --shots` drives the identical UI with
no window and no event loop to produce `screenshots/`, and `tests/gpu.rs`
renders through the real GPU pipeline offscreen and compares it pixel-for-pixel
against the CPU path.

Cargo features: `soft` (default, softbuffer) and `gpu` (wgpu).

## Design notes

**Why software rendering.** A conventional toolkit spends most of its complexity
on resolution independence: antialiasing, hinting, subpixel positioning, vector
rasterisation. pixui opts out of all of it. A frame is a few hundred pixels on a
side — well under 150k pixels, about 0.3 ms to draw — so there are no shaders
and no render graph. The pixel-art look is not a costume on a conventional
toolkit; it is what makes the toolkit small.

**Integer scaling, always.** Fractional scaling is what makes pixel art look
like a bad JPEG: some source pixels get two output pixels and their neighbours
get three. `Scaling::Fixed` keeps the canvas and magnifies it, letterboxing the
rest; `Scaling::Adaptive` pins the magnification and grows the canvas, so a
bigger window means *more room*. Zoom steps whole pixels, which is why the steps
are coarse: from 2 the next stop is 3, and there is nothing in between.

**Resize is answered inside the resize event.** The window has already changed
size when `Resized` arrives, so until a new frame is presented the compositor
stretches the previous one. Waiting even one frame for the redraw tick shows up
as the UI distorting during a drag.

**Nothing dithered goes behind text.** `gradient_rect` uses ordered dithering,
which is the right way to fake a gradient from a small palette and exactly the
wrong thing to put behind 5x7 letterforms — the checkerboard lands between the
strokes.

**Glyphs are 5 wide on a 7 pixel advance.** `text_width` is the ink extent;
`advance_width` is where the next glyph starts. Returning the first from a
drawing call makes every caller that accumulates widths creep a pixel left per
run. Anything boxing a character — a block caret, a selection — should be
`GLYPH_W + 2` wide starting one pixel before the glyph.

**Springs, not lerps.** The satisfying part of a pixel button is the small
overshoot on release. `anim::Spring` is under-damped on purpose, and the press
offset may reach -1px so the button pops up past its resting position.

**The pointer is drawn, not borrowed.** The system pointer is composited at the
display's real resolution and looks like it belongs to a different program.
`Ui::finish` hides it and draws its own last thing each frame.

## Performance

Measured with `PIXUI_PROFILE=1` on a 120Hz panel at 2304x1440 (a 6x integer
scale), `PIXUI_VSYNC=0` so `present` is real work rather than a wait for the
next vblank:

| backend | ui + raster | present | CPU | sustained |
|---|---|---|---|---|
| `soft` (softbuffer) | 0.08 ms | ~6.2 ms | ~72% of a core | ~113 fps |
| `gpu` (wgpu) | 0.33 ms | ~0.6 ms | ~18% of a core | 120 fps (capped) |

The CPU rasteriser was never the bottleneck. The cost was moving the result: the
CPU path upscales first and hands the platform 3.3M pixels; the GPU path uploads
the canvas *unscaled* — 36x less data — and magnifies it in a two-line fragment
shader. That difference decides whether 120Hz is reachable at all.

## Not done

- **ASCII only.** The 5x7 bitmap font covers printable ASCII. Shaping and IME
  are out of scope for it.
- **No accessibility tree.** To a screen reader this is an opaque rectangle. A
  real product would need to publish an `accesskit` tree alongside the pixels.
- **One window, no OS chrome.** On macOS the system menu bar is the OS and
  cannot be pixelated — the one unavoidable break in the illusion.
- **Scrolling is vertical only.**
