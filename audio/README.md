# Menu music

Four tracks by **Just Music** (<https://www.youtube.com/@Just-Music-Beats>),
used with the artist's written permission, cycled by the menu in this order:

| Track | Length | Disc track |
| --- | --- | --- |
| KNUCKLE DUST | 3:10 | 32 |
| RUSTED HAMMER | 4:04 | 33 |
| CHAINSAW HEART | 3:06 | 34 |
| NIGHT CRAWLER | 2:40 | 35 |

Source: WAV masters sent by the artist on 2026-07-29 (WeTransfer), which
replace the earlier YouTube MP3 rips; these cuts run longer than the YouTube
versions. The `.cdda` files here are those WAVs (48 kHz 16-bit stereo)
resampled to raw 44.1 kHz 16-bit stereo PCM and zero-padded to a whole number
of 2352-byte sectors, which is what `mkdisc --menu-cdda` expects:

```sh
ffmpeg -i track.wav -ar 44100 -ac 2 -f s16le -acodec pcm_s16le track.cdda
python3 -c 'import os;p="track.cdda";r=os.path.getsize(p)%2352;r and open(p,"ab").write(b"\0"*(2352-r))'
```

Re-measure the beat grid after any re-cut (`python3 tools/beatgrid.py
audio/*.cdda`) and refresh `MENU_BEATS` in the Makefile; the WAV masters'
first-beat offsets differ from the MP3 rips'.

## Permission terms (email, 2026-07-30)

- Tracks covered: the four above, for this project only.
- Non-commercial use only, no resale.
- Credit as: Just Music - YouTube channel `@Just-Music-Beats`. The menu shows
  the playing track's name from the disc table and the credit line
  `Just Music - YouTube @Just-Music-Beats` along the bottom, so both halves
  of the attribution are on screen at once; this file is the long form.
- Scope: the demo disc **without the Half-Life port**, and the YouTube video
  of the disc running on real hardware. The layout that carries hl-psx is for
  local hardware verification only and must never be distributed (this also
  matches the Half-Life assets themselves being undistributable). Any copy
  that leaves the house, including the artist's, must be cut without hl-psx.
