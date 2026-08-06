#!/usr/bin/env python3
"""Split a pressed disc into the browser emulator's delivery set.

The web build boots on the data track and streams the CD-DA tracks behind
it, so the disc ships as: the data portion gzipped, each audio track's file
extent FLAC-encoded (lossless; the browser decodes back to the exact
sectors), and a manifest naming the pieces with sizes and checksums.

Every byte of the .bin lands in exactly one piece, and the script proves it
by reassembling the pieces and comparing against the original before it
will write a manifest. A delivery that cannot round-trip is a bug here,
not a support ticket later.

Usage:
  web-delivery.py <disc.cue> <disc.bin> <outdir> [TITLE ...]

Titles are per CD-DA track in disc order; missing ones fall back to
"TRACK NN". Requires the `flac` CLI.
"""

import gzip
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

SECTOR = 2352


def fnv1a32(data: bytes) -> int:
    h = 0x811C9DC5
    for b in data:
        h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
    return h


def msf_to_sector(msf: str) -> int:
    m, s, f = (int(x) for x in msf.split(":"))
    return (m * 60 + s) * 75 + f


def parse_cue(text: str):
    """Track list as (number, kind, file_start_sector): the sector inside the
    single BIN where each track's file extent begins (INDEX 00 when present,
    else INDEX 01), mirroring the emulator's cue loader."""
    tracks = []
    number = kind = index0 = index1 = None

    def push():
        if number is not None:
            start = index0 if index0 is not None else index1
            tracks.append((number, kind, start))

    for line in text.splitlines():
        line = line.strip()
        if m := re.match(r"TRACK (\d+) (\S+)", line):
            push()
            number, kind = int(m.group(1)), m.group(2)
            index0 = index1 = None
        elif m := re.match(r"INDEX 00 (\S+)", line):
            index0 = msf_to_sector(m.group(1))
        elif m := re.match(r"INDEX 01 (\S+)", line):
            index1 = msf_to_sector(m.group(1))
    push()
    return tracks


def flac_encode(raw: bytes, out: Path):
    with tempfile.NamedTemporaryFile(suffix=".cdda") as tmp:
        tmp.write(raw)
        tmp.flush()
        subprocess.run(
            ["flac", "--totally-silent", "--force-raw-format", "--endian=little",
             "--sign=signed", "--channels=2", "--bps=16", "--sample-rate=44100",
             "-8", "-f", "-o", str(out), tmp.name],
            check=True,
        )


def flac_decode(path: Path) -> bytes:
    with tempfile.NamedTemporaryFile(suffix=".cdda") as tmp:
        subprocess.run(
            ["flac", "--totally-silent", "-d", "--force-raw-format",
             "--endian=little", "--sign=signed", "-f", "-o", tmp.name, str(path)],
            check=True,
        )
        return Path(tmp.name).read_bytes()


def main():
    cue_path, bin_path, outdir = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    titles = sys.argv[4:]
    outdir.mkdir(parents=True, exist_ok=True)

    disc = bin_path.read_bytes()
    assert len(disc) % SECTOR == 0, "bin is not whole raw sectors"
    tracks = parse_cue(cue_path.read_text())
    assert tracks and tracks[0][1].startswith("MODE2"), "track 1 must be the data track"

    # File extents: each track runs to the next track's start, in sectors.
    total_sectors = len(disc) // SECTOR
    starts = [t[2] for t in tracks] + [total_sectors]

    data_raw = disc[: starts[1] * SECTOR]
    data_gz = gzip.compress(data_raw, 9)
    (outdir / "demo-data.bin.gz").write_bytes(data_gz)

    manifest = {
        "version": 1,
        "cue": "demo-disc.cue",
        "data": {
            "file": "demo-data.bin.gz",
            "gz_bytes": len(data_gz),
            "raw_bytes": len(data_raw),
            "fnv": fnv1a32(data_raw),
        },
        "tracks": [],
    }

    audio = tracks[1:]
    for i, (number, kind, _) in enumerate(audio):
        assert kind == "AUDIO", f"track {number} is {kind}, expected AUDIO"
        raw = disc[starts[1 + i] * SECTOR : starts[2 + i] * SECTOR]
        name = f"track-{number:02}.flac"
        flac_encode(raw, outdir / name)
        title = titles[i] if i < len(titles) else f"TRACK {number:02}"
        manifest["tracks"].append({
            "number": number,
            "title": title,
            "file": name,
            "flac_bytes": (outdir / name).stat().st_size,
            "raw_bytes": len(raw),
            "fnv": fnv1a32(raw),
        })

    # The round trip: decompress + decode every piece and demand the disc back.
    rebuilt = gzip.decompress(data_gz)
    for entry in manifest["tracks"]:
        rebuilt += flac_decode(outdir / entry["file"])
    assert rebuilt == disc, "reassembled delivery differs from the pressed disc"

    # Line-based twin of the JSON, for the wasm side to parse without a
    # JSON dependency: "data FILE GZ_BYTES RAW_BYTES FNV" then one
    # "track NUMBER FILE FLAC_BYTES RAW_BYTES FNV TITLE..." per track.
    lines = [
        f"data {manifest['data']['file']} {manifest['data']['gz_bytes']} "
        f"{manifest['data']['raw_bytes']} {manifest['data']['fnv']}"
    ]
    for t in manifest["tracks"]:
        lines.append(
            f"track {t['number']} {t['file']} {t['flac_bytes']} {t['raw_bytes']} "
            f"{t['fnv']} {t['title']}"
        )
    (outdir / "web-manifest.txt").write_text("\n".join(lines) + "\n")
    (outdir / "web-manifest.json").write_text(json.dumps(manifest, indent=1))
    saved = len(disc) - len(data_gz) - sum(t["flac_bytes"] for t in manifest["tracks"])
    print(f"web delivery: data {len(data_raw)//1024//1024} MiB -> {len(data_gz)//1024//1024} MiB gz, "
          f"{len(audio)} track(s) to FLAC, {saved//1024//1024} MiB saved, round trip OK")


if __name__ == "__main__":
    main()
