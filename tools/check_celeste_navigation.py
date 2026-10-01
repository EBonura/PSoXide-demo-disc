#!/usr/bin/env python3
"""Check Celeste return and credits navigation through the actual demo disc.

Requires Pillow. Boots the supplied pressing, reads its carousel order from
the TOC, launches each cart, and checks pause-quit and Select+Start returns
with both digital and analog controllers. Keeps logs, screenshots and a
receipt identifying the exact disc and emulator used. A missing checkpoint,
unexpected screen, early exit or timeout fails the route.
"""

from __future__ import annotations

import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import subprocess

from PIL import Image

from check_program_headless import (
    FAILURE_MARKERS, TOC_LBA, TOC_SECTORS, cue_bin, menu_presses, parse_entries,
    read_user_sectors,
)


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def pixels(image):
    return image.get_flattened_data() if hasattr(image, "get_flattened_data") else image.getdata()


def screen(path: Path) -> tuple[str, int | None]:
    with Image.open(path) as source:
        im = source.convert("RGB")
    if im.size != (320, 240):
        return "unknown", None
    if Counter(pixels(im.crop((156, 202, 200, 226))))[(222, 206, 115)] >= 150:
        covers = [sum(map(sum, pixels(im.crop(box))))
                  for box in ((52, 68, 132, 148), (188, 68, 268, 148))]
        return "menu", int(covers[1] > covers[0])
    if Counter(pixels(im))[(8, 8, 24)] >= 320 * 240 * 3 // 10:
        return "credits", None
    border = sum(im.getpixel((56, y)) == (255, 247, 239)
                 and im.getpixel((262, y)) == (255, 247, 239)
                 for y in range(60, 180))
    if border >= 100:
        return "pause", None
    return "unknown", None


def route(prefix: list[str], launch: int, cart: int, chord: bool):
    # Times are relative to the outer carousel's launch, not the standalone
    # collection's boot. Leave room for the CD loader and both title fades.
    start = launch + 3200 + cart * 100
    events = list(prefix)
    if cart:
        events.append(f"{launch + 1500}:right:8")
    events += [f"{launch + t + cart * 100}:cross:12" for t in (1600, 2200, 2800)]
    checks = [(launch + 1400, "menu", 0)]
    if chord:
        events += [f"{start}:select:30", f"{start + 4}:start:26"]
    else:
        events += [f"{start}:start:8", f"{start + 160}:up:8", f"{start + 320}:cross:8"]
        checks += [(start + 100, "pause", None)]
    checks += [(start + 500, "menu", cart)]
    events += [f"{start + 700}:select:8", f"{start + 1100}:cross:8"]
    checks += [(start + 900, "credits", None), (start + 1300, "menu", cart)]
    return events, checks


def stress_route(prefix: list[str], launch: int, cart: int):
    # First quit after a longer gameplay interval, holding Cross across the
    # menu transition. Then relaunch and quit with Select still held, and
    # finally visit the credits while holding Select and exit with Cross.
    events = list(prefix)
    if cart:
        events.append(f"{launch + 1500}:right:8")
    events += [f"{launch + t + cart * 100}:cross:12" for t in (1600, 2200, 2800)]
    checks = [(launch + 1400, "menu", 0)]
    start = launch + 9000
    events += [f"{start}:start:8", f"{start + 160}:up:8", f"{start + 320}:cross:400"]
    checks += [(start + 100, "pause", None), (start + 600, "menu", cart),
               (start + 900, "menu", cart)]
    events += [f"{start + t}:cross:12" for t in (1000, 1600, 2200)]
    events += [f"{start + 2600}:select:700", f"{start + 2604}:start:200"]
    checks += [(start + 3000, "menu", cart), (start + 3500, "menu", cart)]
    events += [f"{start + 3700}:select:700", f"{start + 4100}:cross:200"]
    checks += [(start + 3900, "credits", None), (start + 4300, "menu", cart),
               (start + 4600, "menu", cart)]
    # A third entry and pause-quit guards against stale global input state.
    events += [f"{start + t}:cross:12" for t in (4800, 5400, 6000)]
    events += [f"{start + 6400}:start:8", f"{start + 6560}:up:8", f"{start + 6720}:cross:8"]
    checks += [(start + 6500, "pause", None), (start + 7000, "menu", cart)]
    return events, checks


