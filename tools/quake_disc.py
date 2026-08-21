#!/usr/bin/env python3
"""Verify and record the Quake shareware payload every disc carries."""

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
PSOXIDE_REV_FILE = Path("host/quake-build/main.rs")
PSOXIDE_REV_LINE = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+PSOXIDE_REV\b[^\n]*$", re.MULTILINE
)
PSOXIDE_REV_DECLARATION = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?const\s+PSOXIDE_REV\s*:\s*&str\s*=\s*"
    r'"([^"]*)"\s*;\s*$',
    re.MULTILINE,
)
FILE_LINE = re.compile(
    r'^\s*FILE\s+"([^"]+)"\s+BINARY\s*$', re.IGNORECASE | re.MULTILINE
)
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
QUAKE_PROVENANCE_SCHEMA = 1
GUEST_STAGE_SCHEMA = 1
SHAREWARE_PAK_SHA256 = (
    "35a9c55e5e5a284a159ad2a62e0e8def23d829561fe2f54eb402dbc0a9a946af"
)
SHAREWARE_PAK_BYTES = 18_689_235
PSOXIDE_SOURCE_KIND = "local_checkout"
PSOXIDE_SOURCE_KINDS = frozenset((PSOXIDE_SOURCE_KIND, "pinned_hydration"))
BUILD_PROFILE = "release"


class VerificationError(RuntimeError):
    """An input cannot prove the pinned Quake contract."""


@dataclass(frozen=True)
class VerifiedQuake:
    source_revision: str
    declared_psoxide_revision: str
    psoxide_revision: str
    programs_psoxide_revision: str
    provenance: Path
    provenance_sha256: str
    cue: Path
    bin: Path
    exe: Path
    cue_sha256: str
    bin_sha256: str
    exe_sha256: str
    cue_bytes: int
    bin_bytes: int
    exe_bytes: int
    psoxide_source_kind: str
    pak0_sha256: str
    pak0_bytes: int
    guest_stage_schema: int
    guest_recipe_sha256: str
    rust_toolchain_sha256: str
    rustc_version: str
    cargo_version: str
    profile: str
    features: tuple[str, ...]


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
    normal = value.strip()
    if not pattern.fullmatch(normal):
        raise VerificationError(f"{label} must be a full lowercase hexadecimal value")
    return normal


def require_object(value: object, label: str) -> dict[str, object]:
    if not isinstance(value, dict):
        raise VerificationError(f"{label} must be a JSON object")
    return value


def require_string(value: object, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise VerificationError(f"{label} must be a non-empty string")
    return value


def require_integer(value: object, label: str) -> int:
    if type(value) is not int:
        raise VerificationError(f"{label} must be an integer")
    return value


def member(container: dict[str, object], key: str, label: str) -> object:
    try:
        return container[key]
    except KeyError as error:
        raise VerificationError(f"{label} is missing required field {key!r}") from error


def artifact_file(value: object, label: str) -> str:
    name = require_string(value, f"{label}.file")
    path = Path(name)
    if path.is_absolute() or len(path.parts) != 1 or name in {".", ".."}:
        raise VerificationError(f"{label}.file must be one basename")
    return name


def json_without_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise VerificationError(f"provenance JSON contains duplicate key {key!r}")
        result[key] = value
    return result


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


def verify_clean_checkout(
    source: Path, expected_revision: str, label: str
) -> tuple[Path, str]:
    try:
        source = source.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"{label} checkout does not exist: {source}") from error
    if not source.is_dir():
        raise VerificationError(f"{label} checkout is not a directory: {source}")
    top = Path(git(source, "rev-parse", "--show-toplevel")).resolve()
    if top != source:
        raise VerificationError(
            f"{label} checkout must name the repository root: {source} != {top}"
        )
    revision = git(source, "rev-parse", "--verify", "HEAD^{commit}").lower()
    if revision != expected_revision:
        raise VerificationError(
            f"{label} checkout revision mismatch: expected {expected_revision}, got {revision}"
        )
    dirty = git(source, "status", "--porcelain=v1", "--untracked-files=normal")
    if dirty:
        raise VerificationError(
            f"{label} checkout is dirty; exact revision provenance is false"
        )
    return source, revision


