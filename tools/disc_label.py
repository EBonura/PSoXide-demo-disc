#!/usr/bin/env python3
"""On-disc label for the demo disc: a 120 mm SVG with the 15 mm hole cut out.

    python3 tools/disc_label.py [--hl] [--link "LABEL=URL"]... > assets/label/disc-label.svg

Up to two --link entries become QR codes left/right of the hub with the URL
printed under each (dark modules on a light panel: inverted QRs scan badly).

Everything is in millimetres (viewBox = mm, width/height carry the unit), so a
print shop or a hub-printable CD-R template can drop it in at 1:1. Text stays
outside the 42 mm stacking ring; the background bleeds to the hole so a
hub-printable disc has no white collar.
"""
import argparse
import random
import re
import sys
from xml.sax.saxutils import escape
from pathlib import Path

LOGO = Path(__file__).resolve().parents[2] / "PSoXide/assets/branding/psoxide-logo.svg"

GAMES = ["CORTEX IGNITION", "QUAKE SHAREWARE", "VOXIDE", "NITROXIDE",
         "CELESTE COLLECTION", "PSXCEL", "GH-PSX", "BREAKOUT",
         "SPACE INVADERS", "MAGIKAAAAARP PONG", "HARDWARE TESTS"]

CX = CY = 60.0
R_DISC, R_HOLE, R_HUB = 60.0, 7.5, 21.0   # mm; hub = stacking-ring safe zone
CYAN, INK, DIM = "#00c4ec", "#ffffff", "#8e9aa8"


def wordmark():
    """The <g> from the brand SVG, minus the tagline text."""
    svg = LOGO.read_text()
    defs = re.search(r"<defs>.*?</defs>", svg, re.S).group(0)
    g = re.search(r"<g transform=.*?</g>", svg, re.S).group(0)
    return defs, g


def stars(n=220, seed=1):
    rnd = random.Random(seed)
    out = []
    while len(out) < n:
        x, y = rnd.uniform(0, 120), rnd.uniform(0, 120)
        d = ((x - CX) ** 2 + (y - CY) ** 2) ** 0.5
        if R_HUB + 1 < d < R_DISC - 1:
            r = rnd.choice([0.08, 0.1, 0.12, 0.16, 0.22])
            a = rnd.uniform(0.35, 1.0)
            out.append(f'<circle cx="{x:.2f}" cy="{y:.2f}" r="{r}" fill="#fff" opacity="{a:.2f}"/>')
    return "\n".join(out)