def save_route(prefix: list[str], launch: int, cart: int):
    events = list(prefix)
    if cart:
        events.append(f"{launch + 1500}:right:8")
    events += [f"{launch + t + cart * 100}:cross:12" for t in (1600, 2200, 2800)]
    start = launch + 3200 + cart * 100
    # SFX starts at eight on this fresh memory card. Change it to seven, then
    # wrap up to Quit. Leaving the dirty pause menu must write the card.
    events += [f"{start}:start:8", f"{start + 160}:left:8", f"{start + 320}:up:8",
               f"{start + 480}:cross:200", f"{start + 1700}:select:8",
               f"{start + 2100}:cross:8"]
    checks = [(launch + 1400, "menu", 0), (start + 100, "pause", None),
              (start + 300, "pause", None), (start + 1500, "menu", cart),
              (start + 1900, "credits", None), (start + 2300, "menu", cart)]
    return events, checks


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--frontend", type=Path, required=True)
    ap.add_argument("--cue", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True, help="new evidence directory")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--suite", choices=("basic", "stress", "save", "all"), default="all")
    args = ap.parse_args()
    if args.jobs < 1:
        ap.error("--jobs must be positive")
    frontend, cue = args.frontend.resolve(), args.cue.resolve()
    image = cue_bin(cue)
    entries = parse_entries(read_user_sectors(image, TOC_LBA, TOC_SECTORS))
    prefix, launch = menu_presses(entries, "CELESTE COLLECTION")
    args.out.mkdir(parents=True, exist_ok=False)
    identity = {"cue": str(cue), "cue_sha256": sha256(cue),
                "bin": str(image), "bin_sha256": sha256(image),
                "frontend": str(frontend), "frontend_sha256": sha256(frontend),
                "experimental_dma_fifo": os.environ.get("PSOXIDE_EXPERIMENTAL_DMA_FIFO"),
                "suite": args.suite}
    (args.out / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")

    def run(case):
        cart, chord, digital, suite = case
        name = f"c{cart + 1}-{'chord' if chord else 'pause'}-{'digital' if digital else 'analog'}"
        if suite != "basic":
            name = f"c{cart + 1}-{suite}-analog"
        folder = args.out / name
        shots = folder / "shots"
        shots.mkdir(parents=True)
        if suite == "stress":
            events, checks = stress_route(prefix, launch, cart)
        elif suite == "save":
            events, checks = save_route(prefix, launch, cart)
        else:
            events, checks = route(prefix, launch, cart, chord)
        last = max(c[0] for c in checks)
        cmd = [str(frontend), "--config-dir", str(folder / "config"), "launch",
               "--path", str(cue), "--embedded-playtest", "--steps", str((last + 300) * 300_000),
               "--press", ",".join(events), "--route-screenshot-dir", str(shots),
               "--route-screenshot-interval", "100", "--dump-hash"]
        if digital:
            cmd.append("--digital-pad")
        card = folder / "settings.mcd"
        if suite == "save":
            cmd += ["--memcard", str(card)]
        with (folder / "run.log").open("w") as log:
            try:
                rc = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT, timeout=600).returncode
            except subprocess.TimeoutExpired:
                rc = 124
        results = []
        for tick, want, detail in checks:
            path = shots / f"tick-{tick:06d}.ppm"
            got = screen(path) if path.exists() else ("missing", None)
            ok = got[0] == want and (detail is None or got[1] == detail)
            results.append({"tick": tick, "expected": [want, detail], "actual": got, "passed": ok})
            if path.exists():
                with Image.open(path) as im:
                    im.save(folder / f"check-{tick:06d}.png")
        log_text = (folder / "run.log").read_text()
        # Exactly one outer chain-load: an accidental return to the demo
        # carousel and relaunch must not masquerade as a collection return.
        chainloads = log_text.count("launcher: chain-loading")
        failures = [marker for marker in FAILURE_MARKERS if marker in log_text]
        if suite == "save":
            data = card.read_bytes() if card.exists() else b""
            if len(data) != 128 * 1024 or b"BESLES-00000CELSTCC1" not in data or b"CCS1\x07\x08" not in data:
                failures.append("changed SFX setting missing from memory card")
        ok = rc == 0 and chainloads == 1 and not failures and all(r["passed"] for r in results)
        result = {"name": name, "passed": ok, "rc": rc, "chainloads": chainloads,
                  "failures": failures, "checks": results, "command": cmd}
        (folder / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(f"{'PASS' if ok else 'FAIL'} {name}", flush=True)
        return result

    cases = []
    if args.suite in ("basic", "all"):
        cases += [(cart, chord, digital, "basic") for cart in (0, 1)
                  for chord in (False, True) for digital in (False, True)]
    if args.suite in ("stress", "all"):
        cases += [(cart, False, False, "stress") for cart in (0, 1)]
    if args.suite in ("save", "all"):
        cases += [(cart, False, False, "save") for cart in (0, 1)]
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        results = list(pool.map(run, cases))
    (args.out / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    passed = sum(r["passed"] for r in results)
    print(f"{passed}/{len(results)} routes passed")
    return int(passed != len(results))


if __name__ == "__main__":
    raise SystemExit(main())