def declared_psoxide_revision(source: Path) -> str:
    declaration_path = source / PSOXIDE_REV_FILE
    try:
        text = declaration_path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise VerificationError(
            f"cannot read Quake PSOXIDE_REV declaration {declaration_path}: {error}"
        ) from error
    lines = PSOXIDE_REV_LINE.findall(text)
    if len(lines) != 1:
        raise VerificationError(
            f"{declaration_path}: expected exactly one PSOXIDE_REV const declaration, "
            f"found {len(lines)}"
        )
    match = PSOXIDE_REV_DECLARATION.fullmatch(lines[0])
    if match is None:
        raise VerificationError(
            f"{declaration_path}: malformed PSOXIDE_REV const declaration"
        )
    return require_hex(match.group(1), FULL_REVISION, "Quake PSOXIDE_REV")


def verify_programs_revision_stamp(stamp: Path, expected_revision: str) -> str:
    try:
        text = stamp.read_text(encoding="ascii")
    except FileNotFoundError as error:
        raise VerificationError(
            f"ordinary-program SDK revision stamp does not exist: {stamp}; "
            "run 'make disc'"
        ) from error
    except (OSError, UnicodeDecodeError) as error:
        raise VerificationError(
            f"cannot read ordinary-program SDK revision stamp {stamp}: {error}"
        ) from error
    lines = text.splitlines()
    if len(lines) != 1 or text != lines[0] + "\n":
        raise VerificationError(
            f"ordinary-program SDK revision stamp is malformed: {stamp}"
        )
    revision = require_hex(
        lines[0], FULL_REVISION, "ordinary-program SDK revision stamp"
    )
    if revision != expected_revision:
        raise VerificationError(
            f"ordinary-program SDK revision mismatch: stamp has {revision}, "
            f"but PSoXide checkout is {expected_revision}; run 'make disc'"
        )
    return revision


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
    if (
        relative.is_absolute()
        or len(relative.parts) != 1
        or relative.name in {".", ".."}
    ):
        raise VerificationError(f"{cue}: FILE must name one bin beside the cue")
    bin_path = cue.parent / relative
    try:
        resolved_bin = bin_path.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"cue bin does not exist: {bin_path}") from error
    if resolved_bin.parent != cue.parent or not resolved_bin.is_file():
        raise VerificationError(
            f"{cue}: FILE must resolve to a regular bin beside the cue"
        )

    tracks = [(number, mode.upper()) for number, mode in TRACK_LINE.findall(text)]
    if not tracks or tracks[0] != ("01", "MODE2/2352"):
        raise VerificationError(f"{cue}: first track must be TRACK 01 MODE2/2352")
    if data_only and tracks != [("01", "MODE2/2352")]:
        raise VerificationError(
            f"{cue}: pinned Quake input must contain one data track only"
        )
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
        raise VerificationError(
            f"{demo_bin}: no {TOC_MAGIC.decode()} table at LBA {TOC_LBA}"
        )
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
        description = text_field(
            entry[description_at : description_at + TOC_DESC_BYTES]
        )
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
    if "shareware" not in entry.description.lower():
        raise VerificationError(
            f"{demo_bin}: Quake description does not identify the shareware release"
        )
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
            if expected[:12] != actual[:12] or expected[15:] != actual[15:]:
                raise VerificationError(
                    f"{demo_bin}: embedded Quake sector {sector} differs outside relocated MSF bytes"
                )
    return sectors


