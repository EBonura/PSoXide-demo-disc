#!/usr/bin/env python3
"""Cook the link icons the credits card draws beside each URL.

The links panel writes only the part after the slash -- `EBonura/PSoXide`,
not `github.com/EBonura/PSoXide` -- because the description column is 23
characters wide and every full URL overflows it. The icon carries the domain,
so the text only has to carry the handle.

Sources are Simple Icons (simpleicons.org), whose SVG paths are CC0. Each is
rasterised large and reduced with a proper filter: these are 24x24 viewBox
glyphs and nearest-neighbour at this size loses the GitHub cat entirely.

Coverage becomes brightness. The SVG paths are solid black on transparent, so
the alpha channel IS the shape, and it maps onto a 16-step grey ramp that the
GPU then tints with the primitive colour, the same way the font draws.

CELL is deliberately wider than ICON and a multiple of four. A 4bpp texel is
half a byte, so a cell whose width is not a multiple of four starts a later
icon mid-nibble; that is the exact fault that turned SPLEEN's 'f' into a bare
crossbar on silicon and never showed in an emulator. Four texels a byte, and
the row stride lands on a whole halfword too.

    python3 tools/icons.py <svg-dir> <out-dir>

Writes `icons.tex` (packed 4bpp indices) and `icons.clut` (16 little-endian
15-bit colours) for `include_bytes!`.
"""

import subprocess
import sys
from pathlib import Path

from PIL import Image

# The order here is the order the launcher indexes them by, so it is API.
NAMES = ["github", "itchdotio", "x", "instagram", "buymeacoffee", "youtube"]

ICON = 12
# Padded to a multiple of four texels; see the module docstring.
CELL = 12
# Rasterise well above target so the reduction has something to filter.
SUPER = 256
LEVELS = 16


def rasterise(svg: Path) -> Image.Image:
    """SVG -> ICON x ICON coverage map, as an 8-bit L image."""
    png = subprocess.run(
        ["rsvg-convert", "-w", str(SUPER), "-h", str(SUPER), str(svg)],
        check=True,
        capture_output=True,
    ).stdout
    tmp = svg.with_suffix(".super.png")
    tmp.write_bytes(png)
    with Image.open(tmp) as im:
        alpha = im.convert("RGBA").split()[3]
        small = alpha.resize((ICON, ICON), Image.LANCZOS)
    tmp.unlink()
    return small


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(f"usage: {sys.argv[0]} <svg-dir> <out-dir>")
    svg_dir, out_dir = Path(sys.argv[1]), Path(sys.argv[2])
    out_dir.mkdir(parents=True, exist_ok=True)

    width = CELL * len(NAMES)
    if width % 4:
        raise SystemExit(f"row of {width} texels does not pack to whole bytes")

    # One row of cells, indices 0..15 where 0 is transparent.
    rows = [[0] * width for _ in range(ICON)]
    for slot, name in enumerate(NAMES):
        cov = rasterise(svg_dir / f"{name}.svg")
        pad = (CELL - ICON) // 2
        for y in range(ICON):
            for x in range(ICON):
                # 0 has to stay empty: on this hardware a CLUT entry of
                # 0x0000 is see-through, and that is what lets the panel
                # show through around the mark.
                level = cov.getpixel((x, y)) * (LEVELS - 1) // 255
                rows[y][slot * CELL + pad + x] = level

    tex = bytearray()
    for row in rows:
        for x in range(0, width, 2):
            # Low nibble is the left texel.
            tex.append(row[x] | (row[x + 1] << 4))
    (out_dir / "icons.tex").write_bytes(tex)

    # Grey ramp. Entry 0 transparent, the rest a straight 15-bit grey the
    # primitive colour then tints.
    clut = bytearray()
    for i in range(LEVELS):
        if i == 0:
            clut += (0x0000).to_bytes(2, "little")
            continue
        five = i * 31 // (LEVELS - 1)
        clut += (0x8000 | five | five << 5 | five << 10).to_bytes(2, "little")
    (out_dir / "icons.clut").write_bytes(clut)

    print(
        f"{len(NAMES)} icons, {ICON}x{ICON} in {CELL}-texel cells, "
        f"{width}x{ICON} texels -> icons.tex {len(tex)}B, icons.clut {len(clut)}B"
    )


if __name__ == "__main__":
    main()
