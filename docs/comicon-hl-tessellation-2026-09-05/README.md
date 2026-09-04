# Comicon HL tessellation pressing — 5 September 2026

Image build commit: demo disc `7a898da`. HL source: `7a78459`, merged to main.
SDK runtime remains `8df242b`; the other game pins are unchanged.

The private HL pressing includes the bounded native-triangle warp correction.
The standard pressing is byte-identical to the accepted 4 September build.
Image hashes are in `SHA256SUMS.txt`; binary artifacts remain outside Git in
`/Users/ebonura/Downloads/PSoXide-Comicon-2026-09-05-hl-tessellation`.

## Geometry and timing

See the HL repository's `docs/hl-tessellation-2026-09-04/README.md` for the
103-map source-to-cooked topology audit, Xash3D headless geometry reference,
normal GPU-command error measurement, and rejected refinement experiments.
94.8% of emitted original four-edge source faces retain two triangle equivalents.
The targeted floor region's estimated mean UV error falls 17.4%, from 3.041 to
2.512 cooked texels; one traced residue sample falls from 11.393 to 0.625.
The 95th percentile barely changes. Substantial affine warping remains.

The final demo-checkout standalone HL build completes the entire opening tram
with 8,918 flips and 21.665 FPS, versus 21.658 baseline. The fifth-percentile
five-second window is 9.38 FPS. This is effectively unchanged performance,
not a stable 20 FPS result. These are PSoXide measurements, not console timing.
Representative tram captures and five separate A/B geometry probes were
visually inspected. The final shipping MIPS hazard scan reports zero hazards.
196 host-logic, 156 cooker, and two topology-audit tests passed.

HL was fully recooked and packed in the actual demo submodule checkout. The
cooked files match the tested worktree except `music/tracks.txt`, whose absolute
source paths differ. The embedded EXE SHA-256 is
`88b23dde9c1476e997a104048cc2b0dbc8c558fab11659ccb8bd6780402d1fbc`.
The release receipt verifies clean source and cooked-asset provenance.

## Disc validation

All 35 HL-pressing audio tracks match source bytes. All eight standard-pressing
tracks also match. The HL CUE is byte-identical to the previous release, so
track positions and audio bases remain unchanged: Cortex base 4, HL base 6,
remaining shared base 33. Cortex combat is physical track 6 and menu is track 7.

Two runs of each release-critical chain-load route pass byte-deterministically:
Cortex Ignition, Half-Life, Hardware Tests, and Quake Shareware. The separate
Quake release checker passes both runs, including its relocated runtime read.
All nine additional program routes pass. The logs and audio census accompany
this document; the local release folder contains the sealed receipt and visual
evidence. This evidence-only commit does not change the pressed image.
