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

Seven of the nine, all verified booting from the built disc:

| Program | How it ships |
| --- | --- |
| Voxide | bare EXE |
| Celeste Classic Collection | bare EXE |
| PSXcel | bare EXE |
| Guitar Hero (gh-psx) | whole image, 1 CD-DA track |
| Breakout | bare EXE |
| Space Invaders | bare EXE |
| Magikaaaaarp Pong | bare EXE, plays Guitar Hero's track |

Voxide, Celeste and PSXcel never read the disc after boot, so they ride as
bare EXEs and do not care which SDK built them: a `_start` that ignores the
loader's arguments is still a correct `_start`.

Two still to wire up:

- **Cortex Ignition** needs the editor's cook run against a project
- **Half-Life** needs the Half-Life game data and a full asset cook

Neither needs a source change, only a rebuild against the SDK on PSoXide's
`demo-disc-lba-base` branch.

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
