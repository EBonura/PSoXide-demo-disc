# Changelog

## v0.51

- Re-pressed from `release/lineup-v0.51.json` (Public) and `release/lineup-v0.51-private.json` (Private). NitroXide is 0.5.1 (main 9e20819, tag v0.5.1) on both: the boost flame and smoke in the car's colour, pad pickups that burst in the arena, a calmer kickoff fly-over that starts with the tile sweep, split screen drawn at single-player detail, HUD numbers centred in their panels (see its `CHANGELOG.md`). Hollow Knight on the Private pressing stays at the library build e15a09ee (hk-psx main 9a0bca6), still the newest. Every other program is the v0.50 input byte for byte. The `games/nitroxide` gitlink moves to 9e20819.
- Every input of both lineups is a hash-gated copy under `release-inputs/v0.51/<game>/` (Public) and `release-inputs/v0.51-private/<game>/` (Private) in the campaign folder, and every source repository a bare mirror under `release-inputs/src/`, so a lineup no longer points into a lane folder that a cleanup can remove.
- The browser player pin moves to the scrolling home grid (wheel, trackpad and finger), the swipeable welcome pages and the dense phone debug panel (emulator 4ebdf4bc).

## v0.50

- The loader never shows a partial or blank frame at the hand-off. Its display setup keeps the display start on the buffer the launcher was showing (the launcher leaves that buffer's row in a mailbox word at the end of the blob's reservation, `chainload-layout::SHOWN_BUFFER_MAILBOX`; the link asserts the blob stops short of it), paints its first frame into the other buffer, and only then flips to it and enables the display. The first loader frame is the launch's next step, not a copy.
- VoXide moves to voxide main 12895132 (the library build 82461d86, with the new Minecraft-style loading screen) in both v0.49 lineups and the `games/voxide` pin. Its splash line already reads ALSO TRY CELESTE!, so Public and Private share the one build.

- The Shelf's small text is crisp. The description lines, footer hints, header and version labels, credits and back covers were set in a 9 px anti-aliased Jost, whose soft one-pixel strokes smear when a phone or browser scales the 320x240 picture up (the description was reported "very blurry"). They are now Tiny5 at its 8 px grid size, baked as whole pixels (`tools/menu-art bake <michroma.ttf> <tiny5.ttf> <shots> <out>`; Jost is no longer baked). The EN and IT descriptions of every program still fit three lines at `DESC_W`.

- The hand-off is one shot. The launch animation no longer ends in a flash and a black-out (72 frames, was 96) and the chain loader no longer draws its own globe, starfield, LOADING text and progress bar. It draws the launch's disc from the same code (`loader/src/disc_art.rs`, included by the launcher and the blob), carrying the spin-up and the glow on from the frame the launch ended on, and keeps it turning until the program takes over, with "LOADING" fading in under it in the Shelf's face and colour. The first loader frame is pixel-identical to the launch's last (the launcher's swap shows a frame a VBlank late, so the last frame on screen is `HAND_OFF - 1`). The loader animates from `Presentation::tick` between two buffers (repainting only the disc's square and the label's strip, at half the frame rate during the CPU-bound checksum pass), dims the scene over the last 12 frames of that pass, and uses the launcher's own two buffers (rows 0 and 240; the old second buffer at row 256 overwrote the menu atlas's palette row). `carousel` is no longer a dependency of the loader. The flash and black-out were dead time, so START to the program's first tick on the emulator is shorter or equal: Arcade 143 to 121 ticks, Hollow Knight 411 to 402, Half-Life 136 to 112, Ocarina of Time 602 to 602.
- Re-pressed from `release/lineup-v0.50.json` (Public) and `release/lineup-v0.50-private.json` (Private). NitroXide is 0.5.0 (main 88b2baf) on both: Rocket League air control and flips, nine cars, six tracks of music inside the data track, stereo sound effects, a camera fly-over and a big 3, 2, 1, GO! at every kickoff, golden-goal overtime, moving menus, a rebuilt pause menu, more in-match feedback, the boost-bar loading screen and a Save Settings that no longer freezes the game (see its `CHANGELOG.md`). The Private pressing also takes the latest Hollow Knight (hk-psx main 9a0bca6, library build e15a09ee of 2026-10-26 18:49). Every other program is the v0.49 input byte for byte. The `games/nitroxide` and `games/hk-psx` gitlinks move to the same revisions.

