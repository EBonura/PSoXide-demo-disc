#!/usr/bin/env python3
"""Verify and record the opt-in Quake shareware demo-disc input."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


SECTOR_BYTES = 2352
USER_DATA_AT = 24
USER_DATA_BYTES = 2048
BOOT_EXE_LBA = 22
PSX_EXE_MAGIC = b"PS-X EXE"
TOC_LBA = 22
TOC_SECTORS = 4
TOC_MAGIC = b"PSXDEMO4"
TOC_HEADER_BYTES = 0x16C
TOC_ENTRY_BYTES = 512
TOC_MAX_ENTRIES = (TOC_SECTORS * USER_DATA_BYTES - TOC_HEADER_BYTES) // TOC_ENTRY_BYTES
TOC_NAME_BYTES = 24
TOC_DESC_BYTES = 224
TOC_VERSION_BYTES = 16
FULL_REVISION = re.compile(r"[0-9a-f]{40}\Z")
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
FILE_LINE = re.compile(r'^\s*FILE\s+"([^"]+)"\s+BINARY\s*$', re.IGNORECASE | re.MULTILINE)
TRACK_LINE = re.compile(
    r"^\s*TRACK\s+([0-9]{2})\s+([^\s]+)\s*$", re.IGNORECASE | re.MULTILINE
)
INDEX_LINE = re.compile(
    r"^\s*INDEX\s+([0-9]{2})\s+([0-9]{2}:[0-9]{2}:[0-9]{2})\s*$",
    re.IGNORECASE | re.MULTILINE,
)
REDISTRIBUTION_GATE = (
    "blocked pending separate Quake shareware legal and release approval"
)


class VerificationError(RuntimeError):
    """An input cannot prove the pinned local/test Quake contract."""


@dataclass(frozen=True)
class VerifiedQuake:
    source_revision: str
    cue: Path
    bin: Path
    cue_sha256: str
    bin_sha256: str
    bin_bytes: int


@dataclass(frozen=True)
class QuakeTocEntry:
    exe_lba: int
    lba_offset: int
    cdda_track_base: int
    payload_fnv: int
    version: str
    description: str


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def require_hex(value: str, pattern: re.Pattern[str], label: str) -> str:
    normal = value.strip().lower()
    if not pattern.fullmatch(normal):
        raise VerificationError(f"{label} must be a full lowercase hexadecimal value")
    return normal


def git(source: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(source), *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        raise VerificationError(f"git {' '.join(args)} failed for {source}: {detail}")
    return result.stdout.strip()


def cue_bin(cue: Path, *, data_only: bool) -> Path:
    try:
        cue = cue.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"cue does not exist: {cue}") from error
    if not cue.is_file():
        raise VerificationError(f"cue is not a regular file: {cue}")
    try:
        text = cue.read_text(encoding="ascii")
    except (OSError, UnicodeDecodeError) as error:
        raise VerificationError(f"cannot read ASCII cue {cue}: {error}") from error

    files = FILE_LINE.findall(text)
    if len(files) != 1:
        raise VerificationError(
            f"{cue}: expected exactly one quoted FILE ... BINARY line, found {len(files)}"
        )
    relative = Path(files[0])
    if relative.is_absolute() or len(relative.parts) != 1 or relative.name in {".", ".."}:
        raise VerificationError(f"{cue}: FILE must name one bin beside the cue")
    bin_path = cue.parent / relative
    try:
        resolved_bin = bin_path.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"cue bin does not exist: {bin_path}") from error
    if resolved_bin.parent != cue.parent or not resolved_bin.is_file():
        raise VerificationError(f"{cue}: FILE must resolve to a regular bin beside the cue")

    tracks = [(number, mode.upper()) for number, mode in TRACK_LINE.findall(text)]
    if not tracks or tracks[0] != ("01", "MODE2/2352"):
        raise VerificationError(f"{cue}: first track must be TRACK 01 MODE2/2352")
    if data_only and tracks != [("01", "MODE2/2352")]:
        raise VerificationError(f"{cue}: pinned Quake input must contain one data track only")
    indices = [(number, time) for number, time in INDEX_LINE.findall(text)]
    if data_only and indices != [("01", "00:00:00")]:
        raise VerificationError(
            f"{cue}: pinned Quake input must start at INDEX 01 00:00:00"
        )
    return resolved_bin


def text_field(data: bytes) -> str:
    return data.split(b"\0", 1)[0].decode("ascii", errors="strict")


def read_user_sectors(image: Path, lba: int, count: int) -> bytes:
    chunks = bytearray()
    with image.open("rb") as stream:
        for sector in range(lba, lba + count):
            stream.seek(sector * SECTOR_BYTES + USER_DATA_AT)
            chunk = stream.read(USER_DATA_BYTES)
            if len(chunk) != USER_DATA_BYTES:
                raise VerificationError(
                    f"{image}: ended while reading {count} user-data sectors at LBA {lba}"
                )
            chunks.extend(chunk)
    return bytes(chunks)


def quake_toc_entry(demo_bin: Path, expected_revision: str) -> QuakeTocEntry:
    toc = read_user_sectors(demo_bin, TOC_LBA, TOC_SECTORS)
    if toc[:8] != TOC_MAGIC:
        raise VerificationError(f"{demo_bin}: no {TOC_MAGIC.decode()} table at LBA {TOC_LBA}")
    count = int.from_bytes(toc[8:12], "little")
    if count == 0 or count > TOC_MAX_ENTRIES:
        raise VerificationError(f"{demo_bin}: invalid demo table entry count {count}")

    matches: list[QuakeTocEntry] = []
    for index in range(count):
        at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES
        entry = toc[at : at + TOC_ENTRY_BYTES]
        name = text_field(entry[:TOC_NAME_BYTES])
        if name != "QUAKE SHAREWARE":
            continue
        numbers = TOC_NAME_BYTES
        exe_lba = int.from_bytes(entry[numbers : numbers + 4], "little")
        lba_offset = int.from_bytes(entry[numbers + 4 : numbers + 8], "little")
        cdda_track_base = int.from_bytes(entry[numbers + 8 : numbers + 12], "little")
        payload_fnv = int.from_bytes(entry[numbers + 12 : numbers + 16], "little")
        description_at = numbers + 16
        description = text_field(entry[description_at : description_at + TOC_DESC_BYTES])
        version_at = description_at + 2 * TOC_DESC_BYTES
        version = text_field(entry[version_at : version_at + TOC_VERSION_BYTES])
        matches.append(
            QuakeTocEntry(
                exe_lba=exe_lba,
                lba_offset=lba_offset,
                cdda_track_base=cdda_track_base,
                payload_fnv=payload_fnv,
                version=version,
                description=description,
            )
        )
    if len(matches) != 1:
        raise VerificationError(
            f"{demo_bin}: expected exactly one QUAKE SHAREWARE table entry, found {len(matches)}"
        )
    entry = matches[0]
    if entry.lba_offset < TOC_LBA + TOC_SECTORS:
        raise VerificationError(f"{demo_bin}: Quake image overlaps the demo ISO")
    if entry.exe_lba != entry.lba_offset + BOOT_EXE_LBA:
        raise VerificationError(
            f"{demo_bin}: Quake EXE LBA {entry.exe_lba} does not match image offset "
            f"{entry.lba_offset} + boot LBA {BOOT_EXE_LBA}"
        )
    expected_version = "q" + expected_revision[:7]
    if entry.version != expected_version:
        raise VerificationError(
            f"{demo_bin}: Quake menu version is {entry.version!r}, expected {expected_version!r}"
        )
    if entry.payload_fnv == 0:
        raise VerificationError(f"{demo_bin}: Quake loader payload checksum is zero")
    if "local test" not in entry.description.lower():
        raise VerificationError(f"{demo_bin}: Quake description does not identify a local test build")
    return entry


def verify_embedded_image(demo_bin: Path, quake_bin: Path, lba_offset: int) -> int:
    sectors = quake_bin.stat().st_size // SECTOR_BYTES
    with quake_bin.open("rb") as source, demo_bin.open("rb") as combined:
        combined.seek(lba_offset * SECTOR_BYTES)
        for sector in range(sectors):
            expected = source.read(SECTOR_BYTES)
            actual = combined.read(SECTOR_BYTES)
            if len(actual) != SECTOR_BYTES:
                raise VerificationError(
                    f"{demo_bin}: embedded Quake image ends at sector {sector} of {sectors}"
                )
            if expected[:12] != actual[:12] or expected[16:] != actual[16:]:
                raise VerificationError(
                    f"{demo_bin}: embedded Quake sector {sector} differs outside relocated MSF bytes"
                )
    return sectors


def verify_quake(
    source: Path,
    cue: Path,
    expected_revision: str,
    expected_cue_sha256: str,
    expected_bin_sha256: str,
) -> VerifiedQuake:
    expected_revision = require_hex(expected_revision, FULL_REVISION, "expected revision")
    expected_cue_sha256 = require_hex(expected_cue_sha256, SHA256, "expected cue SHA-256")
    expected_bin_sha256 = require_hex(expected_bin_sha256, SHA256, "expected bin SHA-256")

    try:
        source = source.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"Quake source checkout does not exist: {source}") from error
    if not source.is_dir():
        raise VerificationError(f"Quake source checkout is not a directory: {source}")
    top = Path(git(source, "rev-parse", "--show-toplevel")).resolve()
    if top != source:
        raise VerificationError(f"Quake source must name the repository root: {source} != {top}")
    revision = git(source, "rev-parse", "--verify", "HEAD^{commit}").lower()
    if revision != expected_revision:
        raise VerificationError(
            f"Quake source revision mismatch: expected {expected_revision}, got {revision}"
        )
    dirty = git(source, "status", "--porcelain=v1", "--untracked-files=normal")
    if dirty:
        raise VerificationError("Quake source checkout is dirty; exact revision provenance is false")

    try:
        resolved_cue = cue.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"cue does not exist: {cue}") from error
    resolved_bin = cue_bin(resolved_cue, data_only=True)
    size = resolved_bin.stat().st_size
    if size == 0 or size % SECTOR_BYTES != 0:
        raise VerificationError(
            f"Quake bin is {size} bytes, not a non-empty whole number of {SECTOR_BYTES}-byte sectors"
        )
    boot_at = BOOT_EXE_LBA * SECTOR_BYTES + 24
    with resolved_bin.open("rb") as stream:
        stream.seek(boot_at)
        magic = stream.read(len(PSX_EXE_MAGIC))
    if magic != PSX_EXE_MAGIC:
        raise VerificationError(
            f"Quake bin has no PS-X EXE at the mkdisc boot LBA {BOOT_EXE_LBA}"
        )

    cue_hash = sha256(resolved_cue)
    bin_hash = sha256(resolved_bin)
    if cue_hash != expected_cue_sha256:
        raise VerificationError(
            f"Quake cue SHA-256 mismatch: expected {expected_cue_sha256}, got {cue_hash}"
        )
    if bin_hash != expected_bin_sha256:
        raise VerificationError(
            f"Quake bin SHA-256 mismatch: expected {expected_bin_sha256}, got {bin_hash}"
        )
    return VerifiedQuake(
        source_revision=revision,
        cue=resolved_cue,
        bin=resolved_bin,
        cue_sha256=cue_hash,
        bin_sha256=bin_hash,
        bin_bytes=size,
    )


def verify_from_args(args: argparse.Namespace) -> VerifiedQuake:
    return verify_quake(
        Path(args.source),
        Path(args.cue),
        args.expected_revision,
        args.expected_cue_sha256,
        args.expected_bin_sha256,
    )


def print_verification(verified: VerifiedQuake) -> None:
    print(f"quake source revision: {verified.source_revision}")
    print(f"quake cue SHA-256: {verified.cue_sha256}")
    print(f"quake bin SHA-256: {verified.bin_sha256}")
    print(f"quake bin bytes: {verified.bin_bytes}")


def write_receipt(args: argparse.Namespace, verified: VerifiedQuake) -> Path:
    demo_cue = Path(args.demo_cue).resolve(strict=True)
    demo_bin = Path(args.demo_bin).resolve(strict=True)
    referenced_demo_bin = cue_bin(demo_cue, data_only=False)
    if referenced_demo_bin != demo_bin:
        raise VerificationError(
            f"demo cue references {referenced_demo_bin}, not requested output {demo_bin}"
        )
    demo_size = demo_bin.stat().st_size
    if demo_size == 0 or demo_size % SECTOR_BYTES != 0:
        raise VerificationError(
            f"demo bin is {demo_size} bytes, not a non-empty whole number of {SECTOR_BYTES}-byte sectors"
        )
    toc_entry = quake_toc_entry(demo_bin, verified.source_revision)
    embedded_sectors = verify_embedded_image(demo_bin, verified.bin, toc_entry.lba_offset)

    receipt = {
        "schema": 1,
        "variant": "quake-shareware-local-test",
        "redistribution": REDISTRIBUTION_GATE,
        "quake_input": {
            "source_revision": verified.source_revision,
            "source_tree_clean": True,
            "cue_file": verified.cue.name,
            "cue_sha256": verified.cue_sha256,
            "bin_file": verified.bin.name,
            "bin_sha256": verified.bin_sha256,
            "bin_bytes": verified.bin_bytes,
        },
        "demo_disc_output": {
            "cue_file": demo_cue.name,
            "cue_sha256": sha256(demo_cue),
            "bin_file": demo_bin.name,
            "bin_sha256": sha256(demo_bin),
            "bin_bytes": demo_size,
            "quake_toc": {
                "exe_lba": toc_entry.exe_lba,
                "image_lba_offset": toc_entry.lba_offset,
                "cdda_track_base": toc_entry.cdda_track_base,
                "payload_fnv1a32": f"0x{toc_entry.payload_fnv:08x}",
                "menu_version": toc_entry.version,
            },
            "embedded_quake_data_sectors": embedded_sectors,
            "embedded_quake_matches_input_except_msf": True,
        },
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    temporary = out.with_name(out.name + ".tmp")
    temporary.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="ascii")
    temporary.replace(out)
    return out


def add_verification_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--source", required=True)
    parser.add_argument("--cue", required=True)
    parser.add_argument("--expected-revision", required=True)
    parser.add_argument("--expected-cue-sha256", required=True)
    parser.add_argument("--expected-bin-sha256", required=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify", help="verify the pinned Quake source and image")
    add_verification_args(verify)
    receipt = commands.add_parser("receipt", help="verify again and hash the combined output")
    add_verification_args(receipt)
    receipt.add_argument("--demo-cue", required=True)
    receipt.add_argument("--demo-bin", required=True)
    receipt.add_argument("--out", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        verified = verify_from_args(args)
        print_verification(verified)
        if args.command == "receipt":
            out = write_receipt(args, verified)
            print(f"quake provenance receipt: {out}")
        return 0
    except (OSError, VerificationError) as error:
        print(f"quake-disc: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
