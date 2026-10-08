# Changelog

## v0.43

- Public pressing from `release/lineup-v0.43.json`: a fresh build of every program from its repository's main (PSXcel c08096c, Celeste Collection fabaf73, NitroXide fa023bd, VoXide 42366f2, Quake ab671da, Cortex Ignition from PSoXide-editor 1212e009). PSoXide Arcade is the analog-required 0.1.1 build (e8aec23) from the v0.43 private edition, because Arcade main still has the launcher that does not enable analog mode.
- The disc requires an analog DualShock. The launcher locks the pad in analog mode and shows a notice for a digital pad.
- The left analog stick moves the carousel (launcher change f659e44).
- PSXcel moves to the end of the carousel, just before Credits (48e6a33). The public deploy check now demands that order.
- The disc tools (receipts, components, lineup pressing, Quake pins, web delivery, deploy) are one Rust crate, `tools/disc-tools`. The four release gates that are not ported yet stay Python.
- The README no longer describes a shared private edition: the private editions are local hardware verification builds and are never distributed.


## v0.41

- Celeste Collection 0.2.6: complete, ACK-paced controller reads (SDK psx-pad transport fix) aimed at the SCPH-110 analogue-mode credits loop, and both carts on the psx-tick fixed clock. Emulator-tested; console confirmation pending.
- GH-PSX leaves the disc: its card, screenshots, submodule and build and check entries are gone. PSoXide Arcade keeps the Goncharov track.
- Every other program is its v0.40 input, byte for byte, pressed through `release/lineup-v0.41.json`.
- `make celeste-navigation-check` replays the Celeste return paths through the full pressing.

## v0.40

- Cortex Ignition: the Palermo Comicon update (wrong-stance hits only chip, stance swaps protect like a dodge, stance colours on Aletha and the enemies, longer sword reach, attacks turn to the locked target after a dash, cheaper enemy deaths, gap-free combat music loop, camera fixes, a New Game intro).
- NitroXide: the arena and HUD overhaul (team-coloured halves, walls and roof, lit goals, boost pads as light under the floating orbs, goal boxes and arcs, a crowd, a landing hoop under an airborne ball, a scoreboard tab), from nitroxide land/nitro-arena.
- VoXide: Minecraft Java Edition parity (movement, mining, combat, hunger, spawning, day length, fluids, fishing, experience, breeding), an icon-grid inventory, stack moves in chests and furnaces, fuller saves.
- Quake: lighting fitted to each face's whole lightmap, no sky through near walls, error-bounded tessellation, the one-rule world sort, doors, gibs and explosions closer to Quake, and SLICNSE.TXT in both public packages.
- Half-Life, Counter-Strike and Hollow Knight (private pressing only) move to their latest library builds.
- The Half-Life pressing now carries every current project: Counter-Strike 1.6 and Hollow Knight join Half-Life, Quake, Cortex Ignition, VoXide, NitroXide, the Celeste Collection, the Arcade, GH-PSX and PSXcel.
- The hardware test suite leaves the demo disc for a disc of its own, which is what makes the rest fit an 80-minute CD-R.
- Pressed from the builds in the games library, as played, through a lineup file (`release/lineup-v0.40.json`, `make lineup-disc`) that pins every input by hash, source revision and build receipt.

## v0.38

- Emulator component moved to d16168e: the reverb work-address wrap that ran thousands of loop iterations per sample once the SDK parked the work area at the top of SPU RAM is closed-form now, which restores full speed for every game on the current SDK, in the browser build too, and user-supplied firmware loading is back. The editor (c0ae6e3a), every game's component lock and the Quake contract follow the same tuple.

## v0.37

- Celeste Collection updated to 0.2.3: both games hold 60 fps throughout (the audio is rendered while the game waits for VBlank and each frame is drawn by the GPU from a display list), analog stick support, and the PICO-8 synthesiser running on the PS1.

## v0.36

- Quake: fixed the loading-screen hang on original PlayStation hardware. The CD now stays paused while cached map nodes are converted. Verified on console on 8 September; replaces the earlier v0.36 download and browser build.
- Quake: loading now shows the active stage and displays a CD diagnostic code if a sound or level load fails.

- Cortex: enemies now dissolve from the top down after defeat, with rising fragments that fade before the body is removed.
- Cortex: the latest combat audio, stance, dash and movement polish is included in both the browser build and the downloadable disc.

## v0.35

- Cortex: enemies switch between melee and ranged stances as the distance changes, respecting the five-second swap cooldown.
- Cortex: new heavy-enemy attack animations and chest-fired shots, with longer melee contact windows.
- Cortex: combat music starts at a random point and loops; inactive stance health recovers more slowly.

- Cortex: new stance changes burst into polygons and rebuild from feet to head, with a smooth return to Aletha's colours.
- Cortex: dashes leave fading fragments behind a travelling wireframe, then gradually restore the body.
- Cortex: faster sprint response, cleaner stops, shorter evades and a five-second stance cooldown.
- Cortex: better swing sounds, trails and hit timing; reworked heavy-enemy combos and improved ranged aiming.
- Cortex: improved close-space camera handling, clearer module prompts and animated message dismissal.
- Cortex: combat music continues through the inventory, new pickup and entry sounds, and a darker default brightness.

## v0.34

- Celeste Collection follows NitroXide in the carousel.

- Includes the Cortex Ignition and hl-psx gameplay fixes and the reordered carousel from the v0.33 test builds.

## v0.33.1

- Reordered the carousel: Cortex Ignition, Quake, Half-Life, VoXide, NitroXide, Arcade, GH-PSX, PSXcel, Celeste Collection and Hardware Tests. Credits remain last.

## v0.33

- Cortex Ignition: fixed the camera and movement failure triggered by Select, restored a fresh start when choosing New Game, and raised the default music volume.
- hl-psx pressing: restored NPC replies and microwave interaction; corrected seated scientists, missing heads and the overturned cabinet drawing through a body.

## Source 2026.09.05

This source snapshot is tagged `source-2026.09.05`. Download versions are
listed separately below; source cleanup does not replace an already published disc.

- Documented the SDK, engine/editor, emulator and game revision pins used by the disc.
- Added component receipts and instruction-hazard checks before packing.
- Formatted the launcher and disc tools; refreshed standalone build instructions.

## v0.31-split.20260905 | 2026-09-05

Public demo disc published on itch.io.

- Built from the validated split repositories, with Cortex Ignition 0.4b and Quake in the carousel.
- The public disc excludes Half-Life. Arcade and GH-PSX also have separate downloads with CD audio.