def rings():
    out = []
    for r in range(24, 60, 4):
        a = 0.05 + 0.10 * (r - 24) / 36
        out.append(f'<circle cx="{CX}" cy="{CY}" r="{r}" fill="none" stroke="{CYAN}" stroke-width="0.12" opacity="{a:.2f}"/>')
    for i in range(24):
        import math
        t = math.tau * i / 24
        x1, y1 = CX + R_HUB * math.cos(t), CY + R_HUB * math.sin(t)
        x2, y2 = CX + R_DISC * math.cos(t), CY + R_DISC * math.sin(t)
        out.append(f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="{CYAN}" stroke-width="0.12" opacity="0.10"/>')
    return "\n".join(out)


# ponytail: glyph advances are eyeballed fractions of the font size, not real
# metrics. Good enough for a title ring; librsvg has no <textPath>, so baking
# per-glyph transforms is what makes this render in every tool.
NARROW = {" ": 0.30, "·": 0.36, "I": 0.30, "J": 0.52, "L": 0.60, "-": 0.40}


def ring_text(text, radius, size, fill, gap_deg=0.0):
    """Text laid clockwise around the centre, starting at 12 o'clock."""
    import math
    adv = [size * NARROW.get(c, 0.66) for c in text]
    total = sum(adv)
    span = math.degrees(total / radius) - gap_deg
    a = -span / 2            # centre the run on 12 o'clock
    out = []
    for c, w in zip(text, adv):
        step = math.degrees(w / radius)
        mid = a + step / 2
        if c != " ":
            out.append(f'<text x="0" y="0" text-anchor="middle" fill="{fill}" font-size="{size}" '
                       f'transform="translate({CX} {CY}) rotate({mid:.3f}) translate(0 {-radius})">{escape(c)}</text>')
        a += step
    return "\n".join(out)


def qr(url, cx, cy, size, caption):
    """QR as SVG rects. Dark-on-light panel with a quiet zone, or it won't scan."""
    import segno
    m = segno.make(url, error="h").matrix
    n = len(m)
    quiet = 2
    step = size / (n + 2 * quiet)
    pad = 0.8
    x0, y0 = cx - size / 2, cy - size / 2
    out = [f'<rect x="{x0 - pad:.2f}" y="{y0 - pad:.2f}" width="{size + 2 * pad:.2f}" '
           f'height="{size + 2 * pad:.2f}" rx="1" fill="#ffffff"/>']
    for r, row in enumerate(m):
        for c, v in enumerate(row):
            if v:
                out.append(f'<rect x="{x0 + (c + quiet) * step:.3f}" y="{y0 + (r + quiet) * step:.3f}" '
                           f'width="{step:.3f}" height="{step:.3f}" fill="#04070d"/>')
    out.append(f'<text x="{cx:.2f}" y="{y0 + size + 3.4:.2f}" text-anchor="middle" fill="{DIM}" '
               f'font-size="2.0" letter-spacing="0.15">{caption}</text>')
    return "\n".join(out)


def build(hl: bool, links) -> str:
    games = GAMES[:1] + (["HALF-LIFE"] if hl else []) + GAMES[1:]
    rim = "  ·  ".join(games)
    defs, mark = wordmark()
    # QRs flank the hub, clear of both the stacking ring and the title block.
    slots = [(CX - 34, 56), (CX + 34, 56)]
    qr_block = "\n".join(qr(url, x, y, 17, escape(label))
                         for (label, url), (x, y) in zip(links, slots))
    r_rim = 54.6   # clear of the printable edge
    ring = ring_text(rim, r_rim, 2.7, DIM)
    return f'''<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
     width="120mm" height="120mm" viewBox="0 0 120 120" role="img" aria-label="PSoXide Demo Disc label">
  {defs}
  <defs>
    <radialGradient id="bg" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="#0f1a2c"/>
      <stop offset="55%" stop-color="#0a1220"/>
      <stop offset="100%" stop-color="#04070d"/>
    </radialGradient>
    <radialGradient id="hubGlow" cx="50%" cy="50%" r="50%">
      <stop offset="60%" stop-color="{CYAN}" stop-opacity="0.18"/>
      <stop offset="100%" stop-color="{CYAN}" stop-opacity="0"/>
    </radialGradient>
    <mask id="disc">
      <circle cx="{CX}" cy="{CY}" r="{R_DISC}" fill="#fff"/>
      <circle cx="{CX}" cy="{CY}" r="{R_HOLE}" fill="#000"/>
    </mask>
  </defs>

  <g mask="url(#disc)" font-family="Arial, Helvetica, sans-serif">
    <circle cx="{CX}" cy="{CY}" r="{R_DISC}" fill="url(#bg)"/>
    <circle cx="{CX}" cy="{CY}" r="{R_HUB + 6}" fill="url(#hubGlow)"/>
    {rings()}
    {stars()}

    <!-- hub accent + outer accent ring -->
    <circle cx="{CX}" cy="{CY}" r="{R_HUB}" fill="none" stroke="{CYAN}" stroke-width="0.35" opacity="0.85"/>
    <circle cx="{CX}" cy="{CY}" r="{R_HUB + 1.2}" fill="none" stroke="{CYAN}" stroke-width="0.12" opacity="0.35"/>
    <circle cx="{CX}" cy="{CY}" r="58.0" fill="none" stroke="{CYAN}" stroke-width="0.25" opacity="0.6"/>

    <!-- wordmark: brand group is 1000x260 with glyphs at x 96..900, y 48..172 -->
    <g transform="translate({CX} 26) scale(0.076) translate(-498 -110)">
      {mark}
    </g>
    <text x="{CX}" y="34.2" text-anchor="middle" fill="{DIM}" font-size="2.3" letter-spacing="0.5">PS1 SDK · ENGINE · EDITOR · EMULATOR</text>

    <text x="{CX}" y="93" text-anchor="middle" fill="{INK}" font-size="9" font-weight="bold" letter-spacing="1.5">DEMO DISC</text>
    <text x="{CX}" y="98.2" text-anchor="middle" fill="{CYAN}" font-size="2.6" letter-spacing="0.9">PLAYSTATION HOMEBREW · 2026</text>

    {qr_block}

{ring}
  </g>
</svg>
'''


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--hl", action="store_true", help="the HL pressing (adds HALF-LIFE to the rim)")
    ap.add_argument("--link", action="append", default=[], metavar="LABEL=URL",
                    help="QR code, at most two (they flank the hub)")
    a = ap.parse_args()
    links = [tuple(s.split("=", 1)) for s in a.link]
    assert all(len(p) == 2 and p[1].startswith("http") for p in links), "--link needs LABEL=https://..."
    assert len(links) <= 2, "only two QR slots"
    sys.stdout.write(build(a.hl, links))
