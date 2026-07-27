#!/usr/bin/env python3
"""Convert the PSoXide logo into a 4bpp CLUT texture the launcher can upload.

The menu draws no textures anywhere else, so this is the one asset on the
screen. It has to fit a single PlayStation texture page: UVs are bytes, which
caps a page at 256 texels, and 4bpp keeps sixteen colours in 80 bytes a row.

Palette index 0 is reserved for transparent. The PlayStation treats a CLUT
entry of 0x0000 as see-through, so the logo's alpha becomes index 0 and the
starfield shows through the letterforms instead of a black box sitting on it.

    python3 tools/banner.py <logo.png> <out-dir> [width] [height] [crop-rows]

`crop-rows` keeps only the top N rows of the source. The logo's tagline is
unreadable once the mark fits a menu corner, so it is cropped off rather than
shipped as a grey smear.

Writes `banner.tex` (packed 4bpp indices) and `banner.clut` (16 little-endian
15-bit colours) for `include_bytes!`.
"""

import struct
import sys
import zlib
from pathlib import Path

TRANSPARENT = 0
COLOURS = 16


def read_png(path):
    """Decode a PNG to (width, height, RGBA rows). Enough of the format for
    the one file this tool exists to convert."""
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit(f"{path}: not a PNG")
    pos, idat, meta = 8, bytearray(), None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            w, h, depth, colour = struct.unpack(">IIBB", body[:10])
            if depth != 8 or colour not in (2, 6):
                raise SystemExit(f"{path}: need 8-bit RGB or RGBA, got depth {depth} type {colour}")
            meta = (w, h, 4 if colour == 6 else 3)
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        pos += 12 + length

    w, h, channels = meta
    raw = zlib.decompress(bytes(idat))
    stride = w * channels
    rows, previous, at = [], bytearray(stride), 0
    for _ in range(h):
        filter_type = raw[at]
        line = bytearray(raw[at + 1 : at + 1 + stride])
        at += 1 + stride
        # PNG filters, per the spec. Paeth is the only fiddly one.
        for i in range(stride):
            a = line[i - channels] if i >= channels else 0
            b = previous[i]
            c = previous[i - channels] if i >= channels else 0
            if filter_type == 1:
                line[i] = (line[i] + a) & 0xFF
            elif filter_type == 2:
                line[i] = (line[i] + b) & 0xFF
            elif filter_type == 3:
                line[i] = (line[i] + (a + b) // 2) & 0xFF
            elif filter_type == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xFF
        rows.append(
            [
                (
                    line[x * channels],
                    line[x * channels + 1],
                    line[x * channels + 2],
                    line[x * channels + 3] if channels == 4 else 255,
                )
                for x in range(w)
            ]
        )
        previous = line
    return w, h, rows


def resample(rows, src_w, src_h, dst_w, dst_h):
    """Box filter down to the target size. Averaging beats nearest here: the
    logo's edges are its whole character, and point sampling shreds them."""
    out = []
    for y in range(dst_h):
        y0, y1 = y * src_h // dst_h, max(y * src_h // dst_h + 1, (y + 1) * src_h // dst_h)
        line = []
        for x in range(dst_w):
            x0, x1 = x * src_w // dst_w, max(x * src_w // dst_w + 1, (x + 1) * src_w // dst_w)
            r = g = b = a = n = 0
            for sy in range(y0, y1):
                for sx in range(x0, x1):
                    pr, pg, pb, pa = rows[sy][sx]
                    # Weight colour by coverage so transparent pixels do not
                    # drag the edges toward black.
                    r += pr * pa
                    g += pg * pa
                    b += pb * pa
                    a += pa
                    n += 1
            line.append((r // a, g // a, b // a, a // n) if a else (0, 0, 0, 0))
        out.append(line)
    return out


def to_555(r, g, b):
    return (b >> 3) << 10 | (g >> 3) << 5 | (r >> 3)


def build_palette(pixels):
    """Fifteen opaque colours by popularity in 15-bit space, plus transparent
    at index 0. The logo is a flat mark with a glow, so a handful of levels
    covers it; there is no need for anything cleverer than counting."""
    counts = {}
    for r, g, b, a in pixels:
        if a >= 128:
            counts[to_555(r, g, b)] = counts.get(to_555(r, g, b), 0) + 1
    ranked = sorted(counts, key=lambda c: -counts[c])[: COLOURS - 1]
    return [0x0000] + ranked


def nearest(palette, colour):
    """Closest palette entry, skipping index 0 which means transparent."""
    r, g, b = colour & 31, (colour >> 5) & 31, (colour >> 10) & 31
    best, best_d = 1, None
    for i in range(1, len(palette)):
        pr, pg, pb = palette[i] & 31, (palette[i] >> 5) & 31, (palette[i] >> 10) & 31
        d = (r - pr) ** 2 + (g - pg) ** 2 + (b - pb) ** 2
        if best_d is None or d < best_d:
            best, best_d = i, d
    return best


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 1
    src, out_dir = argv[1], Path(argv[2])
    width = int(argv[3]) if len(argv) > 3 else 160
    height = int(argv[4]) if len(argv) > 4 else 42
    crop_rows = int(argv[5]) if len(argv) > 5 else 0
    if width % 4:
        raise SystemExit("width must be a multiple of 4: 4bpp packs four texels a halfword")
    if width > 256:
        raise SystemExit("width must be at most 256: UVs are bytes, so a page is 256 texels")

    src_w, src_h, rows = read_png(src)
    if crop_rows:
        rows = rows[:crop_rows]
        src_h = crop_rows
    small = resample(rows, src_w, src_h, width, height)
    flat = [p for line in small for p in line]
    palette = build_palette(flat)
    palette += [0x0000] * (COLOURS - len(palette))

    lookup = {}
    indices = []
    for r, g, b, a in flat:
        if a < 128:
            indices.append(TRANSPARENT)
            continue
        key = to_555(r, g, b)
        if key not in lookup:
            lookup[key] = nearest(palette, key)
        indices.append(lookup[key])

    packed = bytearray()
    for y in range(height):
        row = indices[y * width : (y + 1) * width]
        for x in range(0, width, 2):
            packed.append(row[x] | (row[x + 1] << 4))

    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "banner.tex").write_bytes(bytes(packed))
    (out_dir / "banner.clut").write_bytes(b"".join(struct.pack("<H", c) for c in palette))
    opaque = sum(1 for i in indices if i != TRANSPARENT)
    print(
        f"{src} {src_w}x{src_h} -> {width}x{height}, "
        f"{len(packed)} bytes of 4bpp, {len(set(indices))} of {COLOURS} palette slots used, "
        f"{opaque * 100 // len(indices)}% opaque"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
