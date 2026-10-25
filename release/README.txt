PSoXide Demo Disc v0.49
=======================

A little of everything. One PlayStation disc.

Choose from PSXcel, PSoXide Arcade, Celeste Classic Collection, NitroXide,
VoXide, Quake Shareware and the Cortex Ignition tech demo. Arcade
contains Breakout, Space Invaders and Magikaaaaarp Pong.

What's new in v0.49
-------------------
- A new menu. The programs stand as game cases on a shelf: left and right
  browse, Cross or Start runs the one in front, Triangle switches between
  English and Italian, Select opens the credits and links (with the QR code),
  L1 and R1 skip the music. Choosing a program opens its case like a real one
  and the disc spins up while the console finds the program on the disc.
- Leave the disc alone for twenty seconds and the menu starts touring the shelf,
  turning each case to show its spine and back cover, with the floor lit by the
  music. Any button brings the menu back.
- The music credit (Mulula VGM Loops, CC BY 4.0) is on the credits screen and
  appears at the foot of the screen while the menu tours.
- The Konami code is gone; every program on the disc is on the shelf.
- Quake is the latest build. The other programs are as in v0.48.

What's new in v0.45
-------------------
- Every program is its newest build, including ones that are not finished.
- Celeste Classic Collection 0.2.8: the pad is read through the shared pad
  code, a neGcon works like a digital pad, and the sound and maths no longer
  use 64-bit integers. The sound is the same as before.
- NitroXide: sound effects go through the shared mixer, placed in stereo.
  The net is always cut at the near plane, frames cost less with the same
  picture, and the dotted line under the goal glow is gone.
- VoXide 0.4.1 has far fewer slow frames: three-frame stalls on a recorded
  play dropped from 131 to 38. A starved chunk stream now waits for a frame
  with room instead of stalling the one it is in.
- Quake: Chthon's lava balls fly at you and light the walls, rockets light
  the walls they pass, the pentagram and quad glow on you, messages are
  centred, the end-of-level camera comes from the map, nailgun frames are
  shown one at a time, and sounds follow you as you move. The image is
  smaller, so it has more memory left.
- PSoXide Arcade, PSXcel and the disc menu are rebuilt on the current SDK.
  They look and play the same. Cortex Ignition is unchanged.

What's new in v0.44
-------------------
- New menu music: six pieces that play in a shuffled order, none twice until
  all six have played. L1 and R1 skip, and the last stop is MUSIC OFF.
  Music: Mulula VGM Loops, Ambient Jungle Vol 1, Vol 2 and Ambient Jungle D&B
  Vol 3, CC BY 4.0, https://mululavgmloops.itch.io/
- All the music is compressed XA audio now, the menu's and each program's
  own, so the disc has a single data track. The browser version starts much
  faster because it only fetches the parts of the disc it is using.
- Celeste Classic Collection 0.2.7 can change brightness: DEFAULT, DARKER and
  BRIGHTER steps in the pause menu and in Settings.
- NitroXide has nine cars and a fixed ramp texture, reads the pads from an
  interrupt, and saves its settings in the background. VoXide 0.4.1 adds a
  LOAD WORLD title, faster TNT chains and a longer view distance. Quake has
  the latest fixes, including the bridge grunt, wall puffs and the splash,
  and needs less memory.
- The disc menu no longer does 64-bit arithmetic for the song clock.

What's new in v0.43
-------------------
- This disc needs an analog DualShock. The menu switches the pad into analog
  mode and keeps it there, and asks for a DualShock when it finds a digital
  pad. The left stick now moves the carousel as well as the D-pad.
- PSXcel moved to the end of the carousel, just before the credits.
- NitroXide, VoXide and Quake are fresh builds. VoXide: you respawn in the
  overworld after dying in the Inferno or the Void, holding L2 repeats block
  placement, holding jump hops again on landing, the tutorial toast no longer
  covers the dragon's boss bar, and saving asks before it formats a blank
  memory card. NitroXide: the now-playing plate stays off the goal banner,
  fine pitch tile edges no longer snap to a saturated coarse neighbour, and it
  asks before formatting a blank memory card. Quake is rebuilt on the
  current shared engine.
- Cortex Ignition has the newly approved animations and the two-hit R1 combo.

What's new in v0.41
-------------------
- Celeste Classic Collection 0.2.6: controller reads wait for the pad to
  acknowledge each byte, and a read that fails partway is thrown away and
  retried. This targets a DualShock in analog mode opening the Celeste
  credits by itself, with Cross not closing them. Tested in the emulator;
  not yet confirmed on a console. Both Celeste games also run one logic
  update per VBlank, so a slow frame catches up instead of slowing the game.
