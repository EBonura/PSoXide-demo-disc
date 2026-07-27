# PSX demo disc

One CD-R, nine programs, a menu. Boots on a real PS1 (modchipped, same target as
`cortex_ignition`). Target date: 10-13 September 2026.

## Contents

| Slot | Source repo | EXE | Disc data | CD-DA |
| --- | --- | --- | --- | --- |
| Cortex Ignition v1 | PSoXide `editor/projects/cortex_v1` | 1.5 MB | WORLD.PAK + UI.PAK | 1 track (CRT Lab Loop) |
| Half-Life | hl-psx | ~? | pack | 27 tracks |
| Celeste Classic Collection | pico8-psx | ~? | none | none |
| Voxide (Minecraft) | voxide | ~? | none | none |
| Guitar Hero | gh-psx | ~? | none | 1 track (GONCHAROV) |
| PSXcel | psxcel | ~? | none | none |
| Breakout | PSoXide `engine/examples/game-breakout` | 184 KB | none | none |
| Space Invaders | PSoXide `engine/examples/game-invaders` | 193 KB | none | none |
| Magikaaaaarp Pong | PSoXide `engine/examples/game-magikaaaaaarp-pong` | 289 KB | none | shares GONCHAROV |

## Capacity: not the problem

Measured today:

- hl-psx disc as it ships: 480,271,344 bytes = 204,197 sectors = 45.4 min
- gh-psx audio: 40.3 MB (one track, shared with Magikaaaaarp Pong)
- cortex CD-DA source: 12 MB WAV (one loop)
- voxide / psxcel / celeste-collection discs: 1024 sectors each, and almost all of
  that is the fixed playtest-layout padding to LBA 1024, not content

Rough total (estimate, the cortex pack size is not yet cooked): ~240,000 sectors,
~53 min. An 80-min CD-R holds 360,000 sectors. Fits with room to spare, keeping
all 27 Half-Life music tracks. CD-DA track count lands around 30 of the 99 max.

The audio sharing the user asked about (GH + Pong on one GONCHAROV track) falls
out of the layout for free: both games point at the same track index.

## The two real problems

### 1. Chain-loading

A PS1 disc has one `SYSTEM.CNF` and one boot EXE. Every PSoXide game links to the
same load address (`sdk/psoxide.ld`: `LOAD_ADDR = 0x80010000`), so the launcher
cannot simply read a game into RAM: it would overwrite itself mid-copy.

Chosen approach: **relocated loader blob.**

1. Launcher links normally at `0x80010000`. Full RAM for the menu.
2. A second tiny crate (`loader`) links at a high fixed address (`0x801F0000`),
   is built to a raw blob, and is embedded in the launcher as a byte array.
3. On selection: copy the blob up, `FlushCache`, jump to it with (LBA, sector
   count, entry point).
4. The blob reads the target EXE straight off the disc at a build-time-known LBA
   into `0x80010000` (no ISO 9660 parsing, same raw-LBA addressing the rest of
   the disc already uses), then jumps to the entry point.
5. Build-time check: no game's `__bss_end` may reach `0x801F0000`. Cortex, the
   biggest at 1.5 MB of text+data, ends around `0x80190000`, so there is ~400 KB
   of headroom. If a game ever grows past it, the build fails instead of the
   console.

Rejected: BIOS `LoadExec` (A(0x51)). It is two lines of work (`psx-rt/src/bios.rs`
already has the trampoline macro) and it is how retail multi-EXE discs did it, but
it depends on the BIOS CD filesystem, and the emulator's `hle_bios.rs` implements
no file I/O, so it could only ever be tested with a real BIOS image or on console.
The blob loader is testable headless.

### 2. Rebasing each game

Every game currently assumes it owns the disc:

- `psx_pack::cd::WORLD_PACK_DEFAULT_LBA` / the cooked `WORLD_PACK_START_LBA`,
  `UI_PACK_START_LBA` constants: fixed at 1024. On a shared disc each game's packs
  live somewhere else.
- CD-DA: `cdrom::try_play_track(track: u8)` takes an absolute track number.
  Half-Life's track 2 is not track 2 on this disc.

So each game needs two build-time knobs: a base LBA and a CD-DA track base. Both
are one constant each; the work is plumbing them through six repos' build systems
and re-cooking.

## Build shape

```
psx-demo-disc/
  launcher/        menu EXE (PSoXide SDK)
  loader/          high-linked chain-load blob
  tools/mkdisc/    lays out one .bin/.cue: launcher + N game EXEs + N packs + M audio tracks
  Makefile         builds every game from its sibling repo with the right bases, then mkdisc
  dist/
```

`tools/mkdisc` is a sibling of PSoXide's `tools/mkisopsx`, not a fork of it:
`crates/psx-iso`'s `IsoBuilder::add_file` already takes arbitrary files, and
`add_playtest_files` is just one canonical layout built on top of it. A second
layout function alongside it is the small change.

## Progress

Done: chain-loading, whole-image relocation, the CD-DA track base, and seven
of the nine programs (see README). Left: Cortex Ignition and Half-Life, both
of which need their asset cooks run rather than any new mechanism.
