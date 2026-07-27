# Menu music

Four tracks by **Just Music**, used with the artist's permission, cycled by the
menu in this order:

| Track | Length | Disc track |
| --- | --- | --- |
| KNUCKLE DUST | 2:33 | 32 |
| RUSTED HAMMER | 4:02 | 33 |
| CHAINSAW HEART | 2:35 | 34 |
| NIGHT CRAWLER | 1:28 | 35 |

Source: <https://www.youtube.com/watch?v=v-lBPoVexEc>

The credit shown on screen is short enough to fit one line of the menu's font;
this file is the long form. The `.cdda` files here are the source MP3s decoded
to raw 44.1 kHz 16-bit stereo PCM and padded to a whole number of 2352-byte
sectors, which is what `mkdisc --menu-cdda` expects:

```sh
ffmpeg -i track.mp3 -ar 44100 -ac 2 -f s16le -acodec pcm_s16le track.cdda
```

They came from a 48 kHz 320 kbps MP3, so the disc is a lossy source in a
lossless container. If the artist can send WAVs, re-cut from those.