def verify_artifact(
    artifacts: dict[str, object],
    key: str,
    artifact_dir: Path,
    expected_path: Path | None,
    expected_sha256: str,
) -> tuple[Path, str, int]:
    label = f"provenance artifacts.{key}"
    record = require_object(member(artifacts, key, "provenance artifacts"), label)
    filename = artifact_file(member(record, "file", label), label)
    candidate = artifact_dir / filename
    try:
        path = candidate.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"{label} does not exist: {candidate}") from error
    if path.parent != artifact_dir or not path.is_file():
        raise VerificationError(f"{label}.file must resolve beside the Quake cue")
    if expected_path is not None and path != expected_path:
        raise VerificationError(
            f"{label}.file names {path.name!r}, expected {expected_path.name!r}"
        )

    recorded_sha256 = require_hex(
        require_string(member(record, "sha256", label), f"{label}.sha256"),
        SHA256,
        f"{label}.sha256",
    )
    recorded_bytes = require_integer(member(record, "bytes", label), f"{label}.bytes")
    actual_bytes = path.stat().st_size
    if recorded_bytes != actual_bytes:
        raise VerificationError(
            f"{label} byte-size mismatch: sidecar has {recorded_bytes}, actual is {actual_bytes}"
        )
    actual_sha256 = sha256(path)
    if recorded_sha256 != actual_sha256:
        raise VerificationError(
            f"{label} SHA-256 mismatch: sidecar has {recorded_sha256}, actual is {actual_sha256}"
        )
    if actual_sha256 != expected_sha256:
        raise VerificationError(
            f"Quake {key} SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
        )
    return path, actual_sha256, actual_bytes


