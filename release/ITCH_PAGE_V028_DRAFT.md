# PSoXide Demo Disc v0.28 itch.io draft

> Internal publishing draft. Do not paste or upload this until the Quake
> shareware redistribution decision in `docs/quake-shareware.md` is resolved.
> The Half-Life pressing is private and must not be offered on itch.io.

## Short description

Ten native PlayStation programs on one bootable disc, including a three-game
arcade collection, with a beat-synchronised carousel and real CD audio.

## Page copy

One disc, ten outer programs, all of them native PlayStation software. Boot it
and a music-driven carousel appears over a spinning globe and starfield; choose
a program and the disc chain-loads it like the classic shareware demo discs.

No console or emulator handy? The public build can also run in your browser on
the [PSoXide page](https://bonnie-studios.itch.io/psoxide), CD audio included.
The browser player and this download must show the same version before this
copy goes live.

### On the disc

- **Cortex Ignition**, the current new-engine tech demo.
- **VoXide**, a Minecraft-style sandbox.
- **NitroXide**, rocket-car football against the CPU or in split-screen.
- **Celeste Classic Collection**, both PICO-8 Celeste games as native PS1 code.
- **PSXcel**, a working spreadsheet for the original PlayStation.
- **GH-PSX**, an early Guitar Hero-style prototype.
- **PSoXide Arcade**, a nested collection containing Breakout, Space Invaders,
  and Magikaaaaarp Pong.
- **Hardware Tests**, the real-console conformance suite.
- **Quake Shareware**, the complete Quake 1.06 shareware Episode 1 port.

The standard pressing keeps both work-in-progress Cortex entries behind the
classic Konami code. Quake remains visible in the normal carousel. Credits use
the same two-column presentation as the game cards, with project links on the
left and a scannable QR code on the right.

### The music

The menu plays four tracks by
[Just Music](https://www.youtube.com/@Just-Music-Beats) as CD audio, with a
beat-synchronised visualizer. Used with the artist's permission for this
non-commercial release. **Goncharov** is by
[magikAAAAArp](https://www.youtube.com/@magikAAAAArp/videos), used with the
band's permission. PSoXide Arcade owns that track for Magikaaaaarp Pong, and
GH-PSX intentionally borrows the same physical copy.

### How to run it

The v0.28 standard image is a flat BIN/CUE pair: 223.1 MiB, 99,450 sectors,
and eight CD audio tracks. Boot the `.cue` in PSoXide, DuckStation, or
PCSX-Redux, or burn it to a CD-R and run it on a modchipped console.

### Technical details

- Everything is built with the open-source
  [PSoXide](https://github.com/EBonura/PSoXide) Rust SDK.
- The launcher relocates whole disc images and hands each guest its new data
  and CD-track bases, so the same binaries work standalone and on the combined
  disc.
- PSoXide Arcade owns one physical CDDA track shared with GH-PSX, avoiding a
  duplicate audio payload.
- v0.28's standard and private Half-Life pressings were independently
  chain-load tested twice with deterministic Quake results. Only the standard
  pressing is a possible public artifact.

## Download metadata after publication approval

- File: `psoxide-demo-disc-psx.zip`
- User version: `v0.28`
- Contents: `PSoXide Demo Disc.bin`, `PSoXide Demo Disc.cue`, `README.txt`
- Expected uncompressed BIN: 233,906,400 bytes
- The private Half-Life BIN/CUE must not be uploaded.

## Gallery order

1. `assets/readme/v028-carousel.png`
2. `assets/readme/arcade-selector.png`
3. `assets/readme/quake-chainloaded.png`
4. `assets/readme/cortex-current.png`
5. Fresh v0.28 Credits/QR capture.
6. Fresh v0.28 globe-only loading capture.

## Companion-page cross-link replacement

Use this on the VoXide, NitroXide, PSXcel, and Celeste Collection itch pages
instead of a hard-coded program count:

> This also ships on the [PSoXide Demo
> Disc](https://bonnie-studios.itch.io/psoxide-demo-disc). The same disc runs
> [in your browser](https://bonnie-studios.itch.io/psoxide), with no emulator
> setup required.

## PSoXide browser-page lead replacement

Publish this only after the HTML player actually embeds the same v0.28
standard disc:

> The emulator above runs in your browser. Press Run and PSoXide Demo Disc
> v0.28 streams in: ten native PlayStation programs on one bootable image,
> including PSoXide Arcade's three-game collection, with menu music playing
> directly from the CD image. It boots from the initial data and streams the
> remaining audio behind the carousel.
>
> Prefer real hardware? The matching standard BIN/CUE is available from the
> PSoXide Demo Disc page, ready for a PS1 emulator or a modchipped console.
