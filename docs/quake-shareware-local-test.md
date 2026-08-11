# Quake shareware local/test demo disc

This flow adds the validated Quake-PSX whole image to a separate PSoXide demo
disc variant. It does not alter the default public disc or the Half-Life
pressing, and it does not publish anything.

## Pinned input

The default contract is:

| Item | Pinned value |
| --- | --- |
| Quake source revision | `2d26f9eeb624a562ba00f4ced121b740fd61d4bf` |
| Quake-declared PSoXide revision | `f9f83c35b140560c123771893a1fc3e426814550` |
| Shipping provenance sidecar | `dist/quake-psx.provenance.json` |
| Sidecar SHA-256 | `f3f1cf7a837b640af3efba22dee505f581cfea91eb21b6795af734d8dacebc31` |
| Input cue SHA-256 | `5fa78b12b506d4190246e230183e1eebd677f201ff982a584bff10d88ee2594c` |
| Input bin SHA-256 | `76cb839698d69c697116d4ab65112869d80c2d959b53ef40c09479a6490d00b8` |
| Input EXE SHA-256 | `ad4464f6dd64b1132cded9fd5d539724d7c2805b54a5c4ea05e183657049fc56` |
| Guest recipe SHA-256 | `45bbc05f57cb18100c5061b9f9f8b7a1827bedd1134ff7698c2ad5aca9cc4676` |
| Default source checkout | sibling `quake-psx-build-provenance` |
| Default input cue | `dist/quake-psx.cue` in that checkout |

This pin contains the adversarially reviewed real-map Episode 1 combat
checkpoint and its reproducible shipping builder. The builder projects the
guest source into a content-addressed canonical stage, isolates shipping Cargo
inputs, rejects ambient build overrides, and emits the schema-1 sidecar consumed
here. Two clean checkouts at different absolute paths produced byte-identical
EXE, BIN, CUE, and sidecar outputs at this pin.
Any later Quake checkpoint must update the full revision and every pinned hash
together, rebuild the combined image, regenerate the receipt, and rerun the
two-pass headless gate before the demo-disc payload is called current.

The demo-disc PSoXide submodule, Quake's source declaration, and Quake's
shipping sidecar all name `f9f83c35b140560c123771893a1fc3e426814550`.
The ordinary demo-disc programs must be rebuilt from that same clean revision
before the Quake layout step records its revision stamp.

The verifier requires both the Quake source and the explicitly supplied
PSoXide checkout to be clean repository roots at their full expected
revisions. It reads the single `PSOXIDE_REV` declaration from Quake's build
driver and requires that declaration to equal the PSoXide checkout's exact
HEAD. The sidecar must then repeat both clean revisions, identify a local
PSoXide checkout and the canonical Quake 1.06 shareware PAK, record the
content-addressed guest recipe and Rust/Cargo identities, and declare a release
build with no extra features. Its cue, bin, and EXE basenames, byte sizes, and
SHA-256 values are checked against the actual files. The cue must describe one
Mode 2 data track, the bin must use whole 2,352-byte sectors, and a PS-X EXE
must exist at the boot LBA expected by `mkdisc`.

Quake's downloaded `quake106.zip`, `PAK0.PAK`, cooked `WORLD.PAK`, and generated
disc images remain untracked. The demo-disc repository commits none of them.

## Build

To rebuild the normal demo-disc programs before layout:

```bash
make quake-disc
```

The full target removes any prior ordinary-program revision stamp, rebuilds
the programs with `PSOXIDE`, runs `sdk-coherence`, verifies that PSoXide is
still clean, and atomically writes its full HEAD to
`build/programs.psoxide-revision` before layout. Cortex Ignition is copied from
the tracked `editor/samples/cortex_v1` project into `build/cortex_v1` before it
is baked, so generated project output does not dirty the PSoXide checkout.

To reuse already-built demo-disc programs:

```bash
make quake-disc-only
```

This reuse path, and therefore `quake-headless-check`, requires the stamp from
a completed `quake-programs` stage in the full build. A missing, malformed, or
revision-mismatched stamp stops the build. It cannot silently assemble or
replay ordinary program artifacts left over from another PSoXide revision.

The output is separate from both release variants:

```text
PSoXide Demo Disc Quake Shareware/
  PSoXide Demo Disc Quake Shareware.bin
  PSoXide Demo Disc Quake Shareware.cue
  PSoXide Demo Disc Quake Shareware.quake-provenance.json
```

