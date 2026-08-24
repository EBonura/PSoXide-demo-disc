#!/usr/bin/env python3
"""Chain-load Quake twice off a combined disc, with no-image runtime proof."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path


STEPS = "500000000"
PRESS_ROUTE = "400:right:8,600:right:8,1000:cross:12"
EXPECTED_TICK = 500_000_000
EXPECTED_DISPLAY = (320, 240)
# The standard pressing hides the unfinished Cortex card, leaving eight
# visible programs plus CREDITS. The private HL pressing exposes Cortex and
# adds Half-Life, so its caller raises this to eleven. Quake stays immediately
# before CREDITS in both layouts.
DEFAULT_MENU_ENTRIES = 9
# The frame both replays end on, hashed by the emulator. Each pressing needs
# its own pair because its carousel table gives the launcher a different amount
# of work before the fixed instruction budget reaches Quake. Recompute these
# when the Quake, launcher, or ordinary-program PSoXide pin moves.
#
# Absolute cycle counts, route ticks, pad polls, CD command totals and log
# digests are deliberately NOT pinned here. They shift with the launcher binary,
# which changes on every commit to this repo (DISC_VERSION is `git describe`),
# so pinning them would have made the default gate fail on unrelated work. They
# are held to run-to-run equality instead, which is what determinism means.
EXPECTED_FRAME_FNV_BY_MENU_ENTRIES = {
    # Public pressing: unfinished Cortex cards are hidden.
    DEFAULT_MENU_ENTRIES: ("0xdd1fcc34dc5c3f1e", "0x3ac4a28cef8cb565"),
    # Private Half-Life pressing: Cortex and Half-Life are visible.
    11: ("0xb4e15cbaed40cff2", "0x1ba73537914d7c80"),
}
DETERMINISTIC_FIELDS = (
    "tick",
    "cycles",
    "pc_final",
    "route_ticks",
    "pad_polls",
    "vram_fnv",
    "display_fnv",
    "display_width",
    "display_height",
)
LOG_KINDS = ("route", "cd", "gpu", "pc", "pc_callsite", "pc_window")
MARKERS = (
    "launcher: booted",
    "launcher: chain-loading",
    "quake-psx: all-Rust PSoXide boot",
    "quake-psx: Rust Start map resident",
)
FAILURE_MARKERS = (
    "quake-psx: Rust graphics load failed",
    "quake-psx: Rust initial level load failed",
    "STACK/DATA COLLISION",
    "PANIC:",
)
TICK_SUMMARY = re.compile(
    r"tick=(\d+)\s+cycles=(\d+)\s+pc=(0x[0-9a-f]+)"
)
ROUTE_SUMMARY = re.compile(r"route-ticks=(\d+)\s+port1-polls=(\d+)")
VRAM = re.compile(r"vram_fnv1a_64=(0x[0-9a-f]+)")
DISPLAY = re.compile(
    r"display_fnv1a_64=(0x[0-9a-f]+)\s+w=(\d+)\s+h=(\d+)"
)

SECTOR_BYTES = 2_352
USER_DATA_AT = 24
USER_DATA_BYTES = 2_048
TOC_LBA = 22
TOC_SECTORS = 4
TOC_MAGIC = b"PSXDEMO4"
TOC_HEADER_BYTES = 0x16C
TOC_ENTRY_BYTES = 512
TOC_NAME_BYTES = 24
TOC_PAYLOAD_FNV_AT = 36
TOC_DESC_BYTES = 224
TOC_VERSION_AT = 40 + 2 * TOC_DESC_BYTES
TOC_VERSION_BYTES = 16
TOC_FLAGS_AT = 504
TOC_MAX_ENTRIES = (TOC_SECTORS * USER_DATA_BYTES - TOC_HEADER_BYTES) // TOC_ENTRY_BYTES
FLAG_HIDDEN = 1
QUAKE_ENTRY = "QUAKE SHAREWARE"
PSX_EXE_MAGIC = b"PS-X EXE"


class CheckError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def read_user_sectors(image: Path, lba: int, count: int) -> bytes:
    chunks = bytearray()
    with image.open("rb") as stream:
        for sector in range(lba, lba + count):
            stream.seek(sector * SECTOR_BYTES + USER_DATA_AT)
            chunk = stream.read(USER_DATA_BYTES)
            if len(chunk) != USER_DATA_BYTES:
                raise CheckError(
                    f"{image}: ended while reading {count} sectors at LBA {lba}"
                )
            chunks.extend(chunk)
    return bytes(chunks)


def route_button_count(button: str) -> int:
    return sum(
        1
        for press in PRESS_ROUTE.split(",")
        if press.split(":", 2)[1] == button
    )


def quake_menu_entry(
    image: Path, quake_lba: int, expected_menu_entries: int = DEFAULT_MENU_ENTRIES
) -> tuple[int, list[str], dict]:
    """Where QUAKE SHAREWARE sits in the carousel, and what the table says it is.

    Returns the selected index, the visible entry names, and the table's own
    record of the payload, so the caller can hold both against the receipt.
    """
    toc = read_user_sectors(image, TOC_LBA, TOC_SECTORS)
    if toc[:8] != TOC_MAGIC:
        raise CheckError(f"{image}: missing {TOC_MAGIC.decode()} at LBA {TOC_LBA}")
    count = int.from_bytes(toc[8:12], "little")
    if count == 0 or count > TOC_MAX_ENTRIES:
        raise CheckError(f"{image}: invalid demo table entry count {count}")

    visible: list[tuple[str, int]] = []
    payload: dict | None = None
    for index in range(count):
        at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES
        entry = toc[at : at + TOC_ENTRY_BYTES]
        name = entry[:TOC_NAME_BYTES].split(b"\0", 1)[0].decode("ascii")
        exe_lba = int.from_bytes(entry[TOC_NAME_BYTES : TOC_NAME_BYTES + 4], "little")
        flags = int.from_bytes(entry[TOC_FLAGS_AT : TOC_FLAGS_AT + 4], "little")
        if name == QUAKE_ENTRY:
            if payload is not None:
                raise CheckError(f"{image}: more than one {QUAKE_ENTRY} table entry")
            if flags & FLAG_HIDDEN:
                raise CheckError(f"{image}: {QUAKE_ENTRY} is hidden from the carousel")
            payload = {
                "exe_lba": exe_lba,
                "payload_fnv1a32": "0x{:08x}".format(
                    int.from_bytes(
                        entry[TOC_PAYLOAD_FNV_AT : TOC_PAYLOAD_FNV_AT + 4], "little"
                    )
                ),
                "menu_version": entry[TOC_VERSION_AT : TOC_VERSION_AT + TOC_VERSION_BYTES]
                .split(b"\0", 1)[0]
                .decode("ascii"),
            }
        if flags & FLAG_HIDDEN == 0:
            visible.append((name, exe_lba))
    if payload is None:
        raise CheckError(f"{image}: no {QUAKE_ENTRY} entry in the disc table")
    if visible and len(visible) < TOC_MAX_ENTRIES:
        visible.append(("CREDITS", 0))
    if len(visible) != expected_menu_entries:
        raise CheckError(
            f"{image}: carousel has {len(visible)} visible entries, "
            f"expected {expected_menu_entries}: "
            + ", ".join(name for name, _ in visible)
        )

    right_presses = route_button_count("right")
    cross_presses = route_button_count("cross")
    if right_presses != 2 or cross_presses != 1:
        raise CheckError("headless route no longer has two RIGHT presses and one CROSS")
    selected = (-right_presses) % len(visible)
    selected_name, selected_lba = visible[selected]
    if selected_name != QUAKE_ENTRY or selected_lba != quake_lba:
        raise CheckError(
            f"menu route selects {selected_name!r} at LBA {selected_lba}, "
            f"not Quake at LBA {quake_lba}"
        )
    expected_position = expected_menu_entries - 1
    if selected + 1 != expected_position:
        raise CheckError(
            f"{QUAKE_ENTRY} is carousel entry {selected + 1}, "
            f"expected {expected_position}"
        )
    return selected, [name for name, _ in visible], payload


def require_payload_identity(payload: dict, output: dict) -> None:
    """The disc the emulator just ran has to be the one the receipt describes."""
    recorded = output["quake_toc"]
    for field in ("exe_lba", "payload_fnv1a32", "menu_version"):
        if payload[field] != recorded[field]:
            raise CheckError(
                f"disc table {field} {payload[field]!r} does not match the "
                f"receipt's {recorded[field]!r}"
            )
    if output.get("embedded_quake_matches_input_except_msf") is not True:
        raise CheckError("receipt does not claim the embedded Quake image is the pinned one")
    if not output["embedded_quake_data_sectors"] > 0:
        raise CheckError("receipt records no embedded Quake data sectors")


def embedded_exe_evidence(image: Path, quake_lba: int) -> tuple[int, int, int]:
    header = read_user_sectors(image, quake_lba, 1)
    if header[:8] != PSX_EXE_MAGIC:
        raise CheckError(f"no PS-X EXE at Quake table LBA {quake_lba}")
    pc = int.from_bytes(header[0x10:0x14], "little")
    load_addr = int.from_bytes(header[0x18:0x1C], "little")
    payload_bytes = int.from_bytes(header[0x1C:0x20], "little")
    if payload_bytes == 0 or payload_bytes % USER_DATA_BYTES != 0:
        raise CheckError(f"invalid Quake payload size {payload_bytes}")
    if not load_addr <= pc < load_addr + payload_bytes:
        raise CheckError(
            f"Quake entry PC {pc:#010x} is outside payload "
            f"{load_addr:#010x}..{load_addr + payload_bytes:#010x}"
        )
    return pc, load_addr, payload_bytes


def require_runtime_markers(stdout: str) -> None:
    for marker in FAILURE_MARKERS:
        if marker in stdout:
            raise CheckError(f"headless output contains failure marker {marker!r}")
    cursor = 0
    for marker in MARKERS:
        if stdout.count(marker) != 1:
            raise CheckError(f"headless output does not contain exactly one {marker!r}")
        found = stdout.find(marker, cursor)
        if found < 0:
            raise CheckError(f"headless marker is absent or out of order: {marker!r}")
        cursor = found + len(marker)


def stdout_core(stdout: str) -> str:
    return "\n".join(
        line for line in stdout.splitlines() if not line.startswith("[cli] ")
    )


def run_once(frontend: Path, cue: Path, root: Path, name: str) -> dict[str, object]:
    run_root = root / name
    run_root.mkdir()
    paths = {
        "route": run_root / "route.csv",
        "cd": run_root / "cd.csv",
        "gpu": run_root / "gpu.csv",
        "pc": run_root / "pc.csv",
        "pc_callsite": run_root / "pc-callsite.csv",
        "pc_window": run_root / "pc-window.csv",
    }
    command = [
        str(frontend),
        "launch",
        "--embedded-playtest",
        "--path",
        str(cue),
        "--steps",
        STEPS,
        "--press",
        PRESS_ROUTE,
        "--route-log",
        str(paths["route"]),
        "--cd-command-log",
        str(paths["cd"]),
        "--gpu-frame-stats-log",
        str(paths["gpu"]),
        "--pc-sample-log",
        str(paths["pc"]),
        "--pc-sample-callsite-log",
        str(paths["pc_callsite"]),
        "--pc-sample-window-log",
        str(paths["pc_window"]),
        "--pc-sample-window-ticks",
        "300",
        "--pc-sample-instructions",
        "16384",
        "--dump-hash",
    ]
    result = subprocess.run(
        command,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    if result.returncode != 0:
        raise CheckError(f"headless replay {name} failed:\n{result.stdout}")
    require_runtime_markers(result.stdout)
    tick = TICK_SUMMARY.search(result.stdout)
    route = ROUTE_SUMMARY.search(result.stdout)
    vram = VRAM.search(result.stdout)
    display = DISPLAY.search(result.stdout)
    if tick is None or route is None or vram is None or display is None:
        raise CheckError(f"headless replay {name} did not print all summaries")
    return {
        "stdout_core": stdout_core(result.stdout),
        "tick": int(tick.group(1)),
        "cycles": int(tick.group(2)),
        "pc_final": int(tick.group(3), 16),
        "route_ticks": int(route.group(1)),
        "pad_polls": int(route.group(2)),
        "vram_fnv": vram.group(1),
        "display_fnv": display.group(1),
        "display_width": int(display.group(2)),
        "display_height": int(display.group(3)),
        **paths,
    }


def bcd(value: str) -> int:
    raw = int(value, 16)
    high, low = raw >> 4, raw & 0x0F
    if high > 9 or low > 9:
        raise CheckError(f"invalid BCD byte {value}")
    return high * 10 + low


def setloc_lba(row: dict[str, str]) -> int | None:
    if row["command"] != "0x02" or int(row["param_len"]) != 3:
        return None
    minute, second, frame = (bcd(part) for part in row["params"].split())
    return (minute * 60 + second) * 75 + frame - 150


def cd_evidence(
    path: Path,
    quake_lba: int,
    payload_bytes: int,
    image_lba: int,
    image_sectors: int,
) -> tuple[int, int, int]:
    with path.open(newline="", encoding="ascii") as stream:
        rows = list(csv.DictReader(stream))
    read_starts: list[int] = []
    seek_read_starts: list[int] = []
    for index, row in enumerate(rows):
        lba = setloc_lba(row)
        if lba is None:
            continue
        commands: set[str] = set()
        for following in rows[index + 1 :]:
            if following["command"] == "0x02":
                break
            commands.add(following["command"])
        if "0x06" in commands:
            read_starts.append(lba)
        if {"0x15", "0x06"}.issubset(commands):
            seek_read_starts.append(lba)
    required_starts = {quake_lba, quake_lba + 1}
    if not required_starts.issubset(seek_read_starts):
        raise CheckError(
            f"CD log does not seek/read Quake header and payload at "
            f"{quake_lba}/{quake_lba + 1}"
        )
    payload_sectors = payload_bytes // USER_DATA_BYTES
    payload_end = quake_lba + 1 + payload_sectors
    runtime_reads = sorted(
        lba
        for lba in read_starts
        if payload_end <= lba < image_lba + image_sectors
    )
    if not runtime_reads:
        raise CheckError(
            "CD log has no post-loader runtime read inside the relocated Quake image"
        )
    return len(rows), payload_sectors, runtime_reads[0]


def pc_evidence(path: Path, load_addr: int, payload_bytes: int) -> int:
    samples = 0
    with path.open(newline="", encoding="ascii") as stream:
        for row in csv.DictReader(stream):
            pc = int(row["pc"], 16)
            if load_addr <= pc < load_addr + payload_bytes:
                samples += int(row["samples"])
    if samples == 0:
        raise CheckError("PC sampler never observed the loaded Quake address range")
    return samples


def same(first: Path, second: Path, label: str) -> str:
    first_hash = sha256(first)
    second_hash = sha256(second)
    if first_hash != second_hash:
        raise CheckError(f"{label} differs between replays: {first_hash} != {second_hash}")
    return first_hash


def require_pins(
    result: dict[str, object], label: str, expected_menu_entries: int
) -> None:
    try:
        expected_vram, expected_display = EXPECTED_FRAME_FNV_BY_MENU_ENTRIES[
            expected_menu_entries
        ]
    except KeyError as error:
        raise CheckError(
            f"no frame pins for a {expected_menu_entries}-entry pressing"
        ) from error
    expected = {
        "tick": EXPECTED_TICK,
        "vram_fnv": expected_vram,
        "display_fnv": expected_display,
        "display_width": EXPECTED_DISPLAY[0],
        "display_height": EXPECTED_DISPLAY[1],
    }
    for key, value in expected.items():
        if result[key] != value:
            raise CheckError(f"{label} {key} {result[key]!r} != {value!r}")


def require_identical_replays(first: dict, second: dict) -> dict[str, str]:
    """Two replays of one disc have to agree on everything they observed."""
    if first["stdout_core"] != second["stdout_core"]:
        raise CheckError("programmatic stdout differs between replays")
    for field in DETERMINISTIC_FIELDS:
        if first[field] != second[field]:
            raise CheckError(
                f"{field} differs between replays: {first[field]!r} != {second[field]!r}"
            )
    return {
        kind: same(first[kind], second[kind], kind.replace("_", " "))
        for kind in LOG_KINDS
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend", required=True, type=Path)
    parser.add_argument("--cue", required=True, type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    parser.add_argument(
        "--expected-menu-entries", type=int, default=DEFAULT_MENU_ENTRIES
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        frontend = args.frontend.resolve(strict=True)
        cue = args.cue.resolve(strict=True)
        receipt_path = args.receipt.resolve(strict=True)
        receipt = json.loads(receipt_path.read_text(encoding="ascii"))
        if receipt.get("variant") != "quake-shareware-default":
            raise CheckError(f"wrong or missing Quake receipt: {receipt_path}")
        output = receipt["demo_disc_output"]
        quake_lba = output["quake_toc"]["exe_lba"]
        image_lba = output["quake_toc"]["image_lba_offset"]
        image_sectors = output["embedded_quake_data_sectors"]
        if cue.name != output["cue_file"] or sha256(cue) != output["cue_sha256"]:
            raise CheckError("launched cue does not match the receipt")
        image = (cue.parent / output["bin_file"]).resolve(strict=True)
        if image.parent != cue.parent or not image.is_file():
            raise CheckError("receipt BIN must be a regular file beside the cue")
        if sha256(image) != output["bin_sha256"]:
            raise CheckError("launched BIN does not match the receipt")
        cue_files = re.findall(
            r'^FILE "([^"]+)" BINARY$',
            cue.read_text(encoding="ascii"),
            flags=re.MULTILINE,
        )
        if cue_files != [image.name]:
            raise CheckError("cue does not point to the receipt BIN beside it")

        selected, menu, payload = quake_menu_entry(
            image, quake_lba, args.expected_menu_entries
        )
        require_payload_identity(payload, output)
        entry_pc, load_addr, payload_bytes = embedded_exe_evidence(image, quake_lba)
        with tempfile.TemporaryDirectory(prefix="psoxide-quake-chainload-") as directory:
            root = Path(directory)
            replays = {
                name: run_once(frontend, cue, root, name)
                for name in ("first", "second")
            }
            for name, replay in replays.items():
                require_pins(replay, name, args.expected_menu_entries)
                if not load_addr <= replay["pc_final"] < load_addr + payload_bytes:
                    raise CheckError(
                        f"{name} final PC {replay['pc_final']:#010x} is outside "
                        "the Quake payload"
                    )
            first, second = replays["first"], replays["second"]
            require_identical_replays(first, second)

            evidence = {
                name: cd_evidence(
                    replay["cd"], quake_lba, payload_bytes, image_lba, image_sectors
                )
                for name, replay in replays.items()
            }
            samples = {
                name: pc_evidence(replay["pc"], load_addr, payload_bytes)
                for name, replay in replays.items()
            }
            _, payload_sectors, _ = evidence["first"]

            print(f"disc: {image.name}")
            print(
                f"menu: {len(menu)} visible entries, {QUAKE_ENTRY} at "
                f"{selected + 1}/{len(menu)}: " + ", ".join(menu)
            )
            print(
                f"payload matches receipt: EXE LBA {payload['exe_lba']}, "
                f"FNV-1a-32 {payload['payload_fnv1a32']}, "
                f"menu version {payload['menu_version']}, "
                f"{image_sectors} embedded sectors"
            )
            print(f"loader: {payload_sectors} payload sectors; entry {entry_pc:#010x}")
            for name in ("first", "second"):
                replay, (replay_commands, _, replay_read) = replays[name], evidence[name]
                print(
                    f"chain-load {name}: booted, chain-loaded, Quake Start map "
                    f"resident; final PC {replay['pc_final']:#010x}; "
                    f"relocated runtime read LBA {replay_read}; "
                    f"{replay_commands} CD commands; "
                    f"{samples[name]} PC samples inside the payload"
                )
            print("both replays identical: stdout, summaries and all six logs")
            print(
                f"route ticks: {first['route_ticks']}; pad polls: {first['pad_polls']}; "
                f"cycles: {first['cycles']}"
            )
            print(
                f"VRAM/display FNV-1a-64: {first['vram_fnv']} / "
                f"{first['display_fnv']}"
            )
            print("no screenshots, frame dumps, audio dumps, or guest instrumentation used")
        return 0
    except (
        CheckError,
        FileNotFoundError,
        KeyError,
        OSError,
        TypeError,
        ValueError,
        json.JSONDecodeError,
    ) as error:
        print(f"quake-headless-check: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
