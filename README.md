# PSoXide demo disc

One CD-R that boots on a real PlayStation into a menu, and chain-loads any of
the PSoXide programs burned alongside it.

The pressed disc is on
[itch.io](https://bonnie-studios.itch.io/psoxide-demo-disc), and the
[PSoXide page](https://bonnie-studios.itch.io/psoxide) runs this exact image
in your browser, menu music and all: the emulator streams the data track
first and pulls the CD audio in behind it.

```bash
make disc      # -> "PSoXide Demo Disc.{bin,cue}" in the PSoXide game library
make check     # host tests
```

The Quake shareware build is a third, deliberately local/test-only variant:

```bash
make quake-disc       # rebuild programs, then add the pinned Quake image
make quake-disc-only  # reuse built programs and add the pinned Quake image
```

It writes `PSoXide Demo Disc Quake Shareware.{bin,cue}` to a separate library
directory, so it cannot overwrite either release pressing. It is not a public
release target. The reuse and headless paths require a full-build PSoXide
revision stamp, so stale ordinary-program artifacts fail closed. See
[the Quake local/test runbook](docs/quake-shareware-local-test.md).

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

All eleven, each verified booting from the built disc. 521 MiB, 51 minutes, 30
CD-DA tracks, comfortably inside an 80-minute CD-R.

| Program | How it ships |
| --- | --- |
| Cortex Ignition | whole image, 1 CD-DA track |
| Half-Life | whole image, 27 CD-DA tracks |
| Voxide | whole image, WORLD.PAK assets |
| NitroXide | whole image, WORLD.PAK arena atlas |
| Celeste Classic Collection | bare EXE |
| PSXcel | bare EXE |
| GH-PSX | whole image, 1 CD-DA track |
| Breakout | bare EXE |
| Space Invaders | bare EXE |
| Magikaaaaarp Pong | bare EXE, plays GH-PSX's track |
| Hardware Tests | whole image, 1 CD-DA track |

Celeste and PSXcel never read the disc after boot, so they ride as bare EXEs
and do not care which SDK built them: a `_start` that ignores the loader's
arguments is still a correct `_start`. Voxide and NitroXide load `WORLD.PAK`
at startup, so their complete images ride the same relocation path as the
larger streaming games.

The opt-in local/test variant adds `QUAKE SHAREWARE` as one more whole image.
Quake streams `WORLD.PAK`, so a bare executable is not sufficient. The entry
uses the same caller-provided LBA offset that relocates Voxide, NitroXide, and
the other streaming programs.

## The menu

A carousel of glossy blue pills under a turning ball of balls, over a
starfield: the PlayStation demo discs, as closely as flat and gouraud
triangles get you. Left and right spin the ring, X runs the pill at the front, up or down swaps
the description between English and Italian under a drawn flag, and L1 or R1
skips the music. Titles too long for a pill break across two lines.

Top left is a music panel: what is playing, a sixteen-band level meter, and the L1/R1
label. The meter is not keeping time, it is listening: `mkdisc` runs an FFT
over each track at disc-build time and ships the result in a `SPECTRUM.BIN`
region the launcher reads at boot. Bands are normalised per band rather than
globally, because a drum and bass track has so much more energy at 60 Hz than
at 12 kHz that a single scale leaves the top half of the meter permanently
flat. Only that control is labelled on screen. The rest a PlayStation owner
tries without being told; nothing about the screen suggests the shoulder
buttons do anything at all.

Track titles come off the disc like everything else the menu shows, so the
launcher still knows nothing about what it is playing until it reads the
table. That table now spans two sectors, since the titles and two
descriptions per program stopped fitting in one.

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

Choosing a program is an event rather than a cut: the camera accelerates into
the starfield until the stars streak into lines, the ball blows itself apart
past the edges of the screen, and only then does the fade hand over to the
chain-load. A tail brighter than its head marks a star that wrapped back to
the far plane between two frames, which is what keeps a streak from crossing
the whole sky.

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
different library. `PSOXIDE=/absolute/path/to/PSoXide` selects the checkout
used for every rebuilt program. Cortex is staged from that checkout's tracked
`editor/samples/cortex_v1` sample into `build/`, keeping generated bake output
outside the PSoXide worktree. gh-psx keeps its audio in a gitignored
`data/audio/`, so a fresh clone needs that dropped in before `make disc` will
get past it.

Two release pressings exist: `make disc` builds the public one, and `make disc
HL=1` adds Half-Life for show-floor demos. The `games/hl-psx` submodule is
private until its own release, so a fresh clone should init the other
submodules selectively and stick to the default pressing. `make quake-disc`
is a separate local/test artifact and cannot be combined with `HL=1`.