## v0.49

- The menu is the Shelf. The crimson carousel (pills on a tilted ring under a ball of balls) is gone; the launcher now shows the programs as jewel cases standing on a floor in the PSoXide home shell's look, one case in focus with its name, its own version, its description and a start button under the row (`launcher/src/shelf.rs`, `common.rs`, `gfx.rs`, `text.rs`, faces and glyphs baked by `tools/menu-art` into `launcher/assets`). It reads the same disc table as before, so one launcher serves every pressing. Browsing, language (Triangle, EN/IT), the L1/R1 song walk through the six songs and MUSIC OFF, the level meter (eight bars, from the disc's analysis), the song title, the header's version (the real `DISC_VERSION`), the DualShock-in-analog notice, the stick as a d-pad, the browse and confirm blips and the L2 + R2 CD debug overlay (it was Select) all carry over. The ball, the starfield, the pills, the description panel and screenshot box, and the per-program screenshot slideshow are gone: each case wears one cover, baked into the launcher (`launcher/src/cover.rs`; Ocarina of Time's is new, Half-Life's uses the replaced screenshot).
- Choosing a program opens the case like a real jewel case (the cover swings on the left hinge through the GTE, the disc lifts out and spins up into a flash, 96 frames; since Unreleased it spins on into the loader instead). The launcher stops the song and aims the drive at the program's first sector on frame 0 (`launcher/src/chain.rs`), so the seek happens during the animation. The loader's loading screen takes the menu's colours.
- Credits are a screen of their own, opened with Select (Circle or Select to leave): the platform marks with the project links, the music attribution in full (Mulula VGM Loops, the three works, CC BY 4.0, the link) and the QR code. There is no credits case any more. After 20 seconds without a button the menu goes into its attract mode (`launcher/src/attract.rs`): a camera tour along the shelf that turns each case to show its spine and its back cover (blurb and two screenshots), over a floor and light pools that follow the level meter, with a sweep of light across the logo on the beat; the music credit takes turns with "press any button" at the foot of the screen, and any button ends it at once. The back-cover pictures are baked by `menu-art backs` (the shots sit in two 8bpp VRAM pages below the covers; the prototype's single page put the last programs' rows under the palette rows at y 480 and garbled them).
- No runtime reveal. The Konami code is gone from the launcher, `mkdisc --gate` is gone, and the launcher shows every entry the disc table lists. Which programs a pressing carries is decided when the disc is built (nothing is gated today: Cortex Ignition has been on both pressings since v0.43). The only build-time switch inside the launcher is the `private` cargo feature, which compiles in the covers and names of WipEout, Hollow Knight, Counter-Strike, Half-Life and Ocarina of Time; the Makefile sets it for any pressing that carries one, and `disc-tools public-only` stays clean because a Public launcher holds none of them. The Public and Private labels are not drawn anywhere on the disc.
- The shelf does not wrap (the carousel did), so the headless routes (`disc-tools programs`, `chainloads`, `celeste`) walk RIGHT from the first case.
- The v0.48 release line is merged into main: the lineups of v0.45 to v0.48, the chain-load layout crate (blob at 0x801C8000), the loader's flush through the SDK's DMA-safe gate, the quiet CD hand-off, `mkdisc --tail-filler`, and the component tuple SDK 649c7ed8, editor 3cb1839a, emulator dddd2814.
- Re-pressed from `release/lineup-v0.49.json` (Public) and `release/lineup-v0.49-private.json` (Private). Public is the v0.48 Public lineup with Quake on main c6412e2 (the library build of 2026-10-25). Private adds WipEout (0e5a735), Hollow Knight (feat/hk-ram-2026-10-24 7226e94), Counter-Strike (main 4613a46), Half-Life (main e03e9bc, no DIAG overlay) and Ocarina of Time (main cae4681), all as the library builds of 2026-10-22 to 2026-10-25. The disc program shots (`SHOTS.BIN`) are still pressed but the launcher no longer reads them.