- GH-PSX is no longer on the disc.
- Everything else is the same build as v0.40.

What's new in v0.40
-------------------
- Cortex Ignition: the Palermo Comicon update. Hitting an enemy in the wrong
  stance only chips it, swapping stance protects you like a dodge, Aletha and
  the enemies show their stance colours, the sword reaches further, attacks
  turn to your locked target even straight after a dash, enemy deaths no
  longer drag the frame rate down, combat music loops without a gap, the
  camera stays out of Aletha, and New Game opens with a short intro.
- NitroXide: the stadium look. Team-coloured halves, walls and roof, lit
  goals, boost pads as pools of light under floating orbs, goal boxes and arcs
  on the pitch, a crowd behind the cage, a landing hoop under an airborne ball
  and a scoreboard that tucks into a tab while you play.
- VoXide: tuned to Minecraft Java Edition's numbers (movement, mining, combat,
  hunger, mob spawning, day length and more), an icon-grid inventory, chests
  and furnaces that move whole stacks, and saves that keep far more. 0.2.1 turns faster on the right stick and
  lets a mob under the crosshair take the hit before the block behind it.
- Quake Shareware: better lighting, no sky showing through walls up close,
  less texture swimming on big surfaces, and doors, secret doors, gibs and
  explosions closer to the original. Clear water by default, without the
  tint, white lines and canal flashes seen on a real console.
- Celeste Classic Collection 0.2.5: the launcher follows one set of rules,
  and leaving a game returns to the cover menu with that game selected.
- PSoXide Arcade 0.1.1: sound effects cooked with the shared SDK encoder.
- The cards run in a new order, and the hardware test suite now has a disc
  of its own.

The collection is still in development. Celeste 2 can miss occasional frame
deadlines; this release does not claim locked 60 fps throughout either game.
Only the programs listed above are on this disc.

How to play
-----------
Extract the download and keep the CUE and BIN together with their original
names. Open the CUE in your PlayStation emulator and set up your controller.
For an original PlayStation, use a setup that can load homebrew discs.
When writing a CD-R, write the CUE file.

Left/right: choose a game. Cross: play. Up/down: English/Italian description.
L1/R1: change menu music. Follow each game's prompts for its own controls.
To switch games, return to the disc menu where supported or restart the disc.

Credits
-------
Menu and NitroXide music: Mulula VGM Loops, Ambient Jungle Vol 1, Vol 2 and
Ambient Jungle D&B Vol 3, CC BY 4.0, https://mululavgmloops.itch.io/

VoXide music: Mulula VGM Loops, Simple Piano Music Vol 1 (Aranha, Melodia,
Nulo, Pianeta, Side Label, Talher), CC BY 4.0, https://mululavgmloops.itch.io/

Quake music: Mulula VGM Loops, Eerie Ambiences Vol 1, CC BY 4.0,
https://mululavgmloops.itch.io/

NitroXide crowd sounds: Gregor Quendel, Free Crowd Cheering Sounds, CC BY 4.0,
https://opengameart.org/
NitroXide tyre squeal loop: Tom Haigh, CC BY 3.0,
https://opengameart.org/content/car-tire-squeal-skid-loop
NitroXide cars: Pavel 3D, Low Poly City Vehicles Pack, CC BY 4.0,
https://pavel-3d.itch.io/low-poly-city-vehicles-pack

CC BY 4.0: https://creativecommons.org/licenses/by/4.0/
CC BY 3.0: https://creativecommons.org/licenses/by/3.0/

Cortex Ignition music (menu and combat tracks) is by Carmelo Miceli.

Celeste Classic Collection is an unofficial, non-commercial fan port of the
PICO-8 games Celeste and Celeste 2: Lani's Trek by Maddy Thorson and Noel
Berry, with Celeste 2 music by Lena Raine. PICO-8 is by Lexaloffle Games.
All rights to the original games remain with their creators.

Quake Shareware data is id Software's, distributed under the shareware
licence agreement in SLICNSE.TXT, which must accompany it.

Goncharov, used in Magikaaaaarp Pong, is by magikAAAAArp:
https://www.youtube.com/@magikAAAAArp/videos
Included with the band's permission. Individual games retain their own credits.

Bonnie Studios: https://bonnie-studios.itch.io
