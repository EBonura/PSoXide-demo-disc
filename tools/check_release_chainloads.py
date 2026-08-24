#!/usr/bin/env python3
"""Deterministically chain-load and exercise the release-critical disc entries."""

from __future__ import annotations

import argparse
import csv
import hashlib
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path


SECTOR_BYTES = 2352
USER_DATA_AT = 24
USER_DATA_BYTES = 2048
TOC_LBA = 22
TOC_SECTORS = 4
TOC_MAGIC = b"PSXDEMO4"
TOC_HEADER_BYTES = 0x16C
TOC_ENTRY_BYTES = 512
TOC_NAME_BYTES = 24
TOC_VERSION_AT = 488
TOC_VERSION_BYTES = 16
TOC_FLAGS_AT = 504
FLAG_HIDDEN = 1
PSX_EXE_MAGIC = b"PS-X EXE"
DEFAULT_STEPS = 700_000_000
TARGETS = (
    "CORTEX IGNITION",
    "CORTEX IGNITION LEGACY",
    "HALF-LIFE",
    "HARDWARE TESTS",
    "QUAKE SHAREWARE",
)
TARGET_MARKERS = {
    "CORTEX IGNITION": (),
    "CORTEX IGNITION LEGACY": (),
    "HALF-LIFE": ("hl-psx: booting renderer",),
    "HARDWARE TESTS": ("hardware-tests: main menu ready",),
    "QUAKE SHAREWARE": (
        "quake-psx: all-Rust PSoXide boot",
        "quake-psx: Rust Start map resident",
    ),
}
TARGET_FAILURES = {
    "CORTEX IGNITION": ("PANIC:", "STACK/DATA COLLISION"),
    "CORTEX IGNITION LEGACY": ("PANIC:", "STACK/DATA COLLISION"),
    "HALF-LIFE": (
        "PANIC:",
        "STACK/DATA COLLISION",
        "hl-psx: WORLD.PAK texture stream failed",
        "hl-psx: WORLD.PAK world stream failed",
    ),
    "HARDWARE TESTS": ("PANIC:", "STACK/DATA COLLISION"),
    "QUAKE SHAREWARE": (
        "PANIC:",
        "STACK/DATA COLLISION",
        "quake-psx: Rust graphics load failed",
        "quake-psx: Rust initial level load failed",
    ),
}
REQUIRE_RUNTIME_READ = {
    "CORTEX IGNITION",
    "CORTEX IGNITION LEGACY",
    "HALF-LIFE",
    "QUAKE SHAREWARE",
}
TICK_SUMMARY = re.compile(r"tick=(\d+)\s+cycles=(\d+)\s+pc=(0x[0-9a-f]+)")
ROUTE_SUMMARY = re.compile(r"route-ticks=(\d+)\s+port1-polls=(\d+)")
VRAM = re.compile(r"vram_fnv1a_64=(0x[0-9a-f]+)")
DISPLAY = re.compile(r"display_fnv1a_64=(0x[0-9a-f]+)\s+w=(\d+)\s+h=(\d+)")

# These are deliberately below the accepted current Cortex gameplay load
# (median 353 textured triangles/49 quads, maxima 370/52), but above every
# frame of Cortex's own menu/loading sequence (zero textured triangles and
# <=31 quads).
# Requiring both therefore proves that the launcher entered Cortex, Cortex's
# menu accepted input, and a textured model plus the textured world rendered.
CORTEX_GAMEPLAY_TRIANGLES = 300
CORTEX_GAMEPLAY_QUADS = 40
CORTEX_GAMEPLAY_MIN_FRAMES = 30
CORTEX_GAMEPLAY_MAX_FRAME_GAP = 16
CORTEX_GAMEPLAY_MIN_HASHES = 8
# A polygon-count gate cannot distinguish a correctly rendered room from a
# deterministic frame that draws only its floor and character.  Cortex's
# release start deliberately faces a textured wall, so require real high-
# frequency world detail in the upper playfield as a visual-semantic oracle.
# The ROI excludes the HUD and player silhouette.  The broken exterior-facing
# capture measured 92 permille; the corrected authored view measures 551.
CORTEX_GEOMETRY_EDGE_DELTA = 24
CORTEX_GEOMETRY_MIN_EDGE_PERMILLE = 250


