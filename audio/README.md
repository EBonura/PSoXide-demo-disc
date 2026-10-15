# Menu music

Music: Mulula VGM Loops, Ambient Jungle Vol 1, Vol 2 and Ambient Jungle D&B
Vol 3, CC BY 4.0, <https://mululavgmloops.itch.io/>

The launcher plays six pieces made by chaining loops from those three packs
(seed 20261015, 4 s crossfades; each piece draws from one pack):

| Song | Title on screen | Loops, in order |
| --- | --- | --- |
| 0 | JUNGLE 1 A | Air, Samanbaia, Frog, Return, Jungle (Vol 1) |
| 1 | JUNGLE 1 B | Jungle, Bubbles, Manana, Board, Tomate (Vol 1) |
| 2 | JUNGLE 2 A | Crassula, Monstera, Brunfelsia, Rhipsalis, Dracaena (Vol 2) |
| 3 | JUNGLE 2 B | Dracaena, Dypsis, Echeveria, Zamioculca (Vol 2) |
| 4 | JUNGLE DNB A | Pera, Semente, Jumbo, Jaca, Lona (D&B Vol 3) |
| 5 | JUNGLE DNB B | Lona, Corda, Sapo, Mamba (D&B Vol 3) |

The pieces were loudness-matched (-15 LUFS integrated) before encoding. The
license (CC BY 4.0) asks for the author, the work, the license and a link; the
launcher's Credits and Links card carries them, and so do the repository
README, the CHANGELOG and the release notes. Keep those in step.

## What is in `launcher/`

- `MUSIC1.XA`, `MUSIC2.XA`: the six songs, `psx-audio-cook xa-pack`, 37.8 kHz
  stereo, single speed, four to a file (two interleave slots are empty).
- `songs.rs`: the generated table (file, channel, milliseconds per song) the
  launcher compiles in.
- `music.json`: the pack manifest.
- `menu.spectrum`: the level-meter animation of every song (`mkdisc analyse`).

The source WAVs are not in the repository (about 190 MB at 37.8 kHz); rebuild
with `make music`, see the README. The previous menu music (Just Music, under
a permission scoped to the old pressings) is gone from the disc and from the
tree; it stays in git history.
