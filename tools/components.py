#!/usr/bin/env python3
"""Verify and bootstrap the exact repositories selected for this demo disc."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]

def git(path, *args):
    return subprocess.check_output(["git", "-C", str(path), *args], text=True).strip()

def run(check=False, check_main=False):
    lock = json.loads((ROOT / "release-components.json").read_text())
    if lock["schema"] != 1:
        raise ValueError("Unsupported component lock")
    specs = lock["components"]
    for name, spec in specs.items():
        source = ROOT / spec["path"]
        if git(source, "rev-parse", "HEAD") != spec["revision"]:
            raise RuntimeError(f"{name}: checkout does not match release-components.json")
        if git(source, "status", "--porcelain", "--untracked-files=normal"):
            raise RuntimeError(f"{name}: source checkout is dirty")
        if check_main:
            status = subprocess.check_output(["gh", "api", f"repos/{spec['repository']}/compare/{spec['revision']}...main", "--jq", ".status"], text=True).strip()
            if status not in ("identical", "ahead"):
                raise RuntimeError(f"{name}: selected revision is not on its repository main")
    editor = ROOT / specs["editor"]["path"]
    nested = json.loads((editor / "components.lock.json").read_text())["components"]
    for name in ("sdk", "emulator"):
        if nested[name]["revision"] != specs[name]["revision"]:
            raise RuntimeError(f"editor {name} pin differs from the disc component lock")
    for name in ("emulator", "editor"):
        source = ROOT / specs[name]["path"]
        command = [sys.executable, str(source / "tools/bootstrap-components.py")]
        if check or check_main:
            command += ["--check"]
        else:
            command += ["--source", f"sdk={ROOT / specs['sdk']['path']}", "--source", f"emulator={ROOT / specs['emulator']['path']}"]
        subprocess.run(command, check=True)
    print("Release component revisions and imported source receipts verified")


GAMES = ("voxide", "nitroxide", "psxcel", "pico8-psx", "gh-psx", "psoxide-arcade")


def verify_game_locks():
    specs = json.loads((ROOT / "release-components.json").read_text())["components"]
    for game in (*GAMES, "hl-psx"):
        source = ROOT / "games" / game
        lock = source / "components.lock.json"
        if lock.is_file():
            components = json.loads(lock.read_text())["components"]
            for name in ("sdk", "emulator", "editor"):
                for field in ("repository", "revision"):
                    if components[name][field] != specs[name][field]:
                        raise RuntimeError(f"{game}: standalone {name} {field} differs from the disc lock")
        else:
            import re
            manifest = (source / "psoxide-pin/Cargo.toml").read_text()
            pins = re.findall(r'rev\s*=\s*"([a-f0-9]{40})"', manifest)
            if pins != [specs["sdk"]["revision"]]:
                raise RuntimeError(f"{game}: standalone SDK pin differs from the disc lock")
    print("Every standalone game pin agrees with the release component tuple")


def verify_games(hl=False):
    verify_game_locks()
    specs = json.loads((ROOT / "release-components.json").read_text())["components"]
    editor = ROOT / specs["editor"]["path"]
    imported = json.loads((editor / ".components-receipt.json").read_text())["files"]
    # Compare both imported SDK/emulator files and editor-owned build inputs.
    owned = git(editor, "ls-files").splitlines()
    inputs = dict(imported)
    for name in owned:
        path = editor / name
        if path.is_file() and name.startswith(("engine/", "editor/crates/", "sdk/", "crates/")):
            inputs[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    selected = (*GAMES, "hl-psx") if hl else GAMES
    for game in selected:
        hydrated = ROOT / "games" / game / ".psoxide"
        marker = hydrated / ".psoxide-source"
        if not marker.is_file() or marker.read_text() != f"local:{editor}":
            raise RuntimeError(f"{game}: missing current shared-source hydration; run make programs")
        for name, expected in inputs.items():
            path = hydrated / name
            if not path.is_file() or path.is_symlink() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
                raise RuntimeError(f"{game}: hydrated build input changed or missing: {name}")
    print(f"Verified all {len(selected)} required game hydrations against the selected components")


def file_record(path):
    path = path.resolve(strict=True)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": digest.hexdigest()}

def receipt(output, cue, frontend):
    import release_receipt
    repositories = {}
    for line in git(ROOT, "ls-files", "--stage").splitlines():
        mode, revision, rest = line.split(" ", 2)
        if mode != "160000":
            continue
        path = rest.split("\t", 1)[1]
        source = ROOT / path
        if git(source, "rev-parse", "HEAD") != revision:
            raise RuntimeError(f"{path}: working revision differs from the selected gitlink")
        if git(source, "status", "--porcelain", "--untracked-files=normal"):
            raise RuntimeError(f"{path}: source is dirty")
        row = {"revision": revision}
        for name in ("components.lock.json", ".components-receipt.json",
                     ".psoxide/.components-receipt.json", ".psoxide/.psoxide-source"):
            if (source / name).is_file():
                row[name] = file_record(source / name)
        repositories[path] = row
    lock = json.loads((ROOT / "release-components.json").read_text())
    document = {"schema": 1, "components": lock["components"], "repositories": repositories,
                "demo_revision": git(ROOT, "rev-parse", "HEAD"),
                "build_recipe": file_record(ROOT / "Makefile"),
                "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
                "cargo": subprocess.check_output(["cargo", "--version"], text=True).strip(),
                "frontend": file_record(frontend), "cue": file_record(cue),
                "bin": file_record(release_receipt.image_for_cue(cue))}
    release_receipt.write_receipt(output, document)
    print(f"Complete component/game provenance: {output}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--check-main", action="store_true")
    parser.add_argument("--games", action="store_true")
    parser.add_argument("--game-locks", action="store_true")
    parser.add_argument("--hl", action="store_true")
    parser.add_argument("--receipt", type=Path)
    parser.add_argument("--cue", type=Path)
    parser.add_argument("--frontend", type=Path)
    args = parser.parse_args()
    try:
        run(args.check, args.check_main)
        if args.games:
            verify_games(args.hl)
        elif args.game_locks:
            verify_game_locks()
        if args.receipt:
            if not args.cue or not args.frontend:
                raise ValueError("--receipt requires --cue and --frontend")
            receipt(args.receipt, args.cue, args.frontend)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"components: {error}\n")
