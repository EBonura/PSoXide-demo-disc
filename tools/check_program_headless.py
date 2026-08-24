#!/usr/bin/env python3
"""Chain-load the demo disc's independent programs and verify live output."""

from __future__ import annotations

import argparse
import csv
import re
import shlex
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path


SECTOR_BYTES = 2_352
USER_DATA_AT = 24
USER_DATA_BYTES = 2_048
TOC_LBA = 22
TOC_SECTORS = 4
TOC_MAGIC = b"PSXDEMO4"
TOC_HEADER_BYTES = 0x16C
TOC_ENTRY_BYTES = 512
TOC_NAME_BYTES = 24
FLAG_HIDDEN = 1

TICK_SUMMARY = re.compile(r"tick=(\d+)\s+cycles=(\d+)\s+pc=(0x[0-9a-f]+)")
DISPLAY_SUMMARY = re.compile(
    r"display_fnv1a_64=(0x[0-9a-f]+)\s+w=(\d+)\s+h=(\d+)"
)
FAILURE_MARKERS = (
    "PANIC:",
    "psx_rt::halt",
    "STACK/DATA COLLISION",
    "payload checksum mismatch",
)


@dataclass(frozen=True)
class DiscEntry:
    name: str
    cdda_track_base: int
    hidden: bool


@dataclass(frozen=True)
class Route:
    name: str
    presses: str
    steps: int
    launcher_count: int = 1
    markers: tuple[str, ...] = ()
    cdda_owner: str | None = None


def cue_bin(cue: Path) -> Path:
    for line in cue.read_text(encoding="ascii").splitlines():
        words = shlex.split(line)
        if len(words) >= 2 and words[0].upper() == "FILE":
            image = Path(words[1])
            return image if image.is_absolute() else cue.parent / image
    raise ValueError(f"{cue}: no FILE line")


def read_user_sectors(image: Path, lba: int, count: int) -> bytes:
    out = bytearray()
    with image.open("rb") as stream:
        for sector in range(lba, lba + count):
            stream.seek(sector * SECTOR_BYTES + USER_DATA_AT)
            chunk = stream.read(USER_DATA_BYTES)
            if len(chunk) != USER_DATA_BYTES:
                raise ValueError(f"{image}: short read at LBA {sector}")
            out.extend(chunk)
    return bytes(out)


def parse_entries(toc: bytes) -> list[DiscEntry]:
    if len(toc) != TOC_SECTORS * USER_DATA_BYTES or toc[:8] != TOC_MAGIC:
        raise ValueError("demo table is absent or malformed")
    count = int.from_bytes(toc[8:12], "little")
    maximum = (len(toc) - TOC_HEADER_BYTES) // TOC_ENTRY_BYTES
    if count == 0 or count > maximum:
        raise ValueError(f"invalid demo table entry count {count}")
    entries: list[DiscEntry] = []
    for index in range(count):
        at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES
        raw = toc[at : at + TOC_ENTRY_BYTES]
        name = raw[:TOC_NAME_BYTES].split(b"\0", 1)[0].decode("ascii")
        words = TOC_NAME_BYTES
        cdda_track_base = int.from_bytes(raw[words + 8 : words + 12], "little")
        flags = int.from_bytes(raw[504:508], "little")
        entries.append(DiscEntry(name, cdda_track_base, bool(flags & FLAG_HIDDEN)))
    return entries


def menu_presses(entries: list[DiscEntry], target: str) -> tuple[list[str], int]:
    visible = [entry.name for entry in entries if not entry.hidden]
    if target not in visible:
        raise ValueError(f"{target!r} is not visible: {', '.join(visible)}")
    index = visible.index(target)
    count = len(visible) + 1  # Credits is the final non-program card.
    left = index
    right = count - index
    if left <= right:
        button, moves = "left", left
    else:
        button, moves = "right", right
    presses = [f"{400 + move * 200}:{button}:8" for move in range(moves)]
    # Let the final backdrop decode and carousel movement finish before launch.
    # Cards reached after several rapid moves can still be selected while X is
    # temporarily ignored during that visual transition.
    cross = max(1_000, (400 + (moves - 1) * 200 + 600) if moves else 1_000)
    presses.append(f"{cross}:cross:12")
    return presses, cross