class CheckError(RuntimeError):
    pass


@dataclass(frozen=True)
class Entry:
    name: str
    exe_lba: int
    image_lba: int
    payload_fnv: int
    version: str
    flags: int
    visible_index: int


@dataclass(frozen=True)
class Exe:
    pc: int
    load: int
    payload_bytes: int


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def read_user_sector(stream, lba: int) -> bytes:
    stream.seek(lba * SECTOR_BYTES + USER_DATA_AT)
    data = stream.read(USER_DATA_BYTES)
    if len(data) != USER_DATA_BYTES:
        raise CheckError(f"disc ended at LBA {lba}")
    return data


def parse_toc(image: Path) -> tuple[list[Entry], list[str]]:
    raw = bytearray()
    with image.open("rb") as stream:
        for lba in range(TOC_LBA, TOC_LBA + TOC_SECTORS):
            raw.extend(read_user_sector(stream, lba))
    if raw[:8] != TOC_MAGIC:
        raise CheckError(f"missing {TOC_MAGIC!r} at LBA {TOC_LBA}")
    count = int.from_bytes(raw[8:12], "little")
    maximum = (len(raw) - TOC_HEADER_BYTES) // TOC_ENTRY_BYTES
    if count == 0 or count > maximum:
        raise CheckError(f"invalid TOC entry count {count}")
    decoded: list[tuple[str, int, int, int, str, int]] = []
    for index in range(count):
        at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES
        row = raw[at : at + TOC_ENTRY_BYTES]
        name = row[:TOC_NAME_BYTES].split(b"\0", 1)[0].decode("ascii")
        version = row[TOC_VERSION_AT : TOC_VERSION_AT + TOC_VERSION_BYTES].split(
            b"\0", 1
        )[0].decode("ascii")
        decoded.append(
            (
                name,
                int.from_bytes(row[24:28], "little"),
                int.from_bytes(row[28:32], "little"),
                int.from_bytes(row[36:40], "little"),
                version,
                int.from_bytes(row[TOC_FLAGS_AT : TOC_FLAGS_AT + 4], "little"),
            )
        )
    visible_names = [row[0] for row in decoded if row[5] & FLAG_HIDDEN == 0]
    visible = visible_names + (["CREDITS"] if visible_names else [])
    entries = [
        Entry(*row, visible_names.index(row[0]))
        for row in decoded
        if row[5] & FLAG_HIDDEN == 0
    ]
    return entries, visible


def parse_exe(image: Path, entry: Entry) -> Exe:
    with image.open("rb") as stream:
        header = read_user_sector(stream, entry.exe_lba)
    if header[:8] != PSX_EXE_MAGIC:
        raise CheckError(f"{entry.name}: no PS-X EXE at LBA {entry.exe_lba}")
    exe = Exe(
        int.from_bytes(header[0x10:0x14], "little"),
        int.from_bytes(header[0x18:0x1C], "little"),
        int.from_bytes(header[0x1C:0x20], "little"),
    )
    if exe.payload_bytes == 0 or exe.payload_bytes % USER_DATA_BYTES:
        raise CheckError(f"{entry.name}: invalid payload size {exe.payload_bytes}")
    if not exe.load <= exe.pc < exe.load + exe.payload_bytes:
        raise CheckError(f"{entry.name}: entry PC is outside its payload")
    digest = 0x811C9DC5
    remaining = exe.payload_bytes
    with image.open("rb") as stream:
        lba = entry.exe_lba + 1
        while remaining:
            for byte in read_user_sector(stream, lba)[:remaining]:
                digest = ((digest ^ byte) * 0x01000193) & 0xFFFFFFFF
            remaining -= min(remaining, USER_DATA_BYTES)
            lba += 1
    if digest != entry.payload_fnv:
        raise CheckError(
            f"{entry.name}: payload FNV {digest:#010x} != TOC {entry.payload_fnv:#010x}"
        )
    return exe


def launcher_route(index: int, count: int) -> tuple[list[str], int]:
    left = index
    right = (-index) % count
    button, presses = ("left", left) if left <= right else ("right", right)
    events = [f"{400 + i * 200}:{button}:8" for i in range(presses)]
    cross_tick = max(1000, 400 + presses * 200 + 200)
    events.append(f"{cross_tick}:cross:12")
    return events, cross_tick


