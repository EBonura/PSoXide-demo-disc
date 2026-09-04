# Comicon disc build and audio audit

Both pressings build with Cortex Ignition 0.5's replacement menu music and
new combat track. Adding combat shifts later physical track numbers by one;
the per-game CDDA bases shift with them. There is no off-by-one audio mapping.

| Owner | Standard pressing | HL pressing |
| --- | --- | --- |
| Launcher songs | 2-5 | 2-5 |
| Cortex combat (`cortex_boss.wav`) | 6 | 6 |
| Cortex menu (`menu2.wav`) | 7 | 7 |
| Half-Life | absent | 8-34 |
| GH-PSX and PSoXide Arcade, shared | 8 | 35 |
| Hardware Tests | 9 | 36 |

VoXide, NitroXide, Celeste Collection, PSXcel and Quake append no CD audio.
Each disc also has track 1 for data: 9 total tracks without HL, 36 with HL.

The independent audit compares every appended game audio span, including
pregaps, against the source BIN/CUE; compares all four launcher songs; checks
every game's cumulative base, shared ownership and complete track coverage.
All 8 standard and 35 HL audio tracks pass. A separate PCM comparison verifies
the two Cortex tracks against the actual WAV files, including sector padding.

Reproduce from the repository root after building the programs:

```sh
python3 docs/comicon-audio-2026-09-04/audit_audio.py '/path/PSoXide Demo Disc.cue'
python3 docs/comicon-audio-2026-09-04/audit_audio.py '/path/PSoXide Demo Disc HL.cue'
```

The build starts with `make disc HL=1 DIST=/tmp/astra-perf-20260904/disc-candidate`.
After the validated Cortex and Quake improvements, those guests and the
launcher are rebuilt and both layouts regenerated with `make disc-only`.
The runtime and Quake SDK pins use `b1ee0fd5`; Cortex uses `eee3aa93`;
Quake uses `c2329be`; HL uses `feeb9b3` with the corrected MIPS hazard patcher and measured
terminal-depth optimization.

The runtime fixes CDDA fade duration and clears pending fades during data
loading. Cortex also uses word-aligned decoded vertices without growing
resident pools. Quake takes the newer shared packet linker. HL skips unused terminal-depth LOD calculations with unchanged tessellation
and arenas. Performance
evidence lives in the corresponding PSoXide and Quake reports.

All nine independent-program routes pass on each pressing. The full HL
chain-load battery passes Cortex, Half-Life, Hardware Tests and Quake twice;
after the final Cortex/Quake rebuild, those two pass again with deterministic
logs. Cortex sustains 466 gameplay frames in that staged check. These are
headless build/runtime checks; original-console validation remains separate.

Evidence, hashes and the independent audit script are in
[`comicon-audio-2026-09-04/`](comicon-audio-2026-09-04/).

The fixed-frame Quake pairs were refreshed after the validated runtime/source
pins changed. Both pressings pass the official two-replay gate: payload and
receipt checks, relocated CD reads, exact final frames and all six log types.

## Delivered build

The delivered images were built from clean disc source `dad175d`, with the
source revisions above, into `~/Downloads/PSoXide-Comicon-2026-09-04`.
Both audio audits pass again. Both official Quake two-replay gates pass.
The standard pressing passes all nine independent-program routes, and the
HL pressing passes the full four-target release battery with byte-identical
paired logs. The HL build receipt verifies all embedded payload identities.
`delivery-*.txt` and `delivery-hashes.json` capture these final results.
The following commit only archives validation evidence; the image build
revision remains `dad175d`.
