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
