# Quake shareware on the demo disc

Quake 1.06 shareware Episode 1 is a default program on the demo disc. `make
disc` builds it, verifies it against the pins below, and writes a provenance
receipt beside the image; the Half-Life pressing carries it too. There is no
switch that leaves it off.

Building it is local. Publishing it is a separate owner decision that has not
been made, and `release-web` and `itch` are blocked until it is.

## Pinned input

The default contract is:

| Item | Pinned value |
| --- | --- |
| Quake source revision | `28507a6dd605730a43909d6b2258f081def68a79` |
| Quake-declared PSoXide revision | `79d51dd2f2fd78cfb8aa418e2ad123730f56ac3d` |
| Shipping provenance sidecar | `dist/quake-psx.provenance.json` |
| Sidecar SHA-256 | `8560401739d0def8ec08cfafee9ec02ba90dfb3d62933946f8d7198401d5506d` |
| Input cue SHA-256 | `5fa78b12b506d4190246e230183e1eebd677f201ff982a584bff10d88ee2594c` |
| Input bin SHA-256 | `50a6a9d792e3ef074dd49e300dcb69fb5f2d84a7d2d053c07ae9c8d473f0b1a2` |
| Input EXE SHA-256 | `f0b3e7ff814c5eba320ea78bceb6f38bdec1938f971a52a87213dd774ca6de85` |
| Guest recipe SHA-256 | `3f94b7935843619b24075dbe13514968d7315aa4305afab89e4d2e775c936610` |
| Default source checkout | sibling `quake-psx-comicon-final` |
| Default input cue | `dist/quake-psx.cue` in that checkout |

This pin is the Comicon playable-beta candidate. All nine shareware maps cook
and their indexes validate; the Start-to-E1M1 route and the complete E1M7
Chthon/intermission route are guest-proven, E1M2 is guest-proven end to end,
and the previously
blocking E1M3/E1M4 platform and shootable-button mechanisms have direct guest
evidence. It is deliberately not a claim that every normal and secret route has
an automated end-to-end proof; E1M3 through E1M6 and E1M8 still need ordinary
playthrough coverage and later polish. The reproducible shipping builder
projects the guest source into a content-addressed canonical stage, isolates
shipping Cargo inputs, rejects ambient build overrides, and emits the schema-1
sidecar consumed here. Path-independent byte reproducibility is covered by
builder tests and was measured at earlier named checkpoints; it has not been
rerun from two paths at this exact source pin.
Any later Quake checkpoint must update the full revision and every pinned hash
together, rebuild the combined image, regenerate the receipt, and rerun the
two-pass headless gate before the demo-disc payload is called current.
`make quake-repin` measures all six from a built tree; README.md has the
procedure.

The demo-disc PSoXide submodule, Quake's source declaration, and Quake's
shipping sidecar all name `79d51dd2f2fd78cfb8aa418e2ad123730f56ac3d`.
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

The ordinary full build:

```bash
make disc
```

Its `quake-programs` stage removes any prior ordinary-program revision stamp, rebuilds
the programs with `PSOXIDE`, runs `sdk-coherence`, verifies that PSoXide is
still clean, and atomically writes its full HEAD to
`build/programs.psoxide-revision` before layout. Cortex Ignition is copied from
the tracked `editor/samples/cortex_v1` project into `build/cortex_v1` before it
is baked, so generated project output does not dirty the PSoXide checkout.

To reuse already-built demo-disc programs:

```bash
make disc-only
```

This reuse path, and therefore `quake-headless-check`, requires the stamp from
a completed `quake-programs` stage in the full build. A missing, malformed, or
revision-mismatched stamp stops the build. It cannot silently assemble or
replay ordinary program artifacts left over from another PSoXide revision.

The output is the ordinary disc, with the receipt beside it:

```text
PSoXide Demo Disc/
  PSoXide Demo Disc.bin
  PSoXide Demo Disc.cue
  PSoXide Demo Disc.quake-provenance.json
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
make disc-only \
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

## Default inclusion and release gate

- There is no opt-in switch. `make disc` and `make disc-only` carry Quake, and
  so does `make disc HL=1`.
- The Quake image is appended after all existing programs, preserving their
  program order and CD-DA ownership. It owns no CD-DA track.
- `disc-only` will not lay out a sector until `quake-programs-verify` and
  `quake-verify` have passed, and `make check` runs `quake-verify` too.
- `release-web` and `itch` depend on `publication-block`, which always fails.
  They stop before they build anything.
- Public redistribution requires a separate legal and release decision. The
  presence of id Software's shareware data and the ability to build the disc
  locally do not authorize this repository to publish the combined image. That
  decision has not been made here.

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
- the default and Half-Life dry-run recipes both carrying the Quake image,
  its metadata, the verifier and the receipt, and the opt-in switch being gone;
- every way the payload can be wrong stopping `make quake-verify`: absent,
  stale Quake pin, wrong PSoXide pin, stale artifact hash, either checkout
  dirty, stamp stale, stamp missing;
- `make check` reaching the verifier, layout not starting before it passes,
  and the publication block failing before either upload path builds;
- the carousel's entry count and Quake's position in it, a hidden or absent
  Quake entry failing, and payload identity against the receipt;
- `make quake-repin` printing every pin the Makefile holds.
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
runs must have byte-identical stdout, summaries, and route, CD command, GPU
command census, aggregate PC, PC callsite, and windowed PC logs.

Two absolute pins remain: the final VRAM and display FNV-1a-64. Those are
Quake's output and hold across a launcher rebuilt from another path, a changed
`DISC_VERSION`, and a renamed pressing. Cycles, route ticks, pad polls, CD
command totals, the final PC and the six log digests are NOT pinned: they move
with the launcher binary, which changes on every commit here, so pinning them
made the gate fail on unrelated work. They are held to run-to-run equality
instead.

Structurally, the carousel must have its expected number of visible entries
with `QUAKE SHAREWARE` visible at the position the route lands on, the table's
record of the payload must match the receipt, the CD log must seek and read the
Quake EXE header and payload at the table's exact relocated LBA, both replays
must end with the PC inside the embedded Quake payload, and the out-of-band
sampler must observe that address range. This is the durable proof that the
menu selected Quake, the loader verified and entered it, and Quake reached its
own runtime.

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
