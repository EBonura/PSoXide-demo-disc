# Comicon second stack pass

All programs now use the shared runtime at PSoXide 26698622. Quake is pinned
to 60190c7 and HL to de51288. These revisions contain the measured shared
screen checks, GPU extent proof, Cortex packet batching and Quake model-ID
lookup improvement. No music assets or per-game audio policies changed.

Measured full-route Cortex bus cycles fall 2.575%, and work instructions fall
2.958%. Its slowest measured 1,000-poll section improves 5.06%. Quake work falls
0.806% and bus cycles 0.658%; HL's full tram improves 21.585 -> 21.658 fps, with
slow segments still below 20 fps. Final display and VRAM remain exact in all
three standalone route comparisons. These are emulator measurements.

The SDK report is docs/perf-stack-pass-2026-09-04.md in PSoXide. Local raw
artifacts are under /tmp/astra-stack-pass-20260904. The previous validated
Comicon images remain in Downloads/PSoXide-Comicon-2026-09-04 while this pass's
new images and receipts are prepared separately.

Both pressings retain launcher tracks 2-5, Cortex combat 6 and Cortex menu 7.
The standard pressing then has the shared GH/Arcade track 8 and hardware-test
track 9. The private HL pressing has HL tracks 8-34, GH/Arcade 35 and hardware
tests 36. The independent audio audit compares the actual pressed sectors,
including pregaps, with every source image and validates every game's base.


## Validated images

The new BIN/CUE pairs are in
`/Users/ebonura/Downloads/PSoXide-Comicon-2026-09-04-stack-pass`.
They were built from clean disc source d5bd1d7. Later commits record the frame
reference and evidence; the images retain that exact build identity.

Both independent audio audits pass (8 standard and 35 HL tracks), both WAV PCM
comparisons pass, and the private full-build receipt verifies. Each pressing
passes every applicable critical chain-load target twice with identical logs.
Both official Quake pairs pass; all nine other program routes pass on each.
The standard Quake 500-million-instruction frame reference was refreshed after
inspection against the previous image: the same Start-map menu is rendered at
a slightly different animation phase. The complete E1M1-to-E1M2 route keeps its
canonical hashes. The HL pressing's fixed-budget Quake reference did not move.

The Cortex shipping route still presents every two route ticks after entering
gameplay, matching the previous disc. A smaller frame count at the fixed CPU
instruction budget reflects an earlier bus-time endpoint, not lower cadence.
The full-disc checks cover actual relocated payloads and normal shipping
features, separately from the fixed-cadence whole-level performance tape.

`make check` passes: 92 tracked lockfiles, seven hydrated runtimes, source
ancestry/provenance, 55 Rust tests and 66 Python tests. Evidence and exact image
hashes are archived in [comicon-stack-pass-2026-09-04/](comicon-stack-pass-2026-09-04/).