Receipt schema 3 records the exact clean Quake and PSoXide revisions, Quake's
declared PSoXide revision, the ordinary-program revision stamp, the complete
shipping-build contract, input provenance/cue/bin/EXE hashes and sizes,
combined cue/bin hashes, Quake table entry and LBA relocation, and the number
of embedded Quake data sectors. Receipt generation also compares every
embedded Quake sector against the pinned input. Only the three BCD MSF address
bytes that `mkdisc` must relocate may differ.

An explicit checkout and image can be supplied, but all expected provenance
must travel with it. The sidecar must remain beside the cue and be named from
the cue stem exactly as shown:

```bash
make quake-disc-only \
  PSOXIDE=/absolute/path/to/PSoXide \
  QUAKE_SRC=/absolute/path/to/quake-psx \
  QUAKE_CUE=/absolute/path/to/quake-psx.cue \
  QUAKE_PROVENANCE=/absolute/path/to/quake-psx.provenance.json \
  QUAKE_EXPECTED_REV=<full-40-character-git-revision> \
  QUAKE_EXPECTED_PSOXIDE_REV=<full-40-character-git-revision> \
  QUAKE_EXPECTED_PROVENANCE_SHA256=<64-character-sha256> \
  QUAKE_EXPECTED_CUE_SHA256=<64-character-sha256> \
  QUAKE_EXPECTED_BIN_SHA256=<64-character-sha256> \
  QUAKE_EXPECTED_EXE_SHA256=<64-character-sha256>
```

Missing or malformed sidecars, duplicate JSON keys, unsupported schemas,
either dirty or different source revision, a missing or malformed Quake
`PSOXIDE_REV`, cross-revision mismatch, noncanonical shareware, a nonshipping
build configuration, a changed artifact name/size/hash, an unsafe cue path, a
malformed disc image, a missing menu entry, or embedded payload drift stops the
build.

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

- exact revision, sidecar, build recipe, shareware, and artifact acceptance;
- missing/malformed/duplicate-key/stale sidecars;
- wrong source kind, shareware identity, build profile/features, guest recipe,
  toolchain identity, artifact basename, size, hash, and bytes;
- wrong revision, dirty Quake or PSoXide source, malformed declaration,
  cross-revision mismatch, changed bin, and unsafe cue rejection;
- receipt schema 3, shipping-build facts, hashes, and legal gate;
- table-entry and embedded-sector verification;
- unchanged default and Half-Life dry-run recipes when Quake is off;
- Quake-only metadata, whole-image argument, verifier, receipt, separate name,
  and Half-Life incompatibility.
- post-program SDK coherence, missing/malformed/stale SDK stamp rejection, and
  tracked Cortex staging.

The strongest local structural check is the receipt itself. It proves that the
combined image has one visible `QUAKE SHAREWARE` table entry pointing to the
relocated PS-X EXE and that the embedded Quake sectors match the pinned image
apart from required address rewriting.

After a build, run the two-pass headless gate with a release frontend:

```bash
make quake-headless-check FRONTEND=/absolute/path/to/PSoXide/target/release/frontend
```

It fast-boots this combined disc's real launcher through the frontend's HLE disc
path, then replays two RIGHT presses and CROSS twice. The checker reads the
pressed `PSXDEMO4` table first, so it fails if that exact route no longer lands
on `QUAKE SHAREWARE`. It then requires, in order, the launcher's boot and
chain-load TTY markers followed by Quake's own entry-point and successful Start
map residency markers.

No screenshot, PPM, frame dump, WAV, or instrumented guest is involved. Both
runs must have byte-identical route, CD command, GPU command census, aggregate
PC, PC callsite, and windowed PC logs. Their hashes, route ticks, pad polls, CD
command count, final PC, cycles, and final VRAM/display hashes are pinned. The
CD log must seek and read the Quake EXE header and payload at the table's exact
relocated LBA, while the final PC must be inside the embedded Quake payload and
the out-of-band sampler must observe that address range. This is the durable
proof that the menu selected Quake, the loader verified and entered it, and
Quake reached its own runtime.

## Runtime verification boundary

The regular BIOS-backed frontend path does not surface SDK TTY output, so a
display hash on that path cannot identify which executable owns the pixels.
The gate deliberately uses `--embedded-playtest`: only the first disc EXE boot
is HLE; the launcher still reads the pressed table, handles the menu input, and
runs the same high-RAM CD loader against the combined image. This makes the
shipping launcher and Quake TTY markers capturable without changing either
guest.

Headless emulator evidence cannot prove original-console BIOS startup, CD
timing, sustained `WORLD.PAK` streaming, DMA interaction, or drive behaviour.
Those remain hardware gates. Do not call this release-ready until the combined
cue has been burned and exercised on an original PlayStation.
