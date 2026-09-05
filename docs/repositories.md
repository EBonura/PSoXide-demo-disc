# Repository ownership after the split

The original PSoXide repository is now the SDK. Cortex and the engine stay
with the editor. Each owning repository retains its source history; old
monorepo commits remain addressable so existing standalone game pins keep working.

| Repository | Owns | Dependencies | Visibility |
|---|---|---|---|
| [PSoXide](https://github.com/EBonura/PSoXide) | SDK, linker, disc packer, shared formats and low-level PS1 APIs | None of the editor or emulator | Public |
| [PSoXide-editor](https://github.com/EBonura/PSoXide-editor) | Editor, engine, asset cookers, Cortex Ignition and hardware tests | Pinned SDK and emulator libraries | Public |
| [PSoXide-emulator](https://github.com/EBonura/PSoXide-emulator) | Standalone desktop/web emulator, CPU/GPU/SPU and debugging | Pinned SDK/shared contracts | Public |
| [PSoXide-demo-disc](https://github.com/EBonura/PSoXide-demo-disc) | Carousel, loader, disc layout, integration pins and release validation | SDK, editor/engine/Cortex, emulator and every game below | Private |
| [voxide](https://github.com/EBonura/voxide) | Voxel demo | SDK; separate emulator for testing | Public |
| [nitroxide](https://github.com/EBonura/nitroxide) | Rocket-car soccer demo | SDK, engine, texture/model cookers | Public |
| [psxcel](https://github.com/EBonura/psxcel) | Spreadsheet demo | SDK and engine | Public |
| [celeste-collection-psx](https://github.com/EBonura/celeste-collection-psx) | Celeste Classic collection | SDK; emulator libraries for host audio capture | Public |
| [gh-psx](https://github.com/EBonura/gh-psx) | Rhythm game | SDK and engine; borrows Arcade audio on the combined disc | Private |
| [psoxide-arcade](https://github.com/EBonura/psoxide-arcade) | Breakout, Invaders and Magikarp Pong collection | SDK, engine and its own nested loader | Private |
| [quake-psx](https://github.com/EBonura/quake-psx) | Quake port and shareware disc | SDK, engine, audio cooker; separately validated artifact | Public |
| [hl-psx](https://github.com/EBonura/hl-psx) | Half-Life port | SDK, engine, audio cooker and external game data | Private |

## Integration rules

`release-components.json` records the selected SDK, editor and emulator
commits. The editor component lock must agree with that tuple, and the
emulator records its own SDK dependency. Imported files are verified against
content receipts before a release build.

The demo builds its own launcher/loader/packer directly against the SDK.
Ordinary game builds receive an explicit bootstrapped editor/engine source
override; this does not compile the editor UI into any game. Cortex uses
that editor source to cook its project. The separate emulator executable
performs release validation.

VoXide and NitroXide standalone bootstraps have migrated to the split inputs.
Other standalone pins remain reproducible historical versions; advancing them
is an independent runtime upgrade. Their combined-disc builds already receive
the explicitly selected split source tuple. Celeste audio capture remains a
host-only emulator library dependency.

Quake deliberately retains its validated standalone image and original
build-source reference under `games/PSoXide`. That historical checkout is
a provenance fixture, not the source of the current launcher or ordinary games.
Its source/CUE/BIN/EXE hashes and loader/audio relocation checks remain enforced.

`make components` bootstraps exact component sources. `make verify-components`
checks them offline. `make sdk-on-main` checks selected revisions against each
owning repository main. A final `.components.json` receipt records all game
pins, nested source receipts, toolchain identity, emulator hash and image hashes.

Half-Life remains opt-in (`HL=1`), and its original game data and 27 audio
tracks stay external. Both editions preserve the existing normal release
feature sets and separate Quake and HL provenance checks.

Original-console validation remains separate from emulator validation.
