# Repository split validation - 5 September 2026

Build commit: `e14e55c69e0502b1bac3027ed948b6a9301e63cd`. Later documentation and route-test
commits do not change these disc images. Builds use normal release features.

## Selected components

| Component | Repository | Commit |
|---|---|---|
| editor | EBonura/PSoXide-editor | `0f12f4d8f5ebaf2ab3bdf0bd1a7a0469166d7c58` |
| emulator | EBonura/PSoXide-emulator | `fd914c5ed8fbb0b46f8276c5ff7376904b9b444f` |
| quake_build_reference | EBonura/PSoXide | `8df242b353b8a3664c1d2ed20622d692d1349306` |
| sdk | EBonura/PSoXide | `a7bde2b5baeb409061e9458e92f2e0037ba1c7f5` |

## Validation

- SDK host/device checks and six self-contained SDK examples passed.
- SDK triangle produced the same framebuffer hash in the bundled and extracted emulators.
- Editor host workspace: 2,045 tests passed. Engine/runtime contract suite: 213 tests passed.
- SDK, emulator and editor CI passed, including the final example-build safety change.
- Nitro model/arena cookers and Celeste host audio-capture tool compile against split sources.
- All 101 tracked Cargo locks resolve, and remaining legacy standalone pins remain internally consistent.
- Demo carousel, TOC, packer and 66 Python tests passed; label QR codes decode.
- Both normal demo editions packed with component and Quake provenance. The HL source receipt verifies.
- Standard gameplay routes passed for VoXide, NitroXide, PSXcel, GH-PSX, hardware tests and all three Arcade games.
- Celeste collection menu and both Celeste Classic games were separately exercised and visually checked.
- Cortex, Half-Life, hardware tests and Quake each passed two identical chain-load replays on the HL disc.
- The standard Quake replay retains its original expected VRAM/display hashes.
- Branch/load-delay hazard scans pass for every rebuilt guest, both outer loader/launcher and the five nested Arcade executables.
- Cortex also passes the forbidden guest-symbol gate.

The scan found previously unguarded hardware-test and Arcade builds. Their
owning build recipes now disable unsafe LLVM delay-slot filling and run the
scanner before packing. No diagnostic game feature was enabled to address this.

## Audio and images

The standard disc has 8 audio tracks; the HL edition has 35. Every relocated
audio sector, including pregaps, matches its source. Half-Life retains all 27
tracks and exactly the original standalone offsets. On the HL combined disc
those tracks are physical tracks 8 through 34. Shared Arcade/GH track routing
is also verified.

| Edition | BIN SHA-256 | CUE SHA-256 |
|---|---|---|
| Standard | `eef0c5038c51bdf1ecfd4e593b220564c51db677b91efbb0e6781f528507fdbc` | `3dabdc8794b56728feb980b4d4bbaa802c2d3f91ac4f0fba29b32c39b443e4bd` |
| HL | `e7bd7eb9f5ca717bd62918e5604ad319aaf9c7e792c5137a48a1debb5e95966c` | `3be200bd309bc6f25bedfd74b51ba4717c20416d6eafd68b625f9c00e921d160` |

The final artifacts carry `.components.json`, `.quake-provenance.json`, and
(for HL) `.release-receipt.json` sidecars. Hardware burn/CRT acceptance remains
a separate owner-run check; emulator validation does not establish silicon timing.
