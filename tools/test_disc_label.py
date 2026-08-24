#!/usr/bin/env python3
"""python3 tools/test_disc_label.py -- renders the label and scans its QR codes.

A label whose QR does not decode is a wasted print run, so this is the one
check worth keeping. Needs rsvg-convert and opencv.
"""
import subprocess
import sys
import tempfile
from pathlib import Path

import cv2

HERE = Path(__file__).resolve().parent
LINKS = [("ALL LINKS", "https://allmylinks.com/bonniestudiosdev"),
         ("PLAY IN BROWSER", "https://bonnie-studios.itch.io/psoxide")]

with tempfile.TemporaryDirectory() as tmp:
    svg, png = Path(tmp) / "l.svg", Path(tmp) / "l.png"
    out = subprocess.run([sys.executable, HERE / "disc_label.py"]
                         + [a for label, url in LINKS for a in ("--link", f"{label}={url}")],
                         capture_output=True, text=True, check=True).stdout
    svg.write_text(out)
    subprocess.run(["rsvg-convert", "-d", "600", "-p", "600", svg, "-o", png], check=True)

    img = cv2.imread(str(png))
    n = img.shape[0]
    det = cv2.QRCodeDetector()
    # The two QR slots, in the same mm coordinates disc_label.py places them at.
    for (label, url), cx in zip(LINKS, (26, 94)):
        s = int(24 / 120 * n)
        x, y = int(cx / 120 * n), int(56 / 120 * n)
        got, _, _ = det.detectAndDecode(img[y - s // 2:y + s // 2, x - s // 2:x + s // 2])
        assert got == url, f"{label}: scanned {got!r}, wanted {url!r}"
    print("QR CHECK PASS", *(u for _, u in LINKS))
