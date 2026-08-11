#!/usr/bin/env python3
"""Replay the Quake demo-disc route twice and require deterministic gameplay."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import re
import subprocess
import sys
import tempfile
import wave
from array import array
from pathlib import Path


STEPS = "1400000000"
PRESS_ROUTE = (
    "400:right:8,500:right:8,600:right:8,700:right:8,800:right:8,"
    "900:right:8,1000:right:8,1100:right:8,1200:right:8,1500:right:8,"
    "1900:cross:12,4500:cross:12"
)
EXPECTED_ROUTE_TICKS = 7020
EXPECTED_PAD_POLLS = 1562
EXPECTED_CD_COMMANDS = 1185
EXPECTED_DISPLAY_FNV = "0xa2fab28830c30bf7"
EXPECTED_DISPLAY_SHA256 = "cd1d83c7e54e73bd94672732e9a2519157f8d5a0e971f0c0ee1d917274c0b387"
EXPECTED_AUDIO_SHA256 = "ea300e2cca8459706a0d6d67b2e4c25419cad50a6af3ceb992cbb33dc061bac1"
SUMMARY = re.compile(r"route-ticks=(\d+)\s+port1-polls=(\d+)")
DISPLAY = re.compile(r"display_fnv1a_64=(0x[0-9a-f]+)")


class CheckError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def run_once(frontend: Path, cue: Path, root: Path, name: str) -> dict[str, object]:
    route = root / f"{name}-route.csv"
    cd = root / f"{name}-cd.csv"
    audio = root / f"{name}.wav"
    display = root / f"{name}.ppm"
    command = [
        str(frontend),
        "launch",
        "--path",
        str(cue),
        "--steps",
        STEPS,
        "--press",
        PRESS_ROUTE,
        "--route-log",
        str(route),
        "--cd-command-log",
        str(cd),
        "--dump-audio",
        str(audio),
        "--dump-display",
        str(display),
        "--dump-hash",
    ]
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode != 0:
        raise CheckError(f"headless replay {name} failed:\n{result.stdout}")
    summary = SUMMARY.search(result.stdout)
    display_hash = DISPLAY.search(result.stdout)
    if summary is None or display_hash is None:
        raise CheckError(f"headless replay {name} did not print its route/display summary")
    return {
        "stdout": result.stdout,
        "route_ticks": int(summary.group(1)),
        "pad_polls": int(summary.group(2)),
        "display_fnv": display_hash.group(1),
        "route": route,
        "cd": cd,
        "audio": audio,
        "display": display,
    }


def bcd(value: str) -> int:
    raw = int(value, 16)
    high, low = raw >> 4, raw & 0x0F
    if high > 9 or low > 9:
        raise CheckError(f"invalid BCD byte {value}")
    return high * 10 + low


def cd_evidence(path: Path, quake_lba: int) -> tuple[int, int]:
    commands = 0
    highest_read_lba = -1
    with path.open(newline="", encoding="ascii") as stream:
        for row in csv.DictReader(stream):
            commands += 1
            if row["command"] != "0x02" or int(row["param_len"]) != 3:
                continue
            minute, second, frame = (bcd(part) for part in row["params"].split())
            lba = (minute * 60 + second) * 75 + frame - 150
            highest_read_lba = max(highest_read_lba, lba)
    if highest_read_lba < quake_lba:
        raise CheckError(
            f"highest CD read LBA {highest_read_lba} never reached Quake image LBA {quake_lba}"
        )
    return commands, highest_read_lba


def audio_evidence(path: Path) -> tuple[int, int, int, int]:
    with wave.open(str(path), "rb") as stream:
        channels = stream.getnchannels()
        rate = stream.getframerate()
        frames = stream.getnframes()
        width = stream.getsampwidth()
        if channels != 2 or rate != 44100 or width != 2:
            raise CheckError(
                f"unexpected WAV format: channels={channels}, rate={rate}, width={width}"
            )
        samples = array("h", stream.readframes(frames))
    if sys.byteorder != "little":
        samples.byteswap()
    peak = max((abs(value) for value in samples), default=0)
    rms = math.isqrt(sum(value * value for value in samples) // max(1, len(samples)))
    if peak == 0 or rms == 0:
        raise CheckError("headless Quake route produced silent audio")
    return frames, peak, rms, rate


def same(first: Path, second: Path, label: str) -> None:
    first_hash = sha256(first)
    second_hash = sha256(second)
    if first_hash != second_hash:
        raise CheckError(f"{label} differs between replays: {first_hash} != {second_hash}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend", required=True, type=Path)
    parser.add_argument("--cue", required=True, type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        frontend = args.frontend.resolve(strict=True)
        cue = args.cue.resolve(strict=True)
        receipt_path = args.receipt.resolve(strict=True)
        receipt = json.loads(receipt_path.read_text(encoding="ascii"))
        if receipt.get("variant") != "quake-shareware-local-test":
            raise CheckError(f"wrong or missing Quake receipt: {receipt_path}")
        quake_lba = receipt["demo_disc_output"]["quake_toc"]["image_lba_offset"]

        with tempfile.TemporaryDirectory(prefix="psoxide-quake-headless-") as directory:
            root = Path(directory)
            first = run_once(frontend, cue, root, "first")
            second = run_once(frontend, cue, root, "second")
            for label, result in (("first", first), ("second", second)):
                if result["route_ticks"] != EXPECTED_ROUTE_TICKS:
                    raise CheckError(
                        f"{label} route ticks {result['route_ticks']} != {EXPECTED_ROUTE_TICKS}"
                    )
                if result["pad_polls"] != EXPECTED_PAD_POLLS:
                    raise CheckError(
                        f"{label} pad polls {result['pad_polls']} != {EXPECTED_PAD_POLLS}"
                    )
                if result["display_fnv"] != EXPECTED_DISPLAY_FNV:
                    raise CheckError(
                        f"{label} display FNV {result['display_fnv']} != {EXPECTED_DISPLAY_FNV}"
                    )

            for key, label in (
                ("route", "route log"),
                ("cd", "CD command log"),
                ("audio", "audio"),
                ("display", "display"),
            ):
                same(first[key], second[key], label)

            commands, highest_read_lba = cd_evidence(first["cd"], quake_lba)
            if commands != EXPECTED_CD_COMMANDS:
                raise CheckError(f"CD command count {commands} != {EXPECTED_CD_COMMANDS}")
            frames, peak, rms, rate = audio_evidence(first["audio"])
            display_sha = sha256(first["display"])
            audio_sha = sha256(first["audio"])
            if display_sha != EXPECTED_DISPLAY_SHA256:
                raise CheckError(
                    f"display SHA-256 {display_sha} != {EXPECTED_DISPLAY_SHA256}"
                )
            if audio_sha != EXPECTED_AUDIO_SHA256:
                raise CheckError(f"audio SHA-256 {audio_sha} != {EXPECTED_AUDIO_SHA256}")

            print(f"headless replays: 2 identical")
            print(f"route ticks: {EXPECTED_ROUTE_TICKS}; pad polls: {EXPECTED_PAD_POLLS}")
            print(f"display FNV-1a-64: {EXPECTED_DISPLAY_FNV}")
            print(f"display SHA-256: {display_sha}")
            print(f"CD commands: {commands}; highest read LBA: {highest_read_lba}")
            print(f"audio: {frames} frames at {rate} Hz; peak {peak}; RMS {rms}")
            print(f"audio SHA-256: {audio_sha}")
        return 0
    except (CheckError, FileNotFoundError, KeyError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"quake-headless-check: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