def verify_provenance(
    provenance: Path,
    cue: Path,
    bin_path: Path,
    source_revision: str,
    psoxide_revision: str,
    expected_provenance_sha256: str,
    expected_cue_sha256: str,
    expected_bin_sha256: str,
    expected_exe_sha256: str,
) -> dict[str, object]:
    try:
        resolved = provenance.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(
            f"Quake provenance sidecar does not exist: {provenance}"
        ) from error
    if not resolved.is_file():
        raise VerificationError(f"Quake provenance sidecar is not a file: {resolved}")
    expected_sidecar = cue.with_suffix(".provenance.json")
    if resolved != expected_sidecar:
        raise VerificationError(
            "Quake provenance sidecar must be beside the cue and named "
            f"{expected_sidecar.name!r}"
        )
    try:
        document = json.loads(
            resolved.read_text(encoding="ascii"),
            object_pairs_hook=json_without_duplicate_keys,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise VerificationError(
            f"cannot parse Quake provenance sidecar {resolved}: {error}"
        ) from error
    root = require_object(document, "Quake provenance")
    schema = require_integer(
        member(root, "schema", "Quake provenance"), "provenance schema"
    )
    if schema != QUAKE_PROVENANCE_SCHEMA:
        raise VerificationError(
            f"unsupported Quake provenance schema {schema}; expected {QUAKE_PROVENANCE_SCHEMA}"
        )

    quake_source = require_object(
        member(root, "quake_source", "Quake provenance"), "provenance quake_source"
    )
    recorded_quake_revision = require_hex(
        require_string(
            member(quake_source, "revision", "provenance quake_source"),
            "provenance quake_source.revision",
        ),
        FULL_REVISION,
        "provenance Quake revision",
    )
    if recorded_quake_revision != source_revision:
        raise VerificationError(
            "provenance Quake revision mismatch: "
            f"expected {source_revision}, got {recorded_quake_revision}"
        )
    if member(quake_source, "tree_clean", "provenance quake_source") is not True:
        raise VerificationError("provenance must record a clean Quake source tree")

    psoxide = require_object(
        member(root, "psoxide", "Quake provenance"), "provenance psoxide"
    )
    recorded_psoxide_revision = require_hex(
        require_string(
            member(psoxide, "revision", "provenance psoxide"),
            "provenance psoxide.revision",
        ),
        FULL_REVISION,
        "provenance PSoXide revision",
    )
    if recorded_psoxide_revision != psoxide_revision:
        raise VerificationError(
            "provenance PSoXide revision mismatch: "
            f"expected {psoxide_revision}, got {recorded_psoxide_revision}"
        )
    if member(psoxide, "tree_clean", "provenance psoxide") is not True:
        raise VerificationError("provenance must record a clean PSoXide source tree")
    source_kind = require_string(
        member(psoxide, "source_kind", "provenance psoxide"),
        "provenance psoxide.source_kind",
    )
    if source_kind not in PSOXIDE_SOURCE_KINDS:
        raise VerificationError(
            "provenance PSoXide source kind must describe a clean reproducible "
            f"checkout ({', '.join(sorted(PSOXIDE_SOURCE_KINDS))}), got {source_kind!r}"
        )

    shareware = require_object(
        member(root, "shareware", "Quake provenance"), "provenance shareware"
    )
    pak0_sha256 = require_hex(
        require_string(
            member(shareware, "pak0_sha256", "provenance shareware"),
            "provenance shareware.pak0_sha256",
        ),
        SHA256,
        "provenance shareware PAK0 SHA-256",
    )
    pak0_bytes = require_integer(
        member(shareware, "pak0_bytes", "provenance shareware"),
        "provenance shareware.pak0_bytes",
    )
    if pak0_sha256 != SHAREWARE_PAK_SHA256 or pak0_bytes != SHAREWARE_PAK_BYTES:
        raise VerificationError(
            "provenance does not identify the canonical Quake 1.06 shareware PAK0"
        )

    build = require_object(
        member(root, "build", "Quake provenance"), "provenance build"
    )
    guest_stage_schema = require_integer(
        member(build, "guest_stage_schema", "provenance build"),
        "provenance build.guest_stage_schema",
    )
    if guest_stage_schema != GUEST_STAGE_SCHEMA:
        raise VerificationError(
            f"unsupported guest-stage schema {guest_stage_schema}; expected {GUEST_STAGE_SCHEMA}"
        )
    guest_recipe_sha256 = require_hex(
        require_string(
            member(build, "guest_recipe_sha256", "provenance build"),
            "provenance build.guest_recipe_sha256",
        ),
        SHA256,
        "provenance guest recipe SHA-256",
    )
    rust_toolchain_sha256 = require_hex(
        require_string(
            member(build, "rust_toolchain_sha256", "provenance build"),
            "provenance build.rust_toolchain_sha256",
        ),
        SHA256,
        "provenance Rust toolchain SHA-256",
    )
    rustc_version = require_string(
        member(build, "rustc_version", "provenance build"),
        "provenance build.rustc_version",
    )
    cargo_version = require_string(
        member(build, "cargo_version", "provenance build"),
        "provenance build.cargo_version",
    )
    if not rustc_version.startswith("rustc ") or not cargo_version.startswith("cargo "):
        raise VerificationError(
            "provenance build must contain verbose rustc and cargo identities"
        )
    profile = require_string(
        member(build, "profile", "provenance build"), "provenance build.profile"
    )
    features_value = member(build, "features", "provenance build")
    if not isinstance(features_value, list) or any(
        not isinstance(feature, str) for feature in features_value
    ):
        raise VerificationError("provenance build.features must be a string array")
    features = tuple(features_value)
    if profile != BUILD_PROFILE or features:
        raise VerificationError(
            "shipping provenance must record release profile with no Cargo features"
        )

    artifacts = require_object(
        member(root, "artifacts", "Quake provenance"), "provenance artifacts"
    )
    artifact_dir = cue.parent.resolve()
    cue_path, cue_sha256, cue_bytes = verify_artifact(
        artifacts, "cue", artifact_dir, cue, expected_cue_sha256
    )
    verified_bin, bin_sha256, bin_bytes = verify_artifact(
        artifacts, "bin", artifact_dir, bin_path, expected_bin_sha256
    )
    exe, exe_sha256, exe_bytes = verify_artifact(
        artifacts, "exe", artifact_dir, cue.with_suffix(".exe"), expected_exe_sha256
    )
    provenance_sha256 = sha256(resolved)
    if provenance_sha256 != expected_provenance_sha256:
        raise VerificationError(
            "Quake provenance SHA-256 mismatch: "
            f"expected {expected_provenance_sha256}, got {provenance_sha256}"
        )
    return {
        "path": resolved,
        "sha256": provenance_sha256,
        "cue": cue_path,
        "bin": verified_bin,
        "exe": exe,
        "cue_sha256": cue_sha256,
        "bin_sha256": bin_sha256,
        "exe_sha256": exe_sha256,
        "cue_bytes": cue_bytes,
        "bin_bytes": bin_bytes,
        "exe_bytes": exe_bytes,
        "source_kind": source_kind,
        "pak0_sha256": pak0_sha256,
        "pak0_bytes": pak0_bytes,
        "guest_stage_schema": guest_stage_schema,
        "guest_recipe_sha256": guest_recipe_sha256,
        "rust_toolchain_sha256": rust_toolchain_sha256,
        "rustc_version": rustc_version,
        "cargo_version": cargo_version,
        "profile": profile,
        "features": features,
    }


def verify_quake(
    source: Path,
    psoxide: Path,
    programs_psoxide_stamp: Path,
    cue: Path,
    provenance: Path,
    expected_revision: str,
    expected_psoxide_revision: str,
    expected_provenance_sha256: str,
    expected_cue_sha256: str,
    expected_bin_sha256: str,
    expected_exe_sha256: str,
    programs_psoxide: Path | None = None,
    expected_programs_psoxide_revision: str | None = None,
) -> VerifiedQuake:
    expected_revision = require_hex(
        expected_revision, FULL_REVISION, "expected revision"
    )
    expected_psoxide_revision = require_hex(
        expected_psoxide_revision, FULL_REVISION, "expected PSoXide revision"
    )
    if programs_psoxide is None:
        programs_psoxide = psoxide
    if expected_programs_psoxide_revision is None:
        expected_programs_psoxide_revision = expected_psoxide_revision
    expected_programs_psoxide_revision = require_hex(
        expected_programs_psoxide_revision,
        FULL_REVISION,
        "expected ordinary-program PSoXide revision",
    )
    expected_provenance_sha256 = require_hex(
        expected_provenance_sha256, SHA256, "expected provenance SHA-256"
    )
    expected_cue_sha256 = require_hex(
        expected_cue_sha256, SHA256, "expected cue SHA-256"
    )
    expected_bin_sha256 = require_hex(
        expected_bin_sha256, SHA256, "expected bin SHA-256"
    )
    expected_exe_sha256 = require_hex(
        expected_exe_sha256, SHA256, "expected exe SHA-256"
    )

    source, revision = verify_clean_checkout(source, expected_revision, "Quake source")
    declared_revision = declared_psoxide_revision(source)
    _, psoxide_revision = verify_clean_checkout(
        psoxide, expected_psoxide_revision, "PSoXide"
    )
    if declared_revision != psoxide_revision:
        raise VerificationError(
            f"Quake PSOXIDE_REV mismatch: source declares {declared_revision}, "
            f"but disc checkout is {psoxide_revision}"
        )
    _, programs_psoxide_revision = verify_clean_checkout(
        programs_psoxide,
        expected_programs_psoxide_revision,
        "ordinary-program PSoXide",
    )
    programs_psoxide_revision = verify_programs_revision_stamp(
        programs_psoxide_stamp, programs_psoxide_revision
    )

    try:
        resolved_cue = cue.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"cue does not exist: {cue}") from error
    resolved_bin = cue_bin(resolved_cue, data_only=True)
    bin_bytes = resolved_bin.stat().st_size
    if bin_bytes == 0 or bin_bytes % SECTOR_BYTES != 0:
        raise VerificationError(
            f"Quake bin is {bin_bytes} bytes, not a non-empty whole number of {SECTOR_BYTES}-byte sectors"
        )
    boot_at = BOOT_EXE_LBA * SECTOR_BYTES + 24
    with resolved_bin.open("rb") as stream:
        stream.seek(boot_at)
        magic = stream.read(len(PSX_EXE_MAGIC))
    if magic != PSX_EXE_MAGIC:
        raise VerificationError(
            f"Quake bin has no PS-X EXE at the mkdisc boot LBA {BOOT_EXE_LBA}"
        )

    verified_provenance = verify_provenance(
        provenance,
        resolved_cue,
        resolved_bin,
        revision,
        psoxide_revision,
        expected_provenance_sha256,
        expected_cue_sha256,
        expected_bin_sha256,
        expected_exe_sha256,
    )
    return VerifiedQuake(
        source_revision=revision,
        declared_psoxide_revision=declared_revision,
        psoxide_revision=psoxide_revision,
        programs_psoxide_revision=programs_psoxide_revision,
        provenance=verified_provenance["path"],
        provenance_sha256=verified_provenance["sha256"],
        cue=verified_provenance["cue"],
        bin=verified_provenance["bin"],
        exe=verified_provenance["exe"],
        cue_sha256=verified_provenance["cue_sha256"],
        bin_sha256=verified_provenance["bin_sha256"],
        exe_sha256=verified_provenance["exe_sha256"],
        cue_bytes=verified_provenance["cue_bytes"],
        bin_bytes=verified_provenance["bin_bytes"],
        exe_bytes=verified_provenance["exe_bytes"],
        psoxide_source_kind=verified_provenance["source_kind"],
        pak0_sha256=verified_provenance["pak0_sha256"],
        pak0_bytes=verified_provenance["pak0_bytes"],
        guest_stage_schema=verified_provenance["guest_stage_schema"],
        guest_recipe_sha256=verified_provenance["guest_recipe_sha256"],
        rust_toolchain_sha256=verified_provenance["rust_toolchain_sha256"],
        rustc_version=verified_provenance["rustc_version"],
        cargo_version=verified_provenance["cargo_version"],
        profile=verified_provenance["profile"],
        features=verified_provenance["features"],
    )


