#!/usr/bin/env python3
"""Bake the printable ASCII of a BDF bitmap font into a Rust glyph table.

The app has no font engine and is not getting one: glyphs are bits, laid out
at authoring time and blitted at runtime. This is the authoring time.

    tools/bdf2rs.py cozette.bdf COZETTE "Cozette" "MIT, (c) ..." > out.rs

Every glyph is placed on one common cell so the renderer can step by a fixed
advance: the cell is `ascent + descent` tall, and a glyph's own bounding box is
positioned against the baseline exactly as the BDF describes it.
"""

import sys


def parse(path):
    ascent = descent = None
    fbb = None
    glyphs = {}
    code = bbx = None
    bits = None
    dwidth = None
    widths = {}
    for line in open(path, encoding="latin-1"):
        parts = line.split()
        if not parts:
            continue
        key = parts[0]
        if key == "FONTBOUNDINGBOX":
            fbb = [int(v) for v in parts[1:5]]
        elif key == "FONT_ASCENT":
            ascent = int(parts[1])
        elif key == "FONT_DESCENT":
            descent = int(parts[1])
        elif key == "ENCODING":
            code = int(parts[1])
        elif key == "BBX":
            bbx = [int(v) for v in parts[1:5]]
        elif key == "DWIDTH":
            dwidth = int(parts[1])
        elif key == "BITMAP":
            bits = []
        elif key == "ENDCHAR":
            if code is not None and bbx is not None and bits is not None:
                glyphs[code] = (bbx, bits)
                if dwidth is not None:
                    widths[code] = dwidth
            code = bbx = bits = dwidth = None
        elif bits is not None:
            bits.append(int(parts[0], 16))
    if ascent is None or descent is None:
        # Fall back to the font bounding box, which is what it is there for.
        ascent = fbb[1] + fbb[3]
        descent = -fbb[3]
    return ascent, descent, glyphs, widths


def main():
    path, ident, name, licence = sys.argv[1:5]
    # An advance override, for a face whose own is too tight for this app's
    # bold: bold here is a double-strike one pixel right, so a glyph needs a
    # column of clear air after it or the strike lands on its neighbour.
    forced = int(sys.argv[5]) if len(sys.argv) > 5 else None
    ascent, descent, glyphs, dwidths = parse(path)
    cell_h = ascent + descent
    # The advance is the font's own, not a guess from the ink: a monospaced
    # font states it, and every printable glyph should agree about it.
    steps = [dwidths[c] for c in range(32, 127) if c in dwidths]
    advance = max(set(steps), key=steps.count)
    # The cell is wide enough for the widest ink, which in some fonts overhangs
    # the advance by a pixel. Overhang is how a bitmap font joins box drawing
    # up, and clipping it would leave gaps.
    inked = {glyphs[c][0][0] + glyphs[c][0][2] for c in range(32, 127) if c in glyphs}
    cell_w = max(max(inked), advance)
    if forced:
        advance = forced

    rows = []
    for c in range(32, 127):
        cell = [0] * cell_h
        if c in glyphs:
            (bw, bh, bx, by), bits = glyphs[c]
            top = ascent - (bh + by)
            for i, word in enumerate(bits):
                y = top + i
                if not 0 <= y < cell_h:
                    continue
                # BDF pads each row to a whole number of bytes, left-aligned.
                pad = (bw + 7) // 8 * 8 - bw
                word >>= pad
                for x in range(bw):
                    if word >> (bw - 1 - x) & 1:
                        px = bx + x
                        if 0 <= px < cell_w:
                            cell[y] |= 1 << (cell_w - 1 - px)
        rows.extend(cell)

    print(f"//! {name}, baked from its BDF by `tools/bdf2rs.py`.")
    print("//!")
    for line in licence.split("\n"):
        print(f"//! {line}")
    print()
    print("use super::Face;")
    print()
    print(f"pub static {ident}: Face = Face {{")
    print(f'    name: "{name.upper()}",')
    print(f"    glyph_w: {cell_w},")
    print(f"    glyph_h: {cell_h},")
    print(f"    advance: {advance},")
    print(f"    line_h: {cell_h},")
    print(f"    rows: &{ident}_ROWS,")
    print("};")
    print()
    print(f"#[rustfmt::skip]")
    print(f"static {ident}_ROWS: [u16; {len(rows)}] = [")
    for c in range(32, 127):
        start = (c - 32) * cell_h
        cell = rows[start : start + cell_h]
        body = ",".join(f"0x{v:04x}" for v in cell)
        ch = chr(c) if c != 92 else "\\\\"
        print(f"    {body}, // {ch}")
    print("];")


main()