def routes(entries: list[DiscEntry]) -> list[Route]:
    def outer(name: str) -> tuple[list[str], int]:
        return menu_presses(entries, name)

    voxide, _ = outer("VOXIDE")
    nitroxide, _ = outer("NITROXIDE")
    celeste, _ = outer("CELESTE COLLECTION")
    psxcel, _ = outer("PSXCEL")
    hardware, _ = outer("HARDWARE TESTS")

    gh, gh_cross = outer("GH-PSX")
    gh.extend((f"{gh_cross + 600}:start:12", f"{gh_cross + 1_000}:cross:12"))

    arcade, arcade_cross = outer("PSOXIDE ARCADE")
    inner_ready = arcade_cross + 600

    breakout = arcade + [f"{inner_ready}:cross:12", f"{inner_ready + 500}:cross:12"]
    invaders = arcade + [
        f"{inner_ready}:right:8",
        f"{inner_ready + 200}:cross:12",
        f"{inner_ready + 700}:cross:12",
    ]
    magikarp = arcade + [
        f"{inner_ready}:right:8",
        f"{inner_ready + 200}:right:8",
        f"{inner_ready + 400}:cross:12",
        f"{inner_ready + 900}:cross:12",
    ]

    return [
        Route("voxide", ",".join(voxide), 450_000_000, markers=("voxide: boot",)),
        Route(
            "nitroxide",
            ",".join(nitroxide),
            450_000_000,
            markers=("psx-engine: loading ready",),
        ),
        Route("celeste", ",".join(celeste), 450_000_000),
        # PSXcel is six carousel moves from the initial card. Give the launcher
        # enough emulated time to reach the delayed launch press before judging
        # the guest, rather than stopping while its card is merely selected.
        Route("psxcel", ",".join(psxcel), 450_000_000),
        Route(
            "gh-psx",
            ",".join(gh),
            800_000_000,
            markers=("gh-psx: cdda playing",),
            cdda_owner="GH-PSX",
        ),
        Route(
            "hardware-tests",
            ",".join(hardware),
            # The suite performs its real CD calibration before announcing the
            # menu. Stopping at 300M instructions can catch that valid probe in
            # progress and falsely report a missing boot marker.
            700_000_000,
            markers=("hardware-tests: main menu ready",),
        ),
        Route("arcade-breakout", ",".join(breakout), 700_000_000, launcher_count=2),
        Route(
            "arcade-invaders",
            ",".join(invaders),
            700_000_000,
            launcher_count=2,
            markers=("invaders: init ok",),
        ),
        Route(
            "arcade-magikarp",
            ",".join(magikarp),
            700_000_000,
            launcher_count=2,
            markers=("magikarp: cdda ok",),
            cdda_owner="PSOXIDE ARCADE",
        ),
    ]


def require_live_ppm(path: Path) -> None:
    data = path.read_bytes()
    try:
        magic, dimensions, maximum, pixels = data.split(b"\n", 3)
        width, height = (int(value) for value in dimensions.split())
    except (ValueError, TypeError) as error:
        raise ValueError(f"{path}: malformed PPM") from error
    if magic != b"P6" or maximum != b"255" or (width, height) != (320, 240):
        raise ValueError(f"{path}: expected a 320x240 P6 display")
    if len(pixels) != width * height * 3:
        raise ValueError(f"{path}: pixel payload has the wrong size")
    colours = set()
    bright = 0
    levels: list[int] = []
    for at in range(0, len(pixels), 3 * 17):
        colour = tuple(pixels[at : at + 3])
        if len(colour) != 3:
            continue
        colours.add(colour)
        level = max(colour)
        levels.append(level)
        bright += level > 40
    # The software display expands PS1 limited-range black to RGB 16, not
    # zero. Flat menu screens also have a deliberately tiny palette, so use
    # contrast and variation instead of demanding photographic colour counts.
    if (
        len(colours) < 4
        or bright < 20
        or min(levels, default=255) > 24
        or max(levels, default=0) - min(levels, default=0) < 40
    ):
        raise ValueError(f"{path}: display is blank or visually implausible")


