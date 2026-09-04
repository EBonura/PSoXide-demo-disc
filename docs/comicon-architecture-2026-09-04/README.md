# Comicon demo discs: third measured pass

Built from clean demo-disc commit 2a83e70, SDK 8df242b3, Quake 263f43f and
HL 5a79262. Both BIN/CUE pairs are fully rebuilt release images. Experimental
HL tessellation changes are not in these images.

Cortex uses the frozen tracked 0.5 project, including combat cortex_boss.wav
and the replacement menu2.wav. Combat is physical track 6 and menu is track 7.
All 8 standard and 35 HL audio tracks, their relocation bases, pregaps and
source bytes pass. Both Cortex tracks additionally match original WAV PCM.

Both layouts pass two deterministic launches of every critical entry, nine
smaller-game routes, and the official two-run Quake check. The HL guest and
cooked assets are byte-identical to the previous release. SDK/game pins and
release receipt are verified. Validation evidence is beside these images.

Cortex full-route emulator bus cycles improve 1.77% over the previous pass.
Quake's isolated lookup saves 0.31%; the final pinned rebuild measures 0.23%
with a 0.21% reduction in useful instructions. HL retains its previous timing.
These are incremental gains. Original-console timing has not been tested.

The fixed-instruction Quake screenshot pins changed with animation phase.
Old/new 320x240 frames were compared, then both new replays agreed exactly.
The complete gameplay route retains the previous display and VRAM hashes.
The published checker update follows the image-build commit above.

Use SHA256SUMS.txt to verify the files. The HL pressing remains private.

Image hashes:

```text
24e614727e16c087ed54fbfb6b8424acdaedb528c2dd0d4504c1d42b91cd2b04  PSoXide Demo Disc HL.bin
f558bf637a01b35b686a4d9b17c1911627cc211bca1148e1a2d25d5e08947fb6  PSoXide Demo Disc HL.cue
3aa5ffd8ec92b528a555c7ea9d713d916f4e482caed338ede5ef9d42ff7c0c7b  PSoXide Demo Disc.bin
85d926750d5e2f8a8953407bc43c5766b309ea584b407d31a2ab8068101db734  PSoXide Demo Disc.cue
```
