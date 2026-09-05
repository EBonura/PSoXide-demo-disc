# Demo-disc roundup, 5 September 2026

Build source: `81f7a84` on `codex/demo-disc-roundup-20260905`.
Remote main heads were refreshed at the start of this pass. The following
updates are included in the standard and private Half-Life images as applicable.

| Project | Previous disc pin | Included pin | Changes |
|---|---|---|---|
| Cortex Ignition | `8df242b3`, project 0.5 | `97a604f6`, project **0.4b** | Combat, camera, perception, audio and ending fixes; backend code pruning; wide division removed. |
| Half-Life | `7a78459` | `22c4f7c` | Pickup contacts, beverage dispensers and visible pickups; refined affine texturing and rectangular wall grids. HL edition only. |
| VoXide | `8b25dba` | `5c119ec` | Includes upstream `20b939c`: terrain-renderer spill/depth work and near-grid axis optimization; corrected bootstrap SDK metadata. |
| NitroXide | `38ece21` | `c11b8a7` | Includes upstream `a165e11`: analog controller negotiation and patched delay-slot build; corrected SDK lock/bootstrap metadata. |
| Quake | `263f43f` | unchanged | Existing validated artifact/provenance pin retained. |
| PSXcel | `ff9025e` | unchanged | No newer pushed main commit. |
| Celeste collection | `66f900e` | unchanged | No newer pushed main commit. |
| GH-PSX | `37b5b65` | unchanged | No newer pushed main commit. |
| PSoXide Arcade | `3ddaf7f` | unchanged | No newer pushed main commit. |
| Shared SDK and hardware suite | `8df242b3` | unchanged | Current pushed PSoXide main; Cortex retains its independently pinned development branch. |

NitroXide's manifest selected `e4f27c2f`, but its bootstrap constant and lockfile
still selected `3d274b74`. VoXide's manifest/lock selected `8df242b3`, but its
bootstrap constant still selected `3d274b74`. The fixes align those declarations
without changing the SDK selected by the demo disc (`PSOXIDE_FROM` points all
ordinary programs to its clean `8df242b3` runtime). They are pushed on each
game's `codex/demo-disc-sdk-metadata-20260905` branch, which this disc pins.

The Cortex input is the exact normal disc built and replay-verified in the
preceding arithmetic pass at `97a604f6`, staged into the build cache after
checking its identity. Its executable SHA-256 is
`c42a834c8f1aef3e25607025b57f800114ff6515e9ec452c201c287cb418c74b`;
BIN SHA-256 is
`f84cb4b0c479d34a28906b5c3aecef914ccc0e9f98d73284a28e42459592fcb0`.
The recipe now selects 0.4b, displays that project version, and enforces the
64-bit guest-symbol gate on cached and fresh Cortex builds. Other programs
were rebuilt by `make disc`; Half-Life was fully recooked before packing.

Artifacts live directly in `/Users/ebonura/Downloads/ps1 games/`:
`PSoXide Demo Disc.{bin,cue}` and `PSoXide Demo Disc HL.{bin,cue}`.
These are local test builds; no publishing or physical burn is part of this pass.

Final-image validation:

- `make check` passes, including SDK coherence, reproducibility of all 92
  tracked lockfiles, five standalone SDK pin checks, Rust tests and 66 Python
  tests. The Cortex contract fixture now checks the selected 0.4b project.
- The final Half-Life image passes its release-receipt verification.

- Both editions pass all nine independent-program headless routes, including
  the three Arcade games.
- Cortex, hardware tests and Quake pass two byte-deterministic chain-load
  replays on each edition. Half-Life also passes both replays on the HL edition.
  Cortex and Half-Life continue reading runtime assets after chain-loading.
- All 8 standard-disc audio tracks and all 35 HL-disc audio tracks match their
  source bytes, including each embedded game's pregaps and CDDA track mapping.
- The normal Cortex executable passes its 64-bit guest-symbol gate. The Cortex,
  VoXide, NitroXide and Half-Life executable scans report zero branch/load-delay
  hazards. NitroXide additionally reports four straight-line advisories in
  apparent embedded data; these are not counted as verified code hazards.
- These checks are emulator and build validation, not a new physical-console test.

The adjacent logs identify the exact final image and emulator hashes. The
final standard pack was performed after the full HL build so both launchers
identify build source `81f7a84`. `SHA256SUMS` records both BIN/CUE pairs.
The later evidence/test-fixture commit does not change those image bytes.


## Main integration: Half-Life depth ordering

The source pin advances from `22c4f7c` to
`50778a1eea549c046780cd85a37abbb184892568`, already on HL-PSX main with
passing Source CI. It adds fractional actor-depth sorting inside the existing
PS1 ordering-table buckets and keeps the rectangular wall-grid improvements.
The matched scientist probe reduces wrong-depth samples from 177 to 52; the
full opening tram measures 21.445 fps versus 21.476 fps before. Persistent RAM
and primitive capacity remain unchanged.

The validated normal standalone executable has SHA256
`383bc08912a4addc7d4af851e7f5c95c7b64a9fd94fd33f27fa340cf525bda9f`; its disc has
SHA256 `d5c8fd225a28e3cd7c8c897930df505f3f3d1f857e27fb47fc46fe11f6b2cd00`.
All 27 Half-Life CDDA tracks and standalone CUE indices remain unchanged.

The combined-disc hashes and replay logs above describe the earlier
`22c4f7c` pressing. They do not certify a combined disc using the new pin.
Rebuild with the normal release configuration and rerun the combined-disc
receipt, chain-load and relocated audio checks before using the new pressing.
Detailed HL validation is in the submodule's
`docs/depth-ordering-2026-09-05.md`.

## Updated Cortex menu music

Cortex now pins `b77a054775342282967c6336da4df7bbf1abd010`, which replaces `menu2.wav` with the
updated `Cortex Intro.wav` delivery. The three menu nodes retain their existing
settings; combat music and the audio track lengths are unchanged. The source
WAV SHA256 is `a5355af0d22dfe89e0d267a63f23178053b6a3a61fbf4a372e7c277749acbce1`.
The changed revision invalidates the Cortex build cache on the next disc build.
Earlier combined-disc hashes in this directory predate this audio replacement.

## Updated Cortex combat music

Cortex now pins `d2132e7e2946d1938a76b7759f404d10e191b7c4`, adding the updated
`Cortex Boss (1).wav` as `cortex_boss.wav` while retaining the new menu music.
The combat track is 72.727279 seconds, down from 77.090930 seconds, and its
padded audio payload is 327 sectors shorter (5,455 instead of 5,782). Rebuild
the Cortex input and combined disc so all subsequent offsets are regenerated.
The changed source revision invalidates the Cortex build cache automatically.
Source WAV SHA256: `18e07a90fa5f674fdfcd92c835a7606b0deead4346d4f3aeb526fc98cb84ce93`.
Earlier combined-disc hashes and audio receipts predate this replacement.
