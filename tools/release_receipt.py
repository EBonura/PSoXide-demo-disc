#!/usr/bin/env python3
"""Create or verify a fail-closed receipt for a combined demo-disc pressing."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


SCHEMA = "psoxide-combined-release-v1"
REQUIRED_PROGRAMS = (
    "CORTEX IGNITION",
    "CORTEX IGNITION LEGACY",
    "HALF-LIFE",
    "HARDWARE TESTS",
    "QUAKE SHAREWARE",
)
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
PSX_EXE_MAGIC = b"PS-X EXE"
BOOT_SCAN_SECTORS = 64
FILE_LINE = re.compile(r'^FILE "([^"\r\n]+)" BINARY$', re.MULTILINE)
TRACK_LINE = re.compile(r"^\s*TRACK\s+(\d+)\s+([^\s]+)\s*$", re.MULTILINE)
INDEX_LINE = re.compile(r"^\s*INDEX\s+(\d+)\s+(\d+):(\d+):(\d+)\s*$", re.MULTILINE)
REVISION = re.compile(r"^[0-9a-f]{40}$")


class ReceiptError(RuntimeError):
    pass


@dataclass(frozen=True)
class TocEntry:
    name: str
    exe_lba: int
    image_lba: int
    payload_fnv: int
    version: str
    flags: int


@dataclass(frozen=True)
class Exe:
    lba: int
    pc: int
    load: int
    payload_bytes: int
    header: bytes
    payload: bytes


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def file_record(path: Path) -> dict[str, object]:
    path = path.resolve(strict=True)
    if not path.is_file():
        raise ReceiptError(f"not a regular file: {path}")
    return {"path": str(path), "bytes": path.stat().st_size, "sha256": sha256(path)}


def split_assignment(value: str, label: str) -> tuple[str, Path]:
    if "=" not in value:
        raise ReceiptError(f"{label} must be NAME=/absolute/path: {value!r}")
    name, raw_path = value.split("=", 1)
    if not name or not raw_path:
        raise ReceiptError(f"invalid {label}: {value!r}")
    return name, Path(raw_path)


def exact_assignments(values: list[str], label: str) -> dict[str, Path]:
    decoded: dict[str, Path] = {}
    for value in values:
        name, path = split_assignment(value, label)
        if name in decoded:
            raise ReceiptError(f"duplicate {label} for {name}")
        decoded[name] = path
    missing = set(REQUIRED_PROGRAMS) - set(decoded)
    extra = set(decoded) - set(REQUIRED_PROGRAMS)
    if missing or extra:
        raise ReceiptError(
            f"{label} names must be exactly {REQUIRED_PROGRAMS}; "
            f"missing={sorted(missing)} extra={sorted(extra)}"
        )
    return decoded


def git(source: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(source), *args],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if result.returncode:
        raise ReceiptError(
            f"git {' '.join(args)} failed for {source}: {result.stderr.strip()}"
        )
    return result.stdout.strip()


def source_record(source: Path) -> dict[str, object]:
    source = source.resolve(strict=True)
    if not source.is_dir():
        raise ReceiptError(f"source is not a directory: {source}")
    top = Path(git(source, "rev-parse", "--show-toplevel")).resolve()
    if top != source:
        raise ReceiptError(f"source must name its repository root: {source} != {top}")
    revision = git(source, "rev-parse", "--verify", "HEAD^{commit}").lower()
    if REVISION.fullmatch(revision) is None:
        raise ReceiptError(f"invalid source revision for {source}: {revision!r}")
    dirty = git(source, "status", "--porcelain=v1", "--untracked-files=normal")
    if dirty:
        raise ReceiptError(f"source is dirty and cannot identify a release: {source}")
    return {
        "path": str(source),
        "kind": "clean-git-commit",
        "revision": revision,
        "tree_clean": True,
    }


def image_for_cue(cue: Path) -> Path:
    cue = cue.resolve(strict=True)
    if not cue.is_file():
        raise ReceiptError(f"cue is not a regular file: {cue}")
    try:
        names = FILE_LINE.findall(cue.read_text(encoding="ascii"))
    except UnicodeDecodeError as error:
        raise ReceiptError(f"cue is not ASCII: {cue}") from error
    unique = list(dict.fromkeys(names))
    if len(unique) != 1:
        raise ReceiptError(f"cue must reference exactly one unique BIN: {cue}")
    relative = Path(unique[0])
    if relative.is_absolute() or len(relative.parts) != 1:
        raise ReceiptError(f"cue BIN must sit beside the cue: {cue}")
    image = (cue.parent / relative).resolve(strict=True)
    if image.parent != cue.parent or not image.is_file():
        raise ReceiptError(f"cue BIN must resolve beside the cue: {cue}")
    if image.stat().st_size == 0 or image.stat().st_size % SECTOR_BYTES:
        raise ReceiptError(f"raw BIN size is not a positive whole sector: {image}")
    return image


def data_track_sectors(cue: Path, image: Path) -> int:
    text = cue.read_text(encoding="ascii")
    tracks = [(int(number), mode.upper()) for number, mode in TRACK_LINE.findall(text)]
    if not tracks or tracks[0] != (1, "MODE2/2352"):
        raise ReceiptError(f"first CUE track must be TRACK 01 MODE2/2352: {cue}")
    total = image.stat().st_size // SECTOR_BYTES
    if len(tracks) == 1:
        return total
    if tracks[1] != (2, "AUDIO"):
        raise ReceiptError(f"second CUE track must be TRACK 02 AUDIO: {cue}")
    # mkdisc embeds only the source data track. Audio tracks are re-laid-out in
    # the combined image, so the first audio pregap (INDEX 00) is the exact
    # source-sector boundary for byte comparison.
    track_two = re.search(
        r"^\s*TRACK\s+02\s+AUDIO\s*$([\s\S]*?)(?=^\s*TRACK\s+\d+|\Z)",
        text,
        re.MULTILINE,
    )
    if track_two is None:
        raise ReceiptError(f"cannot locate TRACK 02 body: {cue}")
    index_zero = re.search(
        r"^\s*INDEX\s+00\s+(\d+):(\d+):(\d+)\s*$",
        track_two.group(1),
        re.MULTILINE,
    )
    if index_zero is None:
        raise ReceiptError(f"audio-bearing CUE has no TRACK 02 INDEX 00: {cue}")
    minute, second, frame = (int(value) for value in index_zero.groups())
    if second >= 60 or frame >= 75:
        raise ReceiptError(f"invalid CUE timestamp at TRACK 02 INDEX 00: {cue}")
    sectors = (minute * 60 + second) * 75 + frame
    if sectors <= 0 or sectors > total:
        raise ReceiptError(f"invalid data-track sector count {sectors} for {cue}")
    return sectors


def read_user_sector(stream, lba: int) -> bytes:
    stream.seek(lba * SECTOR_BYTES + USER_DATA_AT)
    data = stream.read(USER_DATA_BYTES)
    if len(data) != USER_DATA_BYTES:
        raise ReceiptError(f"disc ended at LBA {lba}")
    return data


def parse_exe_at(image: Path, lba: int) -> Exe:
    with image.open("rb") as stream:
        header = read_user_sector(stream, lba)
        if header[:8] != PSX_EXE_MAGIC:
            raise ReceiptError(f"no PS-X EXE at {image} LBA {lba}")
        pc = int.from_bytes(header[0x10:0x14], "little")
        load = int.from_bytes(header[0x18:0x1C], "little")
        payload_bytes = int.from_bytes(header[0x1C:0x20], "little")
        if payload_bytes == 0 or payload_bytes % USER_DATA_BYTES:
            raise ReceiptError(f"invalid PS-X EXE payload size {payload_bytes}")
        if not load <= pc < load + payload_bytes:
            raise ReceiptError(f"PS-X EXE PC {pc:#x} lies outside its payload")
        payload = bytearray()
        for offset in range(payload_bytes // USER_DATA_BYTES):
            payload.extend(read_user_sector(stream, lba + 1 + offset))
    return Exe(lba, pc, load, payload_bytes, header, bytes(payload))


def find_boot_exe(image: Path) -> Exe:
    sectors = image.stat().st_size // SECTOR_BYTES
    with image.open("rb") as stream:
        for lba in range(min(sectors, BOOT_SCAN_SECTORS)):
            if read_user_sector(stream, lba)[:8] == PSX_EXE_MAGIC:
                return parse_exe_at(image, lba)
    raise ReceiptError(f"no boot PS-X EXE in first {BOOT_SCAN_SECTORS} sectors: {image}")


def fnv1a32(data: bytes) -> int:
    digest = 0x811C9DC5
    for byte in data:
        digest = ((digest ^ byte) * 0x01000193) & 0xFFFFFFFF
    return digest


def parse_toc(image: Path) -> dict[str, TocEntry]:
    raw = bytearray()
    with image.open("rb") as stream:
        for lba in range(TOC_LBA, TOC_LBA + TOC_SECTORS):
            raw.extend(read_user_sector(stream, lba))
    if raw[:8] != TOC_MAGIC:
        raise ReceiptError(f"missing {TOC_MAGIC!r} at combined LBA {TOC_LBA}")
    count = int.from_bytes(raw[8:12], "little")
    maximum = (len(raw) - TOC_HEADER_BYTES) // TOC_ENTRY_BYTES
    if count == 0 or count > maximum:
        raise ReceiptError(f"invalid combined TOC entry count {count}")
    entries: dict[str, TocEntry] = {}
    for index in range(count):
        at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES
        row = raw[at : at + TOC_ENTRY_BYTES]
        name = row[:TOC_NAME_BYTES].split(b"\0", 1)[0].decode("ascii")
        if name in entries:
            raise ReceiptError(f"duplicate combined TOC name: {name}")
        entries[name] = TocEntry(
            name,
            int.from_bytes(row[24:28], "little"),
            int.from_bytes(row[28:32], "little"),
            int.from_bytes(row[36:40], "little"),
            row[TOC_VERSION_AT : TOC_VERSION_AT + TOC_VERSION_BYTES]
            .split(b"\0", 1)[0]
            .decode("ascii"),
            int.from_bytes(row[TOC_FLAGS_AT : TOC_FLAGS_AT + 4], "little"),
        )
    return entries


def verify_embedded_image(
    combined: Path, source: Path, image_lba: int, sectors: int
) -> int:
    combined_sectors = combined.stat().st_size // SECTOR_BYTES
    if image_lba + sectors > combined_sectors:
        raise ReceiptError(
            f"embedded image range [{image_lba}, {image_lba + sectors}) exceeds disc"
        )
    with source.open("rb") as expected_stream, combined.open("rb") as actual_stream:
        actual_stream.seek(image_lba * SECTOR_BYTES)
        for sector in range(sectors):
            expected = expected_stream.read(SECTOR_BYTES)
            actual = actual_stream.read(SECTOR_BYTES)
            # mkdisc rewrites only absolute MSF bytes 12..14 on relocation.
            if expected[:12] != actual[:12] or expected[15:] != actual[15:]:
                raise ReceiptError(
                    f"embedded sector {sector} differs outside relocated MSF bytes"
                )
    return sectors


def program_record(cue: Path, combined: Path, entry: TocEntry) -> dict[str, object]:
    cue = cue.resolve(strict=True)
    image = image_for_cue(cue)
    sectors = data_track_sectors(cue, image)
    input_exe = find_boot_exe(image)
    verify_embedded_image(combined, image, entry.image_lba, sectors)
    if entry.exe_lba != entry.image_lba + input_exe.lba:
        raise ReceiptError(
            f"{entry.name}: TOC EXE LBA {entry.exe_lba} != image LBA "
            f"{entry.image_lba} + input boot LBA {input_exe.lba}"
        )
    embedded_exe = parse_exe_at(combined, entry.exe_lba)
    if embedded_exe.header != input_exe.header or embedded_exe.payload != input_exe.payload:
        raise ReceiptError(f"{entry.name}: embedded PS-X EXE differs from input image")
    payload_fnv = fnv1a32(input_exe.payload)
    if payload_fnv != entry.payload_fnv:
        raise ReceiptError(
            f"{entry.name}: payload FNV {payload_fnv:#010x} != TOC {entry.payload_fnv:#010x}"
        )
    exe_bytes = input_exe.header + input_exe.payload
    payload_sectors = input_exe.payload_bytes // USER_DATA_BYTES
    return {
        "input": {
            "cue": file_record(cue),
            "bin": file_record(image),
            "bin_total_sectors": image.stat().st_size // SECTOR_BYTES,
            "data_track_sectors": sectors,
            "exe": {"bytes": len(exe_bytes), "sha256": sha256_bytes(exe_bytes)},
            "payload": {
                "bytes": input_exe.payload_bytes,
                "sha256": sha256_bytes(input_exe.payload),
                "fnv1a32": f"0x{payload_fnv:08x}",
                "pc": f"0x{input_exe.pc:08x}",
                "load": f"0x{input_exe.load:08x}",
            },
        },
        "embedded": {
            "image_lba_start": entry.image_lba,
            "image_lba_end_exclusive": entry.image_lba + sectors,
            "image_sectors": sectors,
            "exe_lba": entry.exe_lba,
            "payload_lba_start": entry.exe_lba + 1,
            "payload_lba_end_exclusive": entry.exe_lba + 1 + payload_sectors,
            "version": entry.version,
            "flags": entry.flags,
        },
    }


def build_document(
    combined_cue: Path,
    frontend: Path,
    build_command: str,
    programs: dict[str, Path],
    sources: dict[str, Path],
) -> dict[str, object]:
    if not build_command or "\n" in build_command or "\r" in build_command:
        raise ReceiptError("build command must be one non-empty line")
    combined_cue = combined_cue.resolve(strict=True)
    combined = image_for_cue(combined_cue)
    entries = parse_toc(combined)
    missing = set(REQUIRED_PROGRAMS) - set(entries)
    if missing:
        raise ReceiptError(f"combined TOC is missing required programs: {sorted(missing)}")
    program_rows: dict[str, object] = {}
    ranges: list[tuple[int, int, str]] = []
    for name in REQUIRED_PROGRAMS:
        record = program_record(programs[name], combined, entries[name])
        program_rows[name] = {
            "source": source_record(sources[name]),
            **record,
        }
        embedded = record["embedded"]
        ranges.append(
            (
                embedded["image_lba_start"],
                embedded["image_lba_end_exclusive"],
                name,
            )
        )
    for previous, current in zip(sorted(ranges), sorted(ranges)[1:]):
        if current[0] < previous[1]:
            raise ReceiptError(
                f"embedded image ranges overlap: {previous[2]} and {current[2]}"
            )
    return {
        "schema": SCHEMA,
        "build_command": build_command,
        "frontend": file_record(frontend),
        "combined": {
            "cue": file_record(combined_cue),
            "bin": file_record(combined),
            "sectors": combined.stat().st_size // SECTOR_BYTES,
        },
        "programs": program_rows,
    }


def write_receipt(path: Path, document: dict[str, object]) -> None:
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(
        json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="ascii"
    )
    os.replace(temporary, path)


def verify_file_record(path: Path, expected: dict[str, object], label: str) -> None:
    if not path.is_file():
        raise ReceiptError(f"{label} is missing: {path}")
    if path.stat().st_size != expected.get("bytes"):
        raise ReceiptError(f"{label} size no longer matches the receipt: {path}")
    if sha256(path) != expected.get("sha256"):
        raise ReceiptError(f"{label} SHA-256 no longer matches the receipt: {path}")


def verify_sealed_document(receipt_path: Path, expected: dict[str, object]) -> None:
    """Verify the sealed combined image without requiring deleted build inputs.

    The ordinary verifier remains the stronger source-provenance gate. This
    mode is for a deliberately archived burn candidate: it verifies the exact
    CUE/BIN hashes and then independently checks every embedded TOC entry and
    executable payload against the receipt.
    """
    try:
        combined_row = expected["combined"]
        cue_row = combined_row["cue"]
        bin_row = combined_row["bin"]
        cue = receipt_path.parent / Path(cue_row["path"]).name
        image = receipt_path.parent / Path(bin_row["path"]).name
    except (KeyError, TypeError) as error:
        raise ReceiptError(f"malformed receipt: missing {error}") from error

    verify_file_record(cue, cue_row, "combined CUE")
    verify_file_record(image, bin_row, "combined BIN")
    if image_for_cue(cue) != image.resolve():
        raise ReceiptError("combined CUE does not reference the sealed BIN beside it")
    sectors = image.stat().st_size // SECTOR_BYTES
    if sectors != combined_row.get("sectors"):
        raise ReceiptError("combined sector count no longer matches the receipt")

    entries = parse_toc(image)
    missing = set(REQUIRED_PROGRAMS) - set(entries)
    if missing:
        raise ReceiptError(f"combined TOC is missing required programs: {sorted(missing)}")
    ranges: list[tuple[int, int, str]] = []
    for name in REQUIRED_PROGRAMS:
        try:
            row = expected["programs"][name]
            embedded = row["embedded"]
            recorded_exe = row["input"]["exe"]
            recorded_payload = row["input"]["payload"]
            source = row["source"]
        except (KeyError, TypeError) as error:
            raise ReceiptError(f"malformed receipt for {name}: missing {error}") from error
        entry = entries[name]
        exact_toc = {
            "exe_lba": entry.exe_lba,
            "image_lba_start": entry.image_lba,
            "version": entry.version,
            "flags": entry.flags,
        }
        for key, actual in exact_toc.items():
            if embedded.get(key) != actual:
                raise ReceiptError(f"{name}: sealed TOC {key} no longer matches receipt")
        start = embedded["image_lba_start"]
        end = embedded["image_lba_end_exclusive"]
        image_sectors = embedded["image_sectors"]
        if end != start + image_sectors or end > sectors:
            raise ReceiptError(f"{name}: invalid sealed image range")
        ranges.append((start, end, name))

        exe = parse_exe_at(image, entry.exe_lba)
        exe_bytes = exe.header + exe.payload
        if len(exe_bytes) != recorded_exe.get("bytes"):
            raise ReceiptError(f"{name}: embedded EXE size no longer matches receipt")
        if sha256_bytes(exe_bytes) != recorded_exe.get("sha256"):
            raise ReceiptError(f"{name}: embedded EXE SHA-256 no longer matches receipt")
        actual_payload = {
            "bytes": exe.payload_bytes,
            "sha256": sha256_bytes(exe.payload),
            "fnv1a32": f"0x{fnv1a32(exe.payload):08x}",
            "pc": f"0x{exe.pc:08x}",
            "load": f"0x{exe.load:08x}",
        }
        if actual_payload != recorded_payload:
            raise ReceiptError(f"{name}: embedded payload no longer matches receipt")
        if embedded.get("payload_lba_start") != entry.exe_lba + 1:
            raise ReceiptError(f"{name}: invalid sealed payload start")
        if embedded.get("payload_lba_end_exclusive") != (
            entry.exe_lba + 1 + exe.payload_bytes // USER_DATA_BYTES
        ):
            raise ReceiptError(f"{name}: invalid sealed payload end")
        if source.get("kind") != "clean-git-commit" or not source.get("tree_clean"):
            raise ReceiptError(f"{name}: receipt lacks clean source provenance")
        if REVISION.fullmatch(str(source.get("revision", ""))) is None:
            raise ReceiptError(f"{name}: invalid recorded source revision")

    for previous, current in zip(sorted(ranges), sorted(ranges)[1:]):
        if current[0] < previous[1]:
            raise ReceiptError(
                f"embedded image ranges overlap: {previous[2]} and {current[2]}"
            )


def create(args: argparse.Namespace) -> None:
    programs = exact_assignments(args.program, "program")
    sources = exact_assignments(args.source, "source")
    document = build_document(
        args.combined_cue, args.frontend, args.build_command, programs, sources
    )
    write_receipt(args.out, document)
    print(f"release receipt: {args.out.resolve()}")
    print(f"combined cue SHA-256: {document['combined']['cue']['sha256']}")
    print(f"combined bin SHA-256: {document['combined']['bin']['sha256']}")


def verify(args: argparse.Namespace) -> None:
    receipt_path = args.receipt.resolve(strict=True)
    expected = json.loads(receipt_path.read_text(encoding="ascii"))
    if expected.get("schema") != SCHEMA:
        raise ReceiptError(f"unsupported receipt schema: {expected.get('schema')!r}")
    if args.sealed:
        verify_sealed_document(receipt_path, expected)
        print(f"sealed release receipt verified: {receipt_path}")
        return
    try:
        programs = {
            name: Path(expected["programs"][name]["input"]["cue"]["path"])
            for name in REQUIRED_PROGRAMS
        }
        sources = {
            name: Path(expected["programs"][name]["source"]["path"])
            for name in REQUIRED_PROGRAMS
        }
        actual = build_document(
            Path(expected["combined"]["cue"]["path"]),
            Path(expected["frontend"]["path"]),
            expected["build_command"],
            programs,
            sources,
        )
    except (KeyError, TypeError) as error:
        raise ReceiptError(f"malformed receipt: missing {error}") from error
    if actual != expected:
        raise ReceiptError("receipt no longer matches its sources or artifacts")
    print(f"release receipt verified: {receipt_path}")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    create_parser = commands.add_parser("create")
    create_parser.add_argument("--combined-cue", required=True, type=Path)
    create_parser.add_argument("--frontend", required=True, type=Path)
    create_parser.add_argument("--build-command", required=True)
    create_parser.add_argument("--program", action="append", required=True)
    create_parser.add_argument("--source", action="append", required=True)
    create_parser.add_argument("--out", required=True, type=Path)
    verify_parser = commands.add_parser("verify")
    verify_parser.add_argument("--receipt", required=True, type=Path)
    verify_parser.add_argument(
        "--sealed",
        action="store_true",
        help="verify the archived CUE/BIN and embedded payloads without build inputs",
    )
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        create(args) if args.command == "create" else verify(args)
        return 0
    except (ReceiptError, FileNotFoundError, OSError, UnicodeDecodeError, ValueError) as error:
        print(f"release-receipt: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
