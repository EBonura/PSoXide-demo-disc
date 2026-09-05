#!/usr/bin/env python3
"""Stage both itch.io packages from one tested public disc, then publish them."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from check_release_chainloads import TOC_LBA, parse_toc, read_user_sector

ROOT = Path(__file__).resolve().parents[1]
PUBLIC_GAMES = [
    "CORTEX IGNITION", "QUAKE SHAREWARE", "VOXIDE", "NITROXIDE",
    "CELESTE COLLECTION", "PSOXIDE ARCADE", "GH-PSX", "PSXCEL",
    "HARDWARE TESTS", "CREDITS",
]
TRACK_TITLES = [
    "KNUCKLE DUST", "RUSTED HAMMER", "CHAINSAW HEART", "NIGHT CRAWLER",
    "CORTEX IGNITION COMBAT", "CORTEX IGNITION MENU", "GONCHAROV",
    "HARDWARE TESTS",
]


def public_image(cue):
    text = cue.read_text()
    files = re.findall(r'^FILE "([^"]+)" BINARY$', text, re.M)
    if len(files) != 1 or Path(files[0]).name != files[0]:
        raise ValueError("expected one adjacent BIN file")
    image = cue.parent / files[0]
    with image.open("rb") as stream:
        count = int.from_bytes(read_user_sector(stream, TOC_LBA)[8:12], "little")
    entries, menu = parse_toc(image)
    if menu != PUBLIC_GAMES or len(entries) != 9 or count != 9:
        raise ValueError("disc menu does not match the public collection")
    tracks = re.findall(r'^\s*TRACK (\d+) (\S+)', text, re.M)
    expected = [("01", "MODE2/2352")] + [(f"{n:02}", "AUDIO") for n in range(2, 10)]
    if tracks != expected:
        raise ValueError("expected the public disc's nine-track layout")
    return image


def run(*args, **kwargs):
    subprocess.run([str(a) for a in args], check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cue", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--emulator", type=Path, default=ROOT.parent / "PSoXide-emulator")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--web-build", type=Path, help="reuse an already built Trunk directory")
    parser.add_argument("--publish", action="store_true", help="upload BOTH verified packages")
    args = parser.parse_args()
    cue = args.cue.resolve()
    image = public_image(cue)
    if not re.fullmatch(r"v[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:[-.][A-Za-z0-9.]+)?", args.version):
        parser.error("provide a release version such as v0.34")
    out = args.out.resolve()
    if out.exists() and any(out.iterdir()):
        parser.error("output must be empty; retain earlier release packages separately")
    out.mkdir(parents=True, exist_ok=True)
    download, web = out / "download", out / "web"
    download.mkdir()
    for source in (cue, image, ROOT / "release/README.txt"):
        shutil.copy2(source, download / source.name)
    emulator = args.emulator.resolve()
    if args.web_build:
        shutil.copytree(args.web_build.resolve(), web)
    else:
        run(sys.executable, emulator / "tools/bootstrap-components.py", "--check")
        env = dict(os.environ)
        env.pop("NO_COLOR", None)
        env["RUSTFLAGS"] = "-C target-feature=+simd128 -C link-arg=-zstack-size=16777216"
        run("trunk", "build", "--release", "--public-url", "./", "--dist", web,
            cwd=emulator / "emu/crates/frontend", env=env)
    if not (web / "index.html").is_file() or not list(web.glob("*.wasm")):
        raise ValueError("web build is missing its entry page or emulator")
    # A reused build may contain an older disc. Only retain frontend assets.
    for pattern in ("demo-disc.*", "demo-data.*", "track-*.flac", "web-manifest.*"):
        for path in web.glob(pattern):
            path.unlink()
    web_cue = web / "demo-disc.cue"
    web_cue.write_text(re.sub(r'^FILE .*$', 'FILE "demo-disc.bin" BINARY', cue.read_text(), flags=re.M))
    run(sys.executable, ROOT / "tools/web-delivery.py", web_cue, image, web, *TRACK_TITLES)
    # web-delivery decodes every FLAC and reconstructs the original BIN before
    # writing its manifest. Neither public package can silently use an old disc.
    oversized = [p.name for p in web.rglob("*") if p.is_file() and p.stat().st_size >= 200_000_000]
    if oversized:
        raise ValueError(f"itch.io HTML file limit exceeded: {oversized}")
    receipt = {
        "version": args.version,
        "disc_sha256": hashlib.sha256(image.read_bytes()).hexdigest(),
        "disc_source": subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "HEAD"], text=True).strip(),
        "emulator_source": subprocess.check_output(["git", "-C", str(emulator), "rev-parse", "HEAD"], text=True).strip(),
        "download": "bonnie-studios/psoxide-demo-disc:psx",
        "playable": "bonnie-studios/psoxide:html5",
        "published": [],
    }
    receipt_path = out / "deployment.json"
    receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
    if args.publish:
        for key, directory in (("download", download), ("playable", web)):
            run("butler", "push", directory, receipt[key], "--userversion", args.version)
            receipt["published"].append(key)
            receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
        for key in ("download", "playable"):
            run("butler", "status", receipt[key])
    print(f"Both public packages staged: {out}")
    print("Verify both live itch.io pages. Google Drive is a separate, request-only upload.")


if __name__ == "__main__":
    main()