def route_for(target: str, index: int, count: int) -> str:
    events, cross_tick = launcher_route(index, count)
    if target == "CORTEX IGNITION":
        # Cortex has its own menu after the demo-disc carousel. Sparse presses
        # remain deterministic across the two scene loads and enter the first
        # playable project without depending on a single timing edge.
        events.extend(
            f"{cross_tick + offset}:cross:12"
            for offset in (400, 800, 1200, 1600, 2000, 2400)
        )
    return ",".join(events)


def cortex_gameplay_evidence(path: Path) -> dict[str, int]:
    qualifying: list[tuple[int, str]] = []
    with path.open(newline="", encoding="ascii") as stream:
        for row in csv.DictReader(stream):
            if (
                int(row["textured_tris"]) >= CORTEX_GAMEPLAY_TRIANGLES
                and int(row["textured_quads"]) >= CORTEX_GAMEPLAY_QUADS
            ):
                qualifying.append((int(row["route_tick"]), row["frame_draw_hash"]))

    longest = 0
    run = 0
    previous: int | None = None
    for tick, _ in qualifying:
        if previous is not None and tick - previous <= CORTEX_GAMEPLAY_MAX_FRAME_GAP:
            run += 1
        else:
            run = 1
        longest = max(longest, run)
        previous = tick
    hashes = len({digest for _, digest in qualifying})
    if len(qualifying) < CORTEX_GAMEPLAY_MIN_FRAMES:
        raise CheckError(
            "CORTEX IGNITION: only "
            f"{len(qualifying)} textured gameplay frames; "
            f"need {CORTEX_GAMEPLAY_MIN_FRAMES}"
        )
    if longest < CORTEX_GAMEPLAY_MIN_FRAMES:
        raise CheckError(
            "CORTEX IGNITION: textured frames were not sustained "
            f"({longest} in one run)"
        )
    if hashes < CORTEX_GAMEPLAY_MIN_HASHES:
        raise CheckError(
            "CORTEX IGNITION: textured gameplay did not animate "
            f"({hashes} distinct frame hashes)"
        )
    return {"frames": len(qualifying), "sustained": longest, "hashes": hashes}


def cortex_geometry_evidence(path: Path) -> int:
    payload = path.read_bytes()
    header = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", payload)
    if header is None:
        raise CheckError("CORTEX IGNITION: malformed P6 display dump")
    width, height = (int(value) for value in header.groups())
    pixels = payload[header.end() :]
    if len(pixels) != width * height * 3:
        raise CheckError("CORTEX IGNITION: truncated display dump")
    if (width, height) != (320, 240):
        raise CheckError(
            f"CORTEX IGNITION: unexpected display size {width}x{height}"
        )

    edge_pixels = 0
    compared = 0
    for y in range(36, 150):
        for x in range(32, 287):
            offset = (y * width + x) * 3
            delta = sum(
                abs(pixels[offset + channel] - pixels[offset + 3 + channel])
                for channel in range(3)
            )
            edge_pixels += delta >= CORTEX_GEOMETRY_EDGE_DELTA
            compared += 1
    edge_permille = edge_pixels * 1000 // compared
    if edge_permille < CORTEX_GEOMETRY_MIN_EDGE_PERMILLE:
        raise CheckError(
            "CORTEX IGNITION: upper playfield lacks textured geometry "
            f"({edge_permille} edge permille; need "
            f"{CORTEX_GEOMETRY_MIN_EDGE_PERMILLE})"
        )
    return edge_permille


def stdout_core(output: str) -> str:
    return "\n".join(line for line in output.splitlines() if not line.startswith("[cli] "))