def require_track(cd_log: Path, track: int) -> None:
    expected = f"{track:02d}"
    with cd_log.open(newline="", encoding="ascii") as stream:
        rows = list(csv.DictReader(stream))
    if not any(
        row["command"] == "0x03"
        and row["param_len"] == "1"
        and row["params"].strip() == expected
        for row in rows
    ):
        raise ValueError(f"{cd_log}: no Play command for relocated track {expected}")


def run_route(
    frontend: Path,
    cue: Path,
    output: Path,
    route: Route,
    by_name: dict[str, DiscEntry],
) -> str:
    run_dir = output / route.name
    run_dir.mkdir(parents=True, exist_ok=True)
    display = run_dir / "final.ppm"
    cd_log = run_dir / "cd.csv"
    command = [
        str(frontend),
        "launch",
        "--embedded-playtest",
        "--config-dir",
        str(run_dir / "config"),
        "--path",
        str(cue),
        "--steps",
        str(route.steps),
        "--press",
        route.presses,
        "--guest-debug-log",
        "--cd-command-log",
        str(cd_log),
        "--dump-display",
        str(display),
        "--dump-hash",
    ]
    (run_dir / "command.txt").write_text(shlex.join(command) + "\n", encoding="utf-8")
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    (run_dir / "stdout.txt").write_text(result.stdout, encoding="utf-8")
    if result.returncode:
        raise ValueError(f"{route.name}: frontend exited {result.returncode}\n{result.stdout}")
    for marker in FAILURE_MARKERS:
        if marker in result.stdout:
            raise ValueError(f"{route.name}: output contains failure marker {marker!r}")
    if result.stdout.count("launcher: booted") != route.launcher_count:
        raise ValueError(f"{route.name}: wrong launcher boot count")
    if result.stdout.count("launcher: chain-loading") != route.launcher_count:
        raise ValueError(f"{route.name}: wrong chain-load count")
    for marker in route.markers:
        if marker not in result.stdout:
            raise ValueError(f"{route.name}: missing runtime marker {marker!r}")
    tick = TICK_SUMMARY.search(result.stdout)
    display_summary = DISPLAY_SUMMARY.search(result.stdout)
    if tick is None or int(tick.group(1)) != route.steps:
        raise ValueError(f"{route.name}: did not retire the requested instruction count")
    pc = int(tick.group(3), 16)
    if not 0x8001_0000 <= pc < 0x8020_0000:
        raise ValueError(f"{route.name}: final PC {pc:#010x} is outside PlayStation RAM")
    if display_summary is None or tuple(map(int, display_summary.groups()[1:])) != (320, 240):
        raise ValueError(f"{route.name}: missing 320x240 display summary")
    require_live_ppm(display)
    if route.cdda_owner is not None:
        owner = by_name[route.cdda_owner]
        require_track(cd_log, owner.cdda_track_base + 2)
    return f"{route.name}: PASS {display_summary.group(1)} pc={pc:#010x}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend", required=True, type=Path)
    parser.add_argument("--cue", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--only", action="append", default=[])
    args = parser.parse_args()

    entries = parse_entries(read_user_sectors(cue_bin(args.cue), TOC_LBA, TOC_SECTORS))
    by_name = {entry.name: entry for entry in entries}
    selected = routes(entries)
    if args.only:
        wanted = set(args.only)
        selected = [route for route in selected if route.name in wanted]
        missing = wanted - {route.name for route in selected}
        if missing:
            parser.error("unknown route(s): " + ", ".join(sorted(missing)))

    args.out.mkdir(parents=True, exist_ok=True)
    failures: list[str] = []
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        pending = {
            pool.submit(run_route, args.frontend, args.cue, args.out, route, by_name): route
            for route in selected
        }
        for future in as_completed(pending):
            route = pending[future]
            try:
                print(future.result(), flush=True)
            except Exception as error:  # Report every failed route in one pass.
                failures.append(f"{route.name}: FAIL {error}")
                print(failures[-1], file=sys.stderr, flush=True)
    if failures:
        print(f"program headless check: {len(failures)} failure(s)", file=sys.stderr)
        return 1
    print(f"program headless check: PASS ({len(selected)} routes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
