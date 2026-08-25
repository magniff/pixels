# Embedded fonts

The glyphs in `src/font/` are baked from these by `tools/bdf2rs.py`. There is
no font engine here and there is not going to be one: a face is a table of bits,
laid out at authoring time and blitted at runtime.

| face | source | licence |
|---|---|---|
| Creep2 | <https://github.com/raymond-w-ko/creep2> | MIT, (c) 2014 Romeo Van Snick |
| Tamzen 6x12 | <https://github.com/sunaku/tamzen-font> | free, (c) 2011 Suraj N. Kurapati, from Tamsyn |
| Gohufont 14 | <https://github.com/hchargois/gohufont> | WTFPL v2, (c) 2015 Hugo Chargois |
| Cozette | <https://github.com/the-moonwitch/Cozette> | MIT, (c) 2020-2025 Ines |

The licence texts here are the upstream ones, kept because these licences ask
that they travel with the work. The 5x7 face they sit beside is this toolkit's
own, drawn by hand.

Every baked face is stepped one pixel wider than its own advance. This app's
bold is a double-strike one pixel to the right, so a glyph needs a column of
clear air after it or the strike lands on its neighbour.

The 6x11 and 6x12 faces this began with — Gohufont 11 and Spleen 6x12 — came
out again on looking at them properly: at six pixels wide both draw `V` almost
identically to `U`, and their `m` and `w` are cramped to the point of guessing.
Tamzen draws the same cell far more clearly, and Gohufont's own 14 has the room
its letterforms want.

**Ark Pixel** was embedded here for an afternoon and then removed: its
repository carries both an MIT and an OFL licence, and the MIT one covers the
build scripts while the font itself is OFL. Worth checking twice — a licence
file in a repository is not necessarily the licence of the thing you want.
**Scientifica** was passed over for the same reason.
