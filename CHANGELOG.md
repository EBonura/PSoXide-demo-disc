# Changelog

## Unreleased

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