def run_once(
    frontend: Path,
    cue: Path,
    target: Entry,
    exe: Exe,
    menu_count: int,
    steps: int,
    root: Path,
    label: str,
) -> dict[str, object]:
    run_dir = root / f"{target.name.lower().replace(' ', '-')}-{label}"
    run_dir.mkdir()
    paths = {kind: run_dir / f"{kind}.csv" for kind in ("route", "cd", "gpu", "pc")}
    paths["display_ppm"] = run_dir / "display.ppm"
    command = [
        str(frontend),
        "launch",
        "--embedded-playtest",
        "--path",
        str(cue),
        "--steps",
        str(steps),
        "--press",
        route_for(target.name, target.visible_index, menu_count),
        "--route-log",
        str(paths["route"]),
        "--cd-command-log",
        str(paths["cd"]),
        "--gpu-frame-stats-log",
        str(paths["gpu"]),
        "--pc-sample-log",
        str(paths["pc"]),
        "--dump-display",
        str(paths["display_ppm"]),
        "--dump-hash",
    ]
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise CheckError(f"{target.name} {label} failed:\n{result.stdout}")
    for marker in ("launcher: booted", "launcher: chain-loading", *TARGET_MARKERS[target.name]):
        if result.stdout.count(marker) != 1:
            raise CheckError(f"{target.name}: expected one {marker!r}")
    for marker in TARGET_FAILURES[target.name]:
        if marker in result.stdout:
            raise CheckError(f"{target.name}: found failure marker {marker!r}")
    tick = TICK_SUMMARY.search(result.stdout)
    route = ROUTE_SUMMARY.search(result.stdout)
    vram = VRAM.search(result.stdout)
    display = DISPLAY.search(result.stdout)
    if None in (tick, route, vram, display):
        raise CheckError(f"{target.name}: incomplete emulator summary")
    final_pc = int(tick.group(3), 16)
    if not exe.load <= final_pc < exe.load + exe.payload_bytes:
        raise CheckError(f"{target.name}: final PC {final_pc:#010x} outside payload")
    samples = 0
    with paths["pc"].open(newline="", encoding="ascii") as stream:
        for row in csv.DictReader(stream):
            pc = int(row["pc"], 16)
            if exe.load <= pc < exe.load + exe.payload_bytes:
                samples += int(row["samples"])
    if not samples:
        raise CheckError(f"{target.name}: PC sampler never observed payload code")
    gameplay = (
        cortex_gameplay_evidence(paths["gpu"])
        if target.name == "CORTEX IGNITION"
        else {"frames": 0, "sustained": 0, "hashes": 0}
    )
    geometry_edge_permille = (
        cortex_geometry_evidence(paths["display_ppm"])
        if target.name == "CORTEX IGNITION"
        else 0
    )
    return {
        "stdout": stdout_core(result.stdout),
        "tick": int(tick.group(1)),
        "cycles": int(tick.group(2)),
        "pc_final": final_pc,
        "route_ticks": int(route.group(1)),
        "pad_polls": int(route.group(2)),
        "vram": vram.group(1),
        "display": display.group(1),
        "width": int(display.group(2)),
        "height": int(display.group(3)),
        "samples": samples,
        "gameplay_frames": gameplay["frames"],
        "gameplay_sustained": gameplay["sustained"],
        "gameplay_hashes": gameplay["hashes"],
        "geometry_edge_permille": geometry_edge_permille,
        **paths,
    }


def bcd(text: str) -> int:
    value = int(text, 16)
    return (value >> 4) * 10 + (value & 15)


def setloc_lba(row: dict[str, str]) -> int | None:
    if row["command"] != "0x02" or int(row["param_len"]) != 3:
        return None
    minute, second, frame = (bcd(value) for value in row["params"].split())
    return (minute * 60 + second) * 75 + frame - 150


def cd_evidence(path: Path, entry: Entry, exe: Exe) -> tuple[int, int | None]:
    with path.open(newline="", encoding="ascii") as stream:
        rows = list(csv.DictReader(stream))
    read_starts: list[int] = []
    for index, row in enumerate(rows):
        lba = setloc_lba(row)
        if lba is None:
            continue
        commands = set()
        for following in rows[index + 1 :]:
            if following["command"] == "0x02":
                break
            commands.add(following["command"])
        if "0x06" in commands:
            read_starts.append(lba)
    if entry.exe_lba not in read_starts or entry.exe_lba + 1 not in read_starts:
        raise CheckError(f"{entry.name}: loader did not read header and payload")
    payload_end = entry.exe_lba + 1 + exe.payload_bytes // USER_DATA_BYTES
    runtime = next((lba for lba in read_starts if lba >= payload_end), None)
    if entry.name in REQUIRE_RUNTIME_READ and runtime is None:
        raise CheckError(f"{entry.name}: no post-loader runtime CD read")
    return len(rows), runtime


