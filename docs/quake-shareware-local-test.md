# Quake shareware local/test demo disc

This flow adds the validated Quake-PSX whole image to a separate PSoXide demo
disc variant. It does not alter the default public disc or the Half-Life
pressing, and it does not publish anything.

## Pinned input

The default contract is:

| Item | Pinned value |
| --- | --- |
| Quake source revision | `1fd5173a656cb209b4a10bcbb37d34cc4a0650b0` |
| Quake-declared PSoXide revision | `f9f83c35b140560c123771893a1fc3e426814550` |
| Input cue SHA-256 | `5fa78b12b506d4190246e230183e1eebd677f201ff982a584bff10d88ee2594c` |
| Input bin SHA-256 | `5f5316381763ca54783818097e228762667d6d99e70dd42af04cddd1216c21c1` |
| Default source checkout | sibling `quake-psx-combat-adversarial-review` |
| Default input cue | `build-psoxide/quake-psx.cue` in that checkout |

This pin contains the adversarially reviewed real-map Episode 1 combat
checkpoint. It corrects canonical monster profile bounds and mover broadphase
culling, and adds hostile-map regressions before recording the final validation.
Any later Quake checkpoint must update the full revision and both input hashes
together, rebuild the combined image, regenerate the receipt, and rerun the
two-pass headless gate before the demo-disc payload is called current.

The PSoXide value above records this Quake checkpoint's existing declaration;
it is not a choice of the eventual convergence pin. This demo-disc checkout's
PSoXide submodule is still at `f9f520e1e7ce7d3553ecb3e83f6b66219468f5da`,
so the opt-in verifier intentionally fails until the Quake declaration, its
artifact hashes, and the demo-disc submodule are repinned as one reviewed set.

The verifier requires both the Quake source and the explicitly supplied
PSoXide checkout to be clean repository roots at their full expected
revisions. It reads the single `PSOXIDE_REV` declaration from Quake's build
driver and requires that declaration to equal the PSoXide checkout's exact
HEAD. It also requires the cue and bin hashes to match, the cue to describe one
Mode 2 data track, the bin to use whole 2,352-byte sectors, and a PS-X EXE to
exist at the boot LBA expected by `mkdisc`.

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

The JSON receipt records the exact clean Quake and PSoXide revisions, Quake's
declared PSoXide revision, the ordinary-program revision stamp, input cue/bin
hashes, combined cue/bin hashes, Quake table entry and LBA relocation, and the
number of embedded Quake data sectors. Receipt generation also compares every
embedded Quake sector against the pinned input. Only the three BCD MSF address
bytes that `mkdisc` must relocate may differ.

This cross-revision gate proves that Quake's source contract matches the clean
PSoXide checkout used to rebuild the rest of the disc. It does not prove that
the already-generated Quake cue was built from that checkout. The current
Quake builder persists a hydration stamp, but it does not emit a sidecar bound
to the cue and bin hashes. Receipt schema 2 records that boundary explicitly.
Before the eventual pin can be called artifact-complete, the Quake build must
emit and this verifier must consume a sidecar binding the Quake source
revision, PSoXide revision, and cue/bin hashes.

An explicit checkout and image can be supplied, but all expected provenance
must travel with it:

```bash
make quake-disc-only \
  PSOXIDE=/absolute/path/to/PSoXide \
  QUAKE_SRC=/absolute/path/to/quake-psx \
  QUAKE_CUE=/absolute/path/to/quake-psx.cue \
  QUAKE_EXPECTED_REV=<full-40-character-git-revision> \
  QUAKE_EXPECTED_PSOXIDE_REV=<full-40-character-git-revision> \
  QUAKE_EXPECTED_CUE_SHA256=<64-character-sha256> \
  QUAKE_EXPECTED_BIN_SHA256=<64-character-sha256>
```

Missing files, either dirty or different source revision, a missing or
malformed Quake `PSOXIDE_REV`, cross-revision mismatch, a changed hash, an
unsafe cue path, a malformed disc image, a missing menu entry, or embedded
payload drift stops the build.

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
- wrong revision, dirty Quake or PSoXide source, malformed declaration,
  cross-revision mismatch, changed bin, and unsafe cue rejection;
- receipt hashes and legal gate;
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

It replays the exact launcher navigation and starts a new Quake game twice. It
requires identical route logs, CD command logs, rendered display, and audio;
requires route ticks and pad polls to agree across both runs; pins the final
display hashes and CD command count; requires relocated reads within the Quake
image; and rejects silent audio.

## Runtime verification boundary

Headless emulator evidence can prove that the demo launcher displays and
chain-loads the Quake entry, and that Quake reaches its own rendered runtime.
It cannot prove original-console CD timing, sustained `WORLD.PAK` streaming,
DMA interaction, or drive behaviour. Those remain hardware gates. Do not call
this release-ready until the combined cue has been burned and exercised on an
original PlayStation.