def verify_from_args(args: argparse.Namespace) -> VerifiedQuake:
    return verify_quake(
        Path(args.source),
        Path(args.psoxide),
        Path(args.programs_psoxide_stamp),
        Path(args.cue),
        Path(args.provenance),
        args.expected_revision,
        args.expected_psoxide_revision,
        args.expected_provenance_sha256,
        args.expected_cue_sha256,
        args.expected_bin_sha256,
        args.expected_exe_sha256,
        programs_psoxide=(
            Path(args.programs_psoxide) if args.programs_psoxide else None
        ),
        expected_programs_psoxide_revision=(
            args.expected_programs_psoxide_revision
            if args.expected_programs_psoxide_revision
            else None
        ),
    )


def print_verification(verified: VerifiedQuake) -> None:
    print(f"quake source revision: {verified.source_revision}")
    print(f"quake declared PSoXide revision: {verified.declared_psoxide_revision}")
    print(f"disc PSoXide revision: {verified.psoxide_revision}")
    print(f"ordinary-program PSoXide revision: {verified.programs_psoxide_revision}")
    print(f"quake provenance SHA-256: {verified.provenance_sha256}")
    print(f"quake guest recipe SHA-256: {verified.guest_recipe_sha256}")
    print(f"quake cue SHA-256: {verified.cue_sha256}")
    print(f"quake bin SHA-256: {verified.bin_sha256}")
    print(f"quake bin bytes: {verified.bin_bytes}")
    print(f"quake exe SHA-256: {verified.exe_sha256}")
    print(f"quake exe bytes: {verified.exe_bytes}")


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
    embedded_sectors = verify_embedded_image(
        demo_bin, verified.bin, toc_entry.lba_offset
    )

    receipt = {
        "schema": 3,
        "variant": "quake-shareware-default",
        "redistribution": REDISTRIBUTION_GATE,
        "quake_input": {
            "source_revision": verified.source_revision,
            "source_tree_clean": True,
            "declared_psoxide_revision": verified.declared_psoxide_revision,
            "provenance_file": verified.provenance.name,
            "provenance_sha256": verified.provenance_sha256,
            "cue_file": verified.cue.name,
            "cue_sha256": verified.cue_sha256,
            "cue_bytes": verified.cue_bytes,
            "bin_file": verified.bin.name,
            "bin_sha256": verified.bin_sha256,
            "bin_bytes": verified.bin_bytes,
            "exe_file": verified.exe.name,
            "exe_sha256": verified.exe_sha256,
            "exe_bytes": verified.exe_bytes,
        },
        "psoxide_input": {
            "revision": verified.psoxide_revision,
            "tree_clean": True,
            "matches_quake_declared_revision": True,
            "ordinary_programs_revision": verified.programs_psoxide_revision,
            "ordinary_programs_match_checkout": True,
            "ordinary_programs_match_quake_sdk": (
                verified.programs_psoxide_revision == verified.psoxide_revision
            ),
        },
        "quake_artifact_sdk_provenance": {
            "status": "sidecar-bound",
            "schema": QUAKE_PROVENANCE_SCHEMA,
            "psoxide_source_kind": verified.psoxide_source_kind,
            "shareware": {
                "pak0_sha256": verified.pak0_sha256,
                "pak0_bytes": verified.pak0_bytes,
            },
            "build": {
                "guest_stage_schema": verified.guest_stage_schema,
                "guest_recipe_sha256": verified.guest_recipe_sha256,
                "rust_toolchain_sha256": verified.rust_toolchain_sha256,
                "rustc_version": verified.rustc_version,
                "cargo_version": verified.cargo_version,
                "profile": verified.profile,
                "features": list(verified.features),
            },
            "proved": (
                "the clean Quake and PSoXide revisions, canonical shareware PAK, "
                "guest build recipe, toolchain identities, and actual cue/bin/exe "
                "bytes match the shipping sidecar"
            ),
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
    temporary.write_text(
        json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="ascii"
    )
    temporary.replace(out)
    return out


def print_repin(args: argparse.Namespace) -> None:
    """Print what a built Quake tree implies, as the lines to paste into the Makefile.

    Deliberately writes nothing. The pins are edited by hand so the diff shows
    which contract moved, and a repin that silently rewrote them would be a
    verifier that agrees with whatever it is given.
    """
    source = Path(args.source)
    try:
        source = source.resolve(strict=True)
    except FileNotFoundError as error:
        raise VerificationError(f"Quake source does not exist: {source}") from error
    revision = git(source, "rev-parse", "--verify", "HEAD^{commit}").lower()
    dirty = git(source, "status", "--porcelain=v1", "--untracked-files=normal")
    declared = declared_psoxide_revision(source)
    cue = Path(args.cue).resolve(strict=True)
    bin_path = cue_bin(cue, data_only=True)
    exe = cue.with_suffix(".exe")
    provenance = Path(args.provenance).resolve(strict=True)

    print(f"# measured from {source}")
    print(f"QUAKE_EXPECTED_REV ?= {revision}")
    print(f"QUAKE_EXPECTED_PSOXIDE_REV ?= {declared}")
    print(f"QUAKE_EXPECTED_PROVENANCE_SHA256 ?= {sha256(provenance)}")
    print(f"QUAKE_EXPECTED_CUE_SHA256 ?= {sha256(cue)}")
    print(f"QUAKE_EXPECTED_BIN_SHA256 ?= {sha256(bin_path)}")
    print(f"QUAKE_EXPECTED_EXE_SHA256 ?= {sha256(exe)}")
    print()
    print("# then, in this order:")
    print(f"#   git -C games/PSoXide checkout {declared} && git add games/PSoXide")
    print("#   make disc")
    print("#   make quake-headless-check   (recompute the two FNV pins it prints)")
    if dirty:
        print()
        print("# WARNING: the Quake tree is dirty, so these values name no revision.")
        print("# Commit or clean it and measure again; quake-verify will reject them.")


def add_verification_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--source", required=True)
    parser.add_argument("--psoxide", required=True)
    parser.add_argument("--programs-psoxide")
    parser.add_argument("--programs-psoxide-stamp", required=True)
    parser.add_argument("--cue", required=True)
    parser.add_argument("--provenance", required=True)
    parser.add_argument("--expected-revision", required=True)
    parser.add_argument("--expected-psoxide-revision", required=True)
    parser.add_argument("--expected-programs-psoxide-revision")
    parser.add_argument("--expected-provenance-sha256", required=True)
    parser.add_argument("--expected-cue-sha256", required=True)
    parser.add_argument("--expected-bin-sha256", required=True)
    parser.add_argument("--expected-exe-sha256", required=True)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    verify = commands.add_parser(
        "verify", help="verify the pinned Quake source and image"
    )
    add_verification_args(verify)
    receipt = commands.add_parser(
        "receipt", help="verify again and hash the combined output"
    )
    add_verification_args(receipt)
    receipt.add_argument("--demo-cue", required=True)
    receipt.add_argument("--demo-bin", required=True)
    receipt.add_argument("--out", required=True)
    repin = commands.add_parser(
        "repin", help="print the pin values a built Quake tree implies"
    )
    repin.add_argument("--source", required=True)
    repin.add_argument("--cue", required=True)
    repin.add_argument("--provenance", required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.command == "repin":
            print_repin(args)
            return 0
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
