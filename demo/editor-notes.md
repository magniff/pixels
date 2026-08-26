# Editor notes

Rough notes on the thing I am building, written fast and not tidied up.

The toolkit is called **pixui**. The editor built on top of it is called
**notes**. Everything on screen is drawn by hand into a plain buffer of pixels:
no GPU canvas underneath, no system widget anywhere, not even for the save
dialog. The window is a `Vec<u32>` and every glyph is blitted into it.

## Look

The colour schemes live in the toolkit rather than in the app, and each one is
taken from its authors' own published documentation rather than eyeballed from
a screenshot. There are nine. The bitmap faces are handled the same way and
there are five of those, from a 5x7 up to a 14-pixel one.

Anyway it all works and looks fine.

## Keys

Modal editing, so the grammar is vim's: operators combine with motions, counts
repeat them, and doubling an operator makes it linewise. The full list of what
is implemented and what is not is written up in another note.

I keep forgetting which one.

## Rendering

A frame costs about a third of a millisecond at the size the window opens at,
which is why none of the above was ever a problem. Redraws only happen when
something asks for one.

That is the reason the whole thing can be done this way.

## Still to do

Cut some of the colour schemes. There are more of them than one person needs
and I only ever use the one.

## Assistant

A quantised model on the machine, through llama.cpp, on the GPU where there is
one. Nothing leaves the laptop. It gets the passage you selected, the note
around it, and a line about every other note.

Some of these need cleaning up.
