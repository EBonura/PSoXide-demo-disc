# Quake shareware local/test demo disc

This flow adds the validated Quake-PSX whole image to a separate PSoXide demo
disc variant. It does not alter the default public disc or the Half-Life
pressing, and it does not publish anything.

## Pinned input

The default contract is:

| Item | Pinned value |
| --- | --- |
| Quake source revision | `8d5a6815091adfa6abc597d14843fce8d4938879` |
| Input cue SHA-256 | `5fa78b12b506d4190246e230183e1eebd677f201ff982a584bff10d88ee2594c` |
| Input bin SHA-256 | `ac1109b6e172ee157e601563b34b4bd1e236ecba4131f7c4029327b1029021c4` |
| Default source checkout | sibling `quake-psx-convergence` |
| Default input cue | `dist/quake-psx.cue` in that checkout |

This is the validated integration pin available while the Episode 1 runtime
lane continues. It is not a promise that `8d5a681` is the final Quake payload.
After that lane produces its next clean checkpoint, update the full revision
and both input hashes together, rebuild the combined image, regenerate the
receipt, and rerun the two-pass headless gate before calling the demo-disc
payload current.

The verifier requires the source checkout to be clean and at the full pinned
revision. It requires the cue and bin hashes to match, the cue to describe one
Mode 2 data track, the bin to use whole 2,352-byte sectors, and a PS-X EXE to
exist at the boot LBA expected by `mkdisc`.

Quake's downloaded `quake106.zip`, `PAK0.PAK`, cooked `WORLD.PAK`, and generated
disc images remain untracked. The demo-disc repository commits none of them.

## Build

To rebuild the normal demo-disc programs before layout:

```bash
make quake-disc
```

To reuse already-built demo-disc programs:

```bash
make quake-disc-only
```

The output is separate from both release variants:

```text
PSoXide Demo Disc Quake Shareware/
  PSoXide Demo Disc Quake Shareware.bin
  PSoXide Demo Disc Quake Shareware.cue
  PSoXide Demo Disc Quake Shareware.quake-provenance.json
```

The JSON receipt records the exact Quake source revision, input cue/bin hashes,
combined cue/bin hashes, Quake table entry and LBA relocation, and the number of
embedded Quake data sectors. Receipt generation also compares every embedded
Quake sector against the pinned input. Only the three BCD MSF address bytes that
`mkdisc` must relocate may differ.

An explicit checkout and image can be supplied, but all expected provenance
must travel with it:

```bash
make quake-disc-only \
  QUAKE_SRC=/absolute/path/to/quake-psx \
  QUAKE_CUE=/absolute/path/to/quake-psx.cue \
  QUAKE_EXPECTED_REV=<full-40-character-git-revision> \
  QUAKE_EXPECTED_CUE_SHA256=<64-character-sha256> \
  QUAKE_EXPECTED_BIN_SHA256=<64-character-sha256>
```

Missing files, a dirty or different source revision, a changed hash, an unsafe
cue path, a malformed disc image, a missing menu entry, or embedded payload
drift stops the build.

## Variant isolation and release gate

- `QUAKE` defaults off. `QUAKE=0` expands to the same dry-run recipe as an
  unset value for both default and Half-Life builds.
- Quake and Half-Life flags are mutually exclusive.
- The Quake image is appended after all existing programs, preserving their
  program order and CD-DA ownership.
- The Quake output has a distinct disc name and destination.
- `release-web` and `itch` explicitly reject `QUAKE=1`.
- Public redistribution requires a separate legal and release decision. The
  presence of id Software's shareware data and the ability to build a local
  test disc do not authorize this repository to publish the combined image.

## Automated checks

`make check` runs the Quake verifier tests in addition to the existing Rust
test suites. The tests cover:

- exact revision and image acceptance;
- wrong revision, dirty source, changed bin, and unsafe cue rejection;
- receipt hashes and legal gate;
- table-entry and embedded-sector verification;
- unchanged default and Half-Life dry-run recipes when Quake is off;
- Quake-only metadata, whole-image argument, verifier, receipt, separate name,
  and Half-Life incompatibility.

The strongest local structural check is the receipt itself. It proves that the
combined image has one visible `QUAKE SHAREWARE` table entry pointing to the
relocated PS-X EXE and that the embedded Quake sectors match the pinned image
apart from required address rewriting.

After a build, run the two-pass headless gate with a release frontend:

```bash
make quake-headless-check FRONTEND=/absolute/path/to/PSoXide/target/release/frontend
```

It replays the exact launcher navigation and starts a new Quake game twice. It
requires identical route logs, CD command logs, rendered display, and audio;
pins the final display hashes, route ticks, pad polls, and CD command count;
requires relocated reads within the Quake image; and rejects silent audio.

## Runtime verification boundary

Headless emulator evidence can prove that the demo launcher displays and
chain-loads the Quake entry, and that Quake reaches its own rendered runtime.
It cannot prove original-console CD timing, sustained `WORLD.PAK` streaming,
DMA interaction, or drive behaviour. Those remain hardware gates. Do not call
this release-ready until the combined cue has been burned and exercised on an
original PlayStation.