def compare(first: dict[str, object], second: dict[str, object], target: str) -> None:
    for key in (
        "stdout", "tick", "cycles", "pc_final", "route_ticks", "pad_polls",
        "vram", "display", "width", "height", "samples", "gameplay_frames",
        "gameplay_sustained", "gameplay_hashes", "geometry_edge_permille",
    ):
        if first[key] != second[key]:
            raise CheckError(
                f"{target}: {key} differs between replays: "
                f"{first[key]!r} != {second[key]!r}"
            )
    for key in ("route", "cd", "gpu", "pc", "display_ppm"):
        if sha256(first[key]) != sha256(second[key]):
            raise CheckError(f"{target}: {key} log differs between replays")


def image_for_cue(cue: Path) -> Path:
    names = re.findall(
        r'^FILE "([^"]+)" BINARY$', cue.read_text(encoding="ascii"), re.MULTILINE
    )
    unique = list(dict.fromkeys(names))
    if len(unique) != 1:
        raise CheckError("cue must name exactly one combined BIN")
    image = (cue.parent / unique[0]).resolve(strict=True)
    if image.parent != cue.parent:
        raise CheckError("cue BIN must sit beside the cue")
    return image


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend", required=True, type=Path)
    parser.add_argument("--cue", required=True, type=Path)
    parser.add_argument("--steps", type=int, default=DEFAULT_STEPS)
    parser.add_argument("--target", action="append", choices=TARGETS)
    parser.add_argument(
        "--artifact-dir",
        type=Path,
        help="retain the two-run route/CD/GPU/PC logs in this empty directory",
    )
    args = parser.parse_args()
    try:
        frontend = args.frontend.resolve(strict=True)
        cue = args.cue.resolve(strict=True)
        image = image_for_cue(cue)
        entries, menu = parse_toc(image)
        by_name = {entry.name: entry for entry in entries}
        targets = args.target or list(TARGETS)
        print(f"frontend SHA-256: {sha256(frontend)}")
        print(f"cue SHA-256: {sha256(cue)}")
        print(f"bin SHA-256: {sha256(image)}")
        print(f"menu ({len(menu)}): " + ", ".join(menu))
        temporary: tempfile.TemporaryDirectory[str] | None = None
        if args.artifact_dir is None:
            temporary = tempfile.TemporaryDirectory(prefix="psoxide-release-chainload-")
            root = Path(temporary.name)
        else:
            root = args.artifact_dir.resolve()
            if root.exists() and any(root.iterdir()):
                raise CheckError(f"artifact directory is not empty: {root}")
            root.mkdir(parents=True, exist_ok=True)
        try:
            for name in targets:
                if name not in by_name:
                    raise CheckError(f"required visible entry is absent: {name}")
                entry = by_name[name]
                exe = parse_exe(image, entry)
                first = run_once(frontend, cue, entry, exe, len(menu), args.steps, root, "first")
                second = run_once(frontend, cue, entry, exe, len(menu), args.steps, root, "second")
                compare(first, second, name)
                commands, runtime_lba = cd_evidence(first["cd"], entry, exe)
                print(
                    f"{name}: PASS route={route_for(name, entry.visible_index, len(menu))} "
                    f"version={entry.version or '-'} EXE-LBA={entry.exe_lba} "
                    f"image-LBA={entry.image_lba} payload-FNV={entry.payload_fnv:#010x} "
                    f"PC={first['pc_final']:#010x} samples={first['samples']} CD={commands} "
                    f"runtime-read={runtime_lba} VRAM={first['vram']} display={first['display']} "
                    f"gameplay={first['gameplay_frames']}/{first['gameplay_sustained']} "
                    f"hashes={first['gameplay_hashes']} "
                    f"geometry-edge={first['geometry_edge_permille']}permille"
                )
        finally:
            if temporary is not None:
                temporary.cleanup()
        print("all release-critical chain-load replays are byte-deterministic")
        return 0
    except (CheckError, FileNotFoundError, OSError, ValueError) as error:
        print(f"release-chainload-check: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