- The browser player pin moves to the build whose Reset restarts the game in place, whose Library pick of the loaded disc restarts it without a download, whose Back resumes the game, and whose keyboard, touchscreen and gamepads present an analog DualShock from the first poll (no F9 step for the demo disc).
- The browser package of an XA disc is the compact delivery: `demo-disc.pxi` (index, 30 KB for v0.48) and `demo-disc.pxw` (deflated 32-sector chunks, 178 MiB against the 318 MiB image) instead of the raw image and a 330 KB header sidecar. `deploy-public` builds `webdisc` from the emulator checkout (the `web_player` pin must contain `psx-webdisc`, 4d32b426), packs, and has the packer read every file back and compare each sector with the image. `public-only` cannot scan deflated chunks, so it ties the index to a scanned image by game-image hash and sector count and checks the payload sizes; check the web folder together with the download folder. `web-delivery` on its own still writes the raw lazy image.

- The chain loader flushes the instruction cache through the SDK's DMA-safe gate and waits for DMA to be idle before it rewrites the DMA control register. A flush (or an isolated cache) while a channel moves data makes the next instruction fetch return a word of that traffic (Quake, 2026-10-22); the loader carried its own copy of the flush sequence with no such wait. `psx-chainloader` now calls `psx_rt::cache::flush_instruction_cache` (psx-rt without its runtime, so the blob keeps its own entry point and panic handler), and `quiesce` waits for each channel with the SDK's bounded `wait_or_abort`. The component tuple moves to SDK 649c7ed8 (the commit that adds psx-rt's `runtime` feature) and editor 3cb1839a.
- The chain-load blob moves from 0x801F0000 to 0x801C8000, into the gap between the largest bottom-loaded program (Ocarina of Time ends at 0x801B6000) and the Half-Life intro (0x801E0000 to 0x801F2800, which must sit above its own game and so could not fit under the old address; mkdisc refused it). The address and the stack span now live in one crate, `chainload-layout`: the loader's linker script is generated from it (`loader/build.rs`, template `loader/loader.ld`), the launcher copies the blob to it and `mkdisc` reads the same numbers, so the three can no longer drift. `mkdisc` checks each program's payload rounded up to whole sectors against the blob and the stack (`psx_chainloader::check_payload`, the rule the loader applies on the console), prints every program's extent with its margin before the sector budget, and says which span a refused program overlaps instead of "no chain-loadable PSX-EXE". The editor pin moves to the commit that adds `try_load_reserving` and `run_reserving`.
- The chain loader hands the CD drive to the program it loads quiet. A Pause answers twice, and on the console the second answer comes 1.2 to 3.9 million system clocks after the first, so the old loader (a plain Pause after the last chunk, then the checksum and the jump) could leave an INT2 to land in the first moments of the next program. The editor pin (9ae15ba7, SDK 3cf22179) ends the last read with a proved stop and acknowledges the controller before the jump. `tools/chainload-probe` is a payload that checks for it (entry state, an 8 million clock watch with no command sent, an immediate read, and a control that a plain stop() does leave an INT2) and `make chainload-probe` presses it behind the launcher and chain-loads it on an emulator with the silicon Pause model: the old loader fails it (a stray INT2 114,512 clocks into the watch), the new one passes. The launcher's own reads and its song stop need no change: its song stop already waits for the second response, and the loader's first command cancels anything pending.
- The component tuple moves to SDK 3cf22179 and editor 9ae15ba7 (emulator unchanged at dddd2814).

## v0.45

- Re-pressed from `release/lineup-v0.45.json` (Public) and `release/lineup-v0.45-private.json` (Private), so every component is its newest library build of 2026-10-16 and 2026-10-17, finished or not. Public: Arcade 0.1.1 (a1745c6, SDK b5365b18 repin), Celeste Collection 0.2.8 (main a061d2c, the pressed release disc; the library still holds 0.2.7), NitroXide 0.4.1 (main 9b70726), VoXide 0.4.1 (b6d2af3, the frame retune), Quake (main 68a641d), Cortex Ignition (Graybox Reach, the v0.44 XA build; the library holds only the CD-DA twin) and PSXcel (bbbaf10, SDK repin). Private adds WipEout (main e6195fc), Hollow Knight (main a7b4b15, Hornet milestone), Ocarina of Time (main cff5b0d, Deku Tree enemies), Counter-Strike (d607325, panic=immediate-abort) and Half-Life (1b4dbd4: pass 4, the startup movie and the intro chain-load fix for a relocated disc).
- What changed per game since v0.44. Celeste: shared SIO0 pad engine (a neGcon reads like a digital pad), PICO-8 synth and fixed-point maths off 64-bit integers with identical sound. NitroXide: sound effects on the shared psx-sfx mixer in stereo, net always cut at the near plane, frame-cost cuts with the same picture, glow edge fix. VoXide: render and streaming retune, three-vblank frames 131 to 38 on the recorded play. Quake: lava balls fly at the player, rocket and lava lights, pentagram and quad glow, centred messages, map intermission camera, nailgun frames one at a time, respatialised one-shots, panic=immediate-abort (12.8 KB smaller). Arcade, PSXcel, WipEout and the menu: SDK repin, same behaviour. Hollow Knight: all 64-bit arithmetic moved onto psx_math::Wide, pad polled through scene loads, late frames 7,885 to 7,521 over the 55 routes. Half-Life: decals and light styles, startup movie, pruned PGO profile and layout. Counter-Strike: off software 64-bit divides, SDK and editor repin.
- The component tuple moves to SDK b5365b18, editor b743d5e3 and emulator dddd2814 (editor main, whose lock pins the SPU main volume fix). The emulator vendors its SDK, so the nested-pin check and the bootstrap skip a vendored emulator (`components.rs`, with a test); before this the editor's and the emulator's SDK pins could not both agree with one disc SDK.
- The `games/` gitlinks for nitroxide, voxide, psxcel, psoxide-arcade, pico8-psx, hk-psx, hl-psx and cs-psx move to the revisions above.

## v0.44

- Re-pressed from `release/lineup-v0.44.json` (Public) and `release/lineup-v0.44-private.json` (Private), so every component is the newest library install of 2026-10-16: Celeste Collection 0.2.7 (the brightness stepper, DEFAULT, DARKER n, BRIGHTER n, the palette address fix and the card-save repin; chore/celeste-0.2.7-2026-10-16), NitroXide 0.4.1 (863765d: nine cars, ramp texture fix, SFX pass, SDK pad engine, background settings save), VoXide 0.4.1 (25cb82d: LOAD WORLD title, faster TNT chains, 20 block view), Quake (integ/quake-2026-10-16 e05f786: RAM pass, weapons, monsters and survival fixes). Private adds WipEout with the wall collision option (f08be05c), Hollow Knight (main dd8958f), Counter-Strike (feat/cs-pending 2fb98e8) and Half-Life (integ/hl-focus b5a58b2). Arcade, Cortex (Graybox Reach) and PSXcel keep their previous inputs, byte for byte. The launcher carries the 32-bit song clock (fix/no64-2026-10-16).
- The `games/` gitlinks for pico8-psx, nitroxide, hl-psx and cs-psx move to the same revisions; the Quake pins in the Makefile describe the e05f786 Quake. The hk-psx pin goes through the identity-rewritten archive twin first (af88629), then to main.
- The browser player is the renamed emulator (`psoxide-emulator`): PSoXide-emulator main 33b58b6 plus the numbered-parts loader that main lacks (`feat/web-image-parts-on-main-2026-10-16` c45e7fb, the 1fe08f3 commit cherry-picked), so the 333 MB image is served as two files under itch.io's 200 MB cap. `web_player` in `release-components.json` points at c45e7fb, and the Makefile names the emulator package from the checked-out emulator, so it builds either pin.
- `disc-tools public-only` fails when a staged public file or disc image (the download BIN, the web parts, the Celeste package) names Half-Life, Hollow Knight, Counter-Strike or WipEout anywhere: table of contents, ISO listing, every byte, receipts. The public README no longer names Half-Life.
- The carousel-order guard (`tools/disc-tools/tests/carousel_order.rs`) joins from dd/private-order-2026-10-08, and the public lineup now lists its programs in carousel order.
- The pressing is tagged v0.44, so the launcher header and the file names read v0.44 and `deploy-public` accepts the version.

## Unreleased (folded into v0.44)

- All XA, no CD-DA anywhere: the Public pressing is one data track. Arcade (Goncharov), Cortex (Graybox Reach, two songs), Quake (eight ambient loops, each stored three times so a repeat has no seek gap), NitroXide (six songs) and VoXide (four pieces) each carry their own music as XA inside their own image, in addition to the launcher's six menu songs. A song that two programs would share is stored twice for now (interim design; a shared pool is a later step). The Private edition (Half-Life, Counter-Strike, Hollow Knight and WipEout on top) is data-track only too; the hardware tests are left off it because the all-XA set no longer fits an 80-minute blank with them.
- The Cortex slot holds the Graybox Reach project: the default 0.4b project does not link on editor main (.data over the image region), so it has no XA build yet.
- `release/lineup-v0.44-test.json` and the private lineup name the XA inputs (hash gated). The Quake pins in the Makefile describe the XA Quake (0050aaf). The `games/` gitlinks for psoxide-arcade, nitroxide and voxide point at their XA commits.
- `deploy-public` requires a lone data track: a cue with any CD-DA track is refused. The browser package is the lazy image (raw image or numbered parts under 200 MB, plus the header sidecar); the `web_player` pin is the multi-part build b309c31.
- `make release-web` no longer passes CD-DA track titles.
- Gates: the Magikarp route now requires an XA read (Setfilter then ReadS) in the CD log instead of a CD-DA Play, and its marker is `magikarp: xa ok`; the Cortex chain-load edge floor drops from 110 to 100 permille because Graybox Reach measures 105; the VoXide program gate stops at 1.2G steps instead of 450M (the 0.4.0 splash fades through black, and the stop landed on a black frame on the 11-card private pressing).

- The launcher's menu music is XA, not CD-DA. Six songs (Mulula VGM Loops, Ambient Jungle Vol 1, Vol 2 and Ambient Jungle D&B Vol 3, CC BY 4.0, https://mululavgmloops.itch.io/) replace the four Just Music songs: they ride in the launcher's ISO as `MUSIC1.XA` and `MUSIC2.XA`, play through `psx_io::cd::xa::Player` as a shuffled playlist (no repeats until all six have played; L1/R1 and MUSIC OFF as before), and the level meter is timed off XA song time. The credits card shows the attribution. `audio/*.cdda` and `--menu-cdda` are gone; `mkdisc analyse` writes the level-meter file and `make music` re-authors the songs. The launcher alone took the pressing from 7 CD-DA tracks to 3; the program migrations below remove the rest.
- The component tuple moves to SDK 4e97cf3, editor 0e9d552, emulator ca42a30 (the coherent set from 2026-10-08) so the launcher has the XA player. The launcher follows the SDK renames (`Deadzone::is_outside`, the dropped Gouraud line wrapper).
- No tempo grid is pressed for the new songs (loops of different tempos chained together have no single tempo); `disc-tools beatgrid` now reads WAV files at any sample rate and takes `--bpm LOW:HIGH`.
- `mkdisc` no longer enforces "menu songs are the first CD-DA tracks"; the borrowing convention for NitroXide and Quake is gone from the layout.
- The browser player's emulator pin (`web_player` in `release-components.json`) moves to aad98fb: the demo disc boots on its data track alone with music decoded on a time budget, and a disc whose audio is XA is read lazily over HTTP Range requests.
- `disc-tools web-delivery` (and so `deploy-public`) delivers a disc with no CD-DA tracks as the raw image plus a gzipped sector-header sidecar and a manifest `image` line, instead of a gzipped data track. An image over itch.io's 200 MB file limit is split into numbered parts, which needs the emulator branch `feat/web-lazy-image-parts-2026-10-15` or later. A disc with CD-DA tracks is delivered exactly as before (gzip data track and FLAC pieces, byte for byte).
- `make release-web` reads the image from `dist/` and uploads every file of the delivery.

## v0.43

- Public pressing from `release/lineup-v0.43.json`: a fresh build of every program from its repository's main (PSXcel c08096c, Celeste Collection fabaf73, NitroXide fa023bd, VoXide 42366f2, Quake ab671da, Cortex Ignition from PSoXide-editor 1212e009). PSoXide Arcade is the analog-required 0.1.1 build (e8aec23) from the v0.43 private edition, because Arcade main still has the launcher that does not enable analog mode.
- The disc requires an analog DualShock. The launcher locks the pad in analog mode and shows a notice for a digital pad.
- The left analog stick moves the carousel (launcher change f659e44).
- PSXcel moves to the end of the carousel, just before Credits (48e6a33). The public deploy check now demands that order.
- The disc tools (receipts, components, lineup pressing, Quake pins, web delivery, deploy) are one Rust crate, `tools/disc-tools`. The three release gates that are not ported yet (chain-loads, program boots, Celeste navigation) stay Python.
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
