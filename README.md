# PSoXide demo disc

One CD-R that boots on a real PlayStation into a menu, and chain-loads any of
the PSoXide programs burned alongside it.

```bash
make disc      # -> dist/demo.{bin,cue}
make check     # host tests
```

## How it boots

The BIOS reads `SYSTEM.CNF`, loads `PSX.EXE` (the launcher), and runs it. The
launcher reads `DEMOTOC.BIN` at a fixed LBA to find out what is on the disc,
draws the menu, and on selection streams the chosen program off the disc and
jumps to it.

That last step needs one trick. Every PSoXide program links to `0x80010000`
(`sdk/psoxide.ld`), including the launcher, so the launcher cannot load a game
itself: it would overwrite its own code halfway through the copy. The `loader`
crate is therefore built separately, linked at `0x801F0000`, and embedded in
the launcher as raw bytes. The launcher copies it above every game's payload,
flushes the instruction cache, and jumps. From there nothing below
`0x801F0000` is live, so the game can land wherever it wants.

`mkdisc` refuses to build a disc containing a program whose payload would
reach `0x801F0000`, so that constraint fails the build rather than the console.

## Layout

| Piece | What it is |
| --- | --- |
| `loader/` | the chain-load blob, its own linker script, no `psx-rt` |
| `launcher/` | the menu, a normal PSoXide program |
| `carousel/` | where the ring and the ball of balls land on screen, host-testable |
| `disc-toc/` | the on-disc table format, shared by `mkdisc` and the launcher |
| `tools/mkdisc/` | host disc builder, on top of PSoXide's `psx-iso` |
| `games/` | the source repos, as submodules |

## Sharing a disc

A program built for its own disc has that disc's geometry baked in: pack LBAs
cooked at 1024, CD-DA tracks numbered from 2. `mkdisc` does not re-cook
anything. It drops each game's *existing* disc image onto the big one verbatim
and records two numbers: how far the image moved, and how many CD-DA tracks sit
ahead of it. The loader passes those to the game's `_start`, and
`psx_io::disc_base` applies them inside the three places the SDK addresses the
disc. Both default to zero, so the same binary boots standalone unchanged.

Relocated sectors get their BCD MSF header rewritten so the drive's seeks land.
That is safe in place: Mode 2 Form 1 ECC is computed with those bytes zeroed.

## What is on it

All ten, each verified booting from the built disc. 521 MiB, 51 minutes, 30
CD-DA tracks, comfortably inside an 80-minute CD-R.

| Program | How it ships |
| --- | --- |
| Cortex Ignition | whole image, 1 CD-DA track |
| Half-Life | whole image, 27 CD-DA tracks |
| Voxide | bare EXE |
| Celeste Classic Collection | bare EXE |
| PSXcel | bare EXE |
| Guitar Hero (gh-psx) | whole image, 1 CD-DA track |
| Breakout | bare EXE |
| Space Invaders | bare EXE |
| Magikaaaaarp Pong | bare EXE, plays Guitar Hero's track |
| Hardware Tests | whole image, 1 CD-DA track |

Voxide, Celeste and PSXcel never read the disc after boot, so they ride as
bare EXEs and do not care which SDK built them: a `_start` that ignores the
loader's arguments is still a correct `_start`.

## The menu

A carousel of glossy blue pills under a turning ball of balls, over a
starfield: the PlayStation demo discs, as closely as flat and gouraud
triangles get you. Left and right spin the ring, X runs the pill at the front,
up or down swaps the description between English and Italian under a drawn
flag. Titles too long for a pill break across two lines.

The whole scene answers the music. `tools/beatgrid.py` fits a tempo and phase
to each track's onset envelope offline and the numbers ship in the disc table,
so the grid stays in step for the length of a track rather than drifting out of
it. On that grid: the ball blows outward on every beat and snaps back, harder
on the downbeat; the pills flash on the downbeat only; the starfield twinkles
on the offbeat, in the gaps the other two leave; and the ball's idle spin rides
the bar, quickest just after the downbeat.

It answers the pad too. Browsing shoves the ball's spin, which coasts back down
over the next second, and the same shove blows the beads apart and lets them
re-form. The starfield drifts with it, brighter stars faster.

A shaft of light crosses the frame behind everything, drawn additively so it
brightens what it passes rather than covering it, and the carousel is mirrored
in the floor below it.

None of it uses a texture or a float. Ellipses are triangle fans whose segment
count follows their size: at a flat twelve the ball alone put the frame over
2000 triangles and the menu stopped holding 60 Hz, which stretched every
time-driven effect with it.

No textures and no floating point. Ellipses are twelve-segment triangle fans
shaded top to bottom; the ring and the sphere are one perspective divide each,
depth-sorted back to front.

## Status

Verified headless against a real BIOS, booting the built `.cue`:

- the menu reads the disc table, navigates, and chain-loads all seven entries
- `hello-pack` streams its pack and reports ALL PASS with its image relocated
  220 sectors in, and still reports ALL PASS standalone (`make relocation-check`)
- two CD-DA discs on one image play 440 Hz and 1000 Hz respectively, so the
  second one's track base shifted it off track 2
- Magikaaaaarp Pong plays audio off Guitar Hero's track, one copy on the disc

## Building

`make disc` writes into PSoXide's game library, so the disc shows up in the
emulator next to everything else:

```
~/Downloads/ps1 games/PSoXide Demo Disc/PSoXide Demo Disc.{bin,cue}
```

Override with `DIST=...` for somewhere else, or `PSOXIDE_LIB=...` for a
different library. gh-psx keeps its audio in a gitignored `data/audio/`, so a
fresh clone needs that dropped in before `make disc` will get past it.
