# Quake shareware on the demo disc

Quake 1.06 shareware Episode 1 is on every pressing, Public and Private. The
owner approved public non-commercial distribution of the canonical Quake 1.06
shareware payload on 2026-08-25. The Public pressing is the only one that is
published; the private pressings (Half-Life, Hollow Knight and the others) are
never uploaded, and `disc-tools public-only` fails on them.

## Where the image comes from

Quake is a row of the lineup (`QUAKE SHAREWARE` in `release/lineup-<version>.json`):
a cue and BIN under `RELEASE_INPUTS/<version>/quake/`, their sha256, the
`quake-psx` revision it was built from, and the build receipt (the shipping
provenance sidecar `quake-psx.provenance.json` beside the cue). `disc-tools lineup
prepare` refuses a pressing whose cue or BIN hash moved, and `disc-tools lineup
receipt` proves that the `QUAKE SHAREWARE` entry of the pressed disc carries the
lineup's bytes (apart from the three BCD MSF address bytes `mkdisc` relocates).
The image streams `WORLD.PAK`, so it rides as a whole image and relies on the
same caller-provided LBA offset as VoXide and NitroXide. It owns no CD-DA track.

Quake's downloaded `quake106.zip`, `PAK0.PAK`, cooked `WORLD.PAK` and the
generated disc images stay untracked; this repository commits none of them.

## Updating it

Build the new Quake image in the `quake-psx` repository, copy the cue, BIN and
sidecar to `RELEASE_INPUTS/<new version>/quake/` (an APFS clone is enough),
mirror the source revision under `RELEASE_INPUTS/src/quake-psx.git`, and edit the
lineup row (`cue_sha256`, `bin_sha256`, `version`, `source`, `build`, `receipt`).
`make lineup-check LINEUP=...` verifies the files and hashes; the gates then
replay the chain-load twice byte-identical (`make gates-public`, `gates-private`).

## History

Until 2026-10-29 the Makefile also rebuilt every program from the submodule pins
and verified Quake against six pinned values (`QUAKE_EXPECTED_*`,
`make quake-verify`, `make quake-repin`). That path carried stale pins and was
retired with `make disc`; the lineup row above replaces it.
