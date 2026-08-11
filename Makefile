# PSoXide demo disc.
#
# `make disc` builds the chain-load blob, the launcher, every program, and lays
# them out into the PSoXide game library as one disc. `make check` runs the
# host tests.
#
# Two of the eleven cannot be rebuilt from a fresh clone: Cortex Ignition's
# project lives under editor/projects/, which PSoXide gitignores, and hl-psx
# keeps its cooked assets and music outside git too. Both are staged from the
# sibling working trees. See PLAN.md.

.PHONY: help disc disc-only quake-disc quake-disc-only quake-verify quake-headless-check _quake-headless-check programs loader launcher examples mkdisc check relocation-check clean

ROOT       := $(CURDIR)
PSOXIDE    := $(ROOT)/games/PSoXide
BUILD      := $(ROOT)/build
OUT        := $(BUILD)/mipsel-sony-psx/release
EXAMPLES   := $(PSOXIDE)/build/examples/mipsel-sony-psx/release
MKDISC     := $(ROOT)/tools/mkdisc/target/release/mkdisc

# Menu music, used with the artist's permission. The credit ships on the disc
# and stays on screen the whole time the track plays; keep the two together.
MENU_CDDA   := $(ROOT)/audio/knuckle-dust.cdda $(ROOT)/audio/rusted-hammer.cdda \
               $(ROOT)/audio/chainsaw-heart.cdda $(ROOT)/audio/night-crawler.cdda
MENU_CREDIT := Just Music - YouTube @Just-Music-Beats
# Tempo and first-beat offset per track, in the same order, measured by
# tools/beatgrid.py. The menu pulses on these; a guessed tempo drifts.
MENU_BEATS  := 176010:359 175000:168 173860:150 174360:325
# Shown as "now playing", in the same order as MENU_CDDA. Written out rather
# than looped: make's foreach splits on whitespace, which takes the titles
# apart at their spaces.

# Two pressings. The public download must not carry Half-Life, and the menu
# music permission is scoped to the disc without it, so HL is opt-in:
#   make disc        -> "PSoXide Demo Disc"     (public, no Half-Life)
#   make disc HL=1   -> "PSoXide Demo Disc HL"  (Palermo Comicon pressing)
# The names differ so the two bins cannot be mistaken for each other.
HL ?=
# HL=0 means off, not "0 is a non-empty string, so on".
override HL := $(filter-out 0,$(HL))

# Quake shareware is a separate local/test pressing. It is intentionally not
# part of either the public or Half-Life variants. The source revision and both
# input hashes are checked before layout, and the combined image gets a
# machine-readable provenance receipt beside it.
#
# The default paths name the validated convergence checkout. A caller can use
# another checkout or cue, but must also state the revision and hashes expected
# from it. The verifier fails closed if any one of them differs.
QUAKE ?=
override QUAKE := $(filter-out 0,$(QUAKE))
QUAKE_SRC ?= $(abspath $(ROOT)/../quake-psx-episode1-next-gate)
QUAKE_CUE ?= $(QUAKE_SRC)/dist/quake-psx.cue
QUAKE_EXPECTED_REV ?= 396b861b3f8d293bbaf61775252df684ffb6aed4
QUAKE_EXPECTED_CUE_SHA256 ?= 5fa78b12b506d4190246e230183e1eebd677f201ff982a584bff10d88ee2594c
QUAKE_EXPECTED_BIN_SHA256 ?= 039ac161d6e3a8b6c9fcca2887f2e1e5ee2153f2772ca8001c302428d919dab4
QUAKE_VERSION := q$(shell printf '%.7s' '$(QUAKE_EXPECTED_REV)')
FRONTEND ?= $(PSOXIDE)/target/release/frontend

ifneq ($(QUAKE),)
ifneq ($(HL),)
$(error QUAKE and HL are mutually exclusive local/test pressings)
endif
endif

# The disc lands in PSoXide's game library, laid out the way every other
# homebrew entry there is: <library>/<Name>/<Name>.{bin,cue}.
ifneq ($(QUAKE),)
DISC_NAME ?= PSoXide Demo Disc Quake Shareware
else ifneq ($(HL),)
DISC_NAME ?= PSoXide Demo Disc HL
else
DISC_NAME ?= PSoXide Demo Disc
endif
PSOXIDE_LIB ?= $(HOME)/Downloads/ps1 games
DIST ?= $(PSOXIDE_LIB)/$(DISC_NAME)

PSX_TARGET  := mipsel-sony-psx
PSX_FLAGS   := --release --target $(PSX_TARGET) -Zbuild-std=core -Zbuild-std-features=compiler-builtins-mem
LOADER_EXE  := $(OUT)/loader.exe
LAUNCHER_EXE := $(OUT)/launcher.exe

# Sibling game repos, as submodules.
GAMES    := $(ROOT)/games
VOXIDE   := $(GAMES)/voxide/dist/voxide.cue
PSXCEL   := $(GAMES)/psxcel/game/target/$(PSX_TARGET)/release/psxcel.exe
CELESTE  := $(GAMES)/pico8-psx/games/celeste-collection/target/$(PSX_TARGET)/release/celeste-collection.exe
GHPSX    := $(GAMES)/gh-psx/dist/gh-psx.cue
HLPSX    := $(GAMES)/hl-psx/dist/hl-psx.cue
CORTEX   := $(PSOXIDE)/editor/projects/cortex_v1/baked/cortex_v1.cue
HWTESTS  := $(EXAMPLES)/hardware-tests.cue

NITROXIDE_SRC ?= $(GAMES)/nitroxide
NITROXIDE_BUILD := $(BUILD)/nitroxide
NITROXIDE       := $(NITROXIDE_BUILD)/NitroXide/NitroXide.cue

help:
	@echo "make disc             - build everything into \"$(DIST)\" (public pressing, no Half-Life)"
	@echo "make disc HL=1        - the Palermo Comicon pressing, with Half-Life"
	@echo "make quake-disc       - local/test pressing with pinned Quake shareware"
	@echo "make quake-disc-only  - relay the local/test Quake pressing from built inputs"
	@echo "make quake-headless-check - build, launch, and replay the Quake pressing twice"
	@echo "make disc-only        - relay out the disc without rebuilding the programs"
	@echo "make check            - host tests (disc-toc, mkdisc)"
	@echo "make relocation-check - disc that proves a relocated game still finds its data"
	@echo "make clean            - drop build/ (the disc in the library is left alone)"

loader:
	cd loader && CARGO_TARGET_DIR=$(BUILD) \
		RUSTFLAGS="-Clink-arg=-Tloader.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

# Each program's own version, read from the source that declares it rather than
# restated here. The disc could always say which pressing it was, but not which
# build of anything it carried: eleven programs come from eight repositories at
# eight different moments, so the header's tag answered a question nobody was
# asking when one game looked wrong.
#
# The manifest is the source, which took a correction to be true. VoXide's tag
# and its on-screen string both said 0.1.9 while its Cargo.toml still carried
# the 0.1.0 it was created with, and the Celeste collection had shipped 0.1.1
# without the manifest hearing about it -- so the first disc to print versions
# announced both games as older than they were. Both manifests are right now
# and VoXide's on-screen string derives from its own, so they cannot drift
# again.
#
# A game genuinely at 0.1.0 is one that has never been released, which is an
# honest thing for the carousel to say. Bumping is a one-line change in the
# game's own manifest and the disc follows.
cargo_version = $(shell awk -F'"' '/^version/{print $$2; exit}' $(1) 2>/dev/null)
V_VOXIDE    := $(call cargo_version,$(GAMES)/voxide/game/Cargo.toml)
V_NITROXIDE := $(call cargo_version,$(NITROXIDE_SRC)/game/Cargo.toml)
V_PSXCEL    := $(call cargo_version,$(GAMES)/psxcel/game/Cargo.toml)
V_CELESTE   := $(call cargo_version,$(GAMES)/pico8-psx/games/celeste-collection/Cargo.toml)
V_GHPSX     := $(call cargo_version,$(GAMES)/gh-psx/game/Cargo.toml)
V_HLPSX     := $(call cargo_version,$(GAMES)/hl-psx/game/Cargo.toml)
# The hardware suite already versions itself on screen; take that same string so
# the carousel and the suite header cannot disagree.
V_HWTESTS   := $(shell awk -F'"' '/SUITE_VERSION: &str/{print $$2; exit}' $(PSOXIDE)/engine/examples/hardware-tests/src/main.rs 2>/dev/null | sed 's/HWTEST v//')
V_BREAKOUT  := $(call cargo_version,$(PSOXIDE)/engine/examples/game-breakout/Cargo.toml)
V_INVADERS  := $(call cargo_version,$(PSOXIDE)/engine/examples/game-invaders/Cargo.toml)
V_MAGIPONG  := $(call cargo_version,$(PSOXIDE)/engine/examples/game-magikaaaaaarp-pong/Cargo.toml)
# Cortex Ignition has no version to read. It is an authored editor project
# rather than a Cargo package, and project.ron declares a name and no version,
# so the disc simply does not claim one for it. Give it a version there and add
# a --version-of line here.

# Which pressing this is, drawn in the launcher's header. Tag a burn
# (`git tag v0.3 && make disc`) and the disc identifies itself on camera.
# Only v* tags name pressings; the rolling `web-disc` release tag that feeds
# the browser emulator would otherwise leak into every version string.
DISC_VERSION := $(shell git -C $(ROOT) describe --tags --match 'v*' --always --dirty 2>/dev/null)

# The launcher embeds the blob, so it always rebuilds after it.
launcher: loader
	cd launcher && CARGO_TARGET_DIR=$(BUILD) LOADER_BLOB=$(LOADER_EXE) DISC_VERSION=$(DISC_VERSION) \
		RUSTFLAGS="-Clink-arg=-T$(PSOXIDE)/sdk/psoxide.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

examples:
	$(MAKE) -C $(PSOXIDE) game-breakout game-invaders game-magikaaaaaarp-pong
	$(MAKE) -C $(PSOXIDE) hardware-tests-disc

# PSXcel and the Celeste collection never read the disc after boot, so they
# ride as bare EXEs and do not care which SDK they were built against.
# NitroXide streams its arena atlas and voxide streams its sfx bank from
# WORLD.PAK at boot, so both ship whole images and ride the same disc_base
# relocation as hello-pack.
# gh-psx plays CD-DA, so it ships its whole image and needs the SDK with
# psx_io::disc_base. Every game reaches that SDK through psoxide-link now, and
# PSOXIDE_FROM below puts them all on this submodule's copy of it.
# PSOXIDE_FROM on every game that carries its own pin. Each of them records a
# rev for its standalone build, and those revs drift -- three of them sat eight
# commits behind a measured SPU fix and nothing said so. A disc built from
# whatever each game happened to pin would press several different SDKs, so the
# pin is overridden here and all eleven programs come off the submodule.
# hl-psx's --psoxide below is the same idea under an older spelling.
programs: examples
	$(MAKE) -C $(GAMES)/voxide disc PSOXIDE_FROM=$(PSOXIDE)
	$(MAKE) -C $(NITROXIDE_SRC) disc PSOXIDE_FROM=$(PSOXIDE) GAMES_DIR=$(NITROXIDE_BUILD)
	$(MAKE) -C $(GAMES)/psxcel build PSOXIDE_FROM=$(PSOXIDE)
	$(MAKE) -C $(GAMES)/pico8-psx collection PSOXIDE_FROM=$(PSOXIDE)
	$(MAKE) -C $(GAMES)/gh-psx disc PSOXIDE_FROM=$(PSOXIDE)
	@$(MAKE) cortex-if-stale
ifneq ($(HL),)
	cd $(GAMES)/hl-psx && cargo run --release -- disc --psoxide $(PSOXIDE)
endif

# Bake Cortex Ignition only when its project has actually changed.
#
# The bake shells out to the PSoXide frontend, which is built with the
# editor feature by default -- so re-baking compiles the whole host
# editor, including crates that have nothing to do with the disc. The
# project itself changes rarely (it is authored content, not code), so
# every other build was paying for a rebuild that produced identical
# bytes, and coupling the disc to whatever state the editor happened to
# be in. Now the bake runs only if a project file is newer than the
# baked cue, and `make disc CORTEX_FORCE=1` overrides. find needs -L and
# a trailing slash: cortex_v1 is a symlink into the sibling PSoXide
# checkout, and find will not descend one otherwise -- which silently
# made the check answer "unchanged" no matter what.
#
# The proper fix is to split the frontend's `editor` feature so the
# authoring CLI does not drag the GUI in with it. That is a bigger job;
# this removes the coupling in the meantime.
CORTEX_PROJECT := $(PSOXIDE)/editor/projects/cortex_v1
CORTEX_FORCE   ?=

.PHONY: cortex-if-stale
cortex-if-stale:
	@if [ -n "$(CORTEX_FORCE)" ] || [ ! -f "$(CORTEX)" ] || [ -n "$$(find -L "$(CORTEX_PROJECT)/" -type f -newer "$(CORTEX)" -not -path '*/baked/*' -print -quit 2>/dev/null)" ]; then \
		echo "cortex: project changed (or forced) -- baking"; \
		$(MAKE) -C $(PSOXIDE) cortex-ignition-v1-project-disc; \
	else \
		echo "cortex: project unchanged -- reusing $(CORTEX)"; \
	fi

mkdisc:
	cd tools/mkdisc && cargo build --release

# Cortex Ignition is pressed on the public disc but held off the carousel
# until the Konami code reveals it: its own release comes later, and this
# lets friends test ahead of it. The Comicon pressing shows everything.
ifeq ($(HL),)
GATE_ARGS = --gate "CORTEX IGNITION"
endif

# The three HALF-LIFE arguments travel together: an image without its version
# and description would press, but announce itself wrong.
ifneq ($(HL),)
HL_ARGS = --image "HALF-LIFE=$(HLPSX)" \
	--shot "HALF-LIFE=$(SHOTS_OUT)/halflife.shot" \
	--version-of "HALF-LIFE=$(V_HLPSX)" \
	--describe "HALF-LIFE=A from-scratch PlayStation port of Half-Life. The full campaign has been converted and much of the game works, but it is not yet playable from start to finish.|Half-Life portato su PlayStation da zero. L'intera campagna e stata convertita e gran parte del gioco funziona, ma non e ancora giocabile dall'inizio alla fine."
endif

ifneq ($(QUAKE),)
QUAKE_ARGS = --image "QUAKE SHAREWARE=$(QUAKE_CUE)" \
	--version-of "QUAKE SHAREWARE=$(QUAKE_VERSION)" \
	--describe "QUAKE SHAREWARE=Quake 1.06 shareware Episode 1 on the original PlayStation. This local test checkpoint cooks all Episode 1 maps and streams them from the embedded Quake disc image; runtime work is still in progress.|Quake 1.06 shareware Episodio 1 sulla PlayStation originale. Questo checkpoint di test locale converte tutte le mappe e le carica dal disco Quake incorporato; il runtime e ancora in sviluppo."
QUAKE_PREREQS = quake-verify
endif

disc: launcher programs mkdisc
	$(MAKE) disc-only

quake-disc:
	$(MAKE) disc QUAKE=1

quake-disc-only:
	$(MAKE) disc-only QUAKE=1

quake-headless-check:
	$(MAKE) quake-disc-only
	$(MAKE) _quake-headless-check QUAKE=1

_quake-headless-check:
	python3 tools/check_quake_headless.py \
		--frontend "$(FRONTEND)" \
		--cue "$(DIST)/$(DISC_NAME).cue" \
		--receipt "$(DIST)/$(DISC_NAME).quake-provenance.json"

quake-verify:
	python3 tools/quake_disc.py verify \
		--source "$(QUAKE_SRC)" \
		--cue "$(QUAKE_CUE)" \
		--expected-revision "$(QUAKE_EXPECTED_REV)" \
		--expected-cue-sha256 "$(QUAKE_EXPECTED_CUE_SHA256)" \
		--expected-bin-sha256 "$(QUAKE_EXPECTED_BIN_SHA256)"

# Menu backdrops: the same in-game captures the itch pages use, cooked from
# assets/shots into the 8bpp CLUT blobs the launcher uploads to VRAM.
# Cooked at build time (needs PIL, like the other tools) so the pressed
# pixels always come from the PNGs actually in the repo.
SHOTS_SRC := $(ROOT)/assets/shots
SHOTS_OUT := $(BUILD)/shots
SHOT_NAMES := cortex cortex2 voxide-day voxide-night nitroxide-boost \
              nitroxide-aerial nitroxide-goal celeste celeste2 psxcel-chart \
              psxcel-editing ghpsx ghpsx2 breakout breakout2 invaders \
              invaders2 pong pong2 hwtests hwtests2 halflife
SHOT_FILES := $(foreach n,$(SHOT_NAMES),$(SHOTS_OUT)/$(n).shot)

$(SHOTS_OUT)/%.shot: $(SHOTS_SRC)/%.png tools/cook-shots.py
	@mkdir -p "$(SHOTS_OUT)"
	python3 tools/cook-shots.py "$<" "$@"

# Just the layout, for when nothing but the text or the audio changed. Also
# the one place the mkdisc invocation lives, so it cannot drift from what
# `make disc` builds.
disc-only: mkdisc $(SHOT_FILES) $(QUAKE_PREREQS)
	@mkdir -p "$(DIST)"
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out "$(DIST)/$(DISC_NAME).bin" --volume PSXDEMO \
		--image "CORTEX IGNITION=$(CORTEX)" \
		$(GATE_ARGS) \
		$(HL_ARGS) \
		--image "VOXIDE=$(VOXIDE)" \
		--image "NITROXIDE=$(NITROXIDE)" \
		--game "CELESTE COLLECTION=$(CELESTE)" \
		--game "PSXCEL=$(PSXCEL)" \
		--image "GH-PSX=$(GHPSX)" \
		--game "BREAKOUT=$(EXAMPLES)/game-breakout.exe" \
		--game "SPACE INVADERS=$(EXAMPLES)/game-invaders.exe" \
		--game "MAGIKAAAAARP PONG=$(EXAMPLES)/game-magikaaaaaarp-pong.exe" \
		--image "HARDWARE TESTS=$(HWTESTS)" \
		$(QUAKE_ARGS) \
		$(foreach t,$(MENU_CDDA),--menu-cdda "$(t)") \
		$(foreach b,$(MENU_BEATS),--menu-beat $(b)) \
		--menu-title "KNUCKLE DUST" --menu-title "RUSTED HAMMER" \
		--menu-title "CHAINSAW HEART" --menu-title "NIGHT CRAWLER" \
		--credit "$(MENU_CREDIT)" \
		--shot "CORTEX IGNITION=$(SHOTS_OUT)/cortex.shot" \
		--shot "CORTEX IGNITION=$(SHOTS_OUT)/cortex2.shot" \
		--shot "VOXIDE=$(SHOTS_OUT)/voxide-day.shot" \
		--shot "VOXIDE=$(SHOTS_OUT)/voxide-night.shot" \
		--shot "NITROXIDE=$(SHOTS_OUT)/nitroxide-boost.shot" \
		--shot "NITROXIDE=$(SHOTS_OUT)/nitroxide-aerial.shot" \
		--shot "NITROXIDE=$(SHOTS_OUT)/nitroxide-goal.shot" \
		--shot "CELESTE COLLECTION=$(SHOTS_OUT)/celeste.shot" \
		--shot "CELESTE COLLECTION=$(SHOTS_OUT)/celeste2.shot" \
		--shot "PSXCEL=$(SHOTS_OUT)/psxcel-chart.shot" \
		--shot "PSXCEL=$(SHOTS_OUT)/psxcel-editing.shot" \
		--shot "GH-PSX=$(SHOTS_OUT)/ghpsx.shot" \
		--shot "GH-PSX=$(SHOTS_OUT)/ghpsx2.shot" \
		--shot "BREAKOUT=$(SHOTS_OUT)/breakout.shot" \
		--shot "BREAKOUT=$(SHOTS_OUT)/breakout2.shot" \
		--shot "SPACE INVADERS=$(SHOTS_OUT)/invaders.shot" \
		--shot "SPACE INVADERS=$(SHOTS_OUT)/invaders2.shot" \
		--shot "MAGIKAAAAARP PONG=$(SHOTS_OUT)/pong.shot" \
		--shot "MAGIKAAAAARP PONG=$(SHOTS_OUT)/pong2.shot" \
		--shot "HARDWARE TESTS=$(SHOTS_OUT)/hwtests.shot" \
		--shot "HARDWARE TESTS=$(SHOTS_OUT)/hwtests2.shot" \
		--share-cdda "MAGIKAAAAARP PONG=GH-PSX" \
		--version-of "VOXIDE=$(V_VOXIDE)" \
		--version-of "NITROXIDE=$(V_NITROXIDE)" \
		--version-of "PSXCEL=$(V_PSXCEL)" \
		--version-of "CELESTE COLLECTION=$(V_CELESTE)" \
		--version-of "GH-PSX=$(V_GHPSX)" \
		--version-of "HARDWARE TESTS=$(V_HWTESTS)" \
		--version-of "BREAKOUT=$(V_BREAKOUT)" \
		--version-of "SPACE INVADERS=$(V_INVADERS)" \
		--version-of "MAGIKAAAAARP PONG=$(V_MAGIPONG)" \
		--describe "CORTEX IGNITION=An original souls-like, built from scratch for the PlayStation. The current build is a PSoXide engine tech demo rather than a complete game, with streamed rooms, combat and animation.|Un souls-like originale, creato da zero per PlayStation. La versione attuale e una tech demo del motore PSoXide, non un gioco completo, con stanze in streaming, combattimento e animazioni." \
		--describe "VOXIDE=A Minecraft clone built for the original PlayStation. This is an early playable build: world generation, mining, crafting and survival work, but much of the game is still unfinished.|Un clone di Minecraft per la prima PlayStation. Prima versione giocabile: generazione del mondo, scavo, crafting e sopravvivenza funzionano, ma gran parte del gioco e ancora incompleta." \
		--describe "NITROXIDE=A Rocket League clone built for the original PlayStation. Play against the CPU or a friend in split screen: drive, boost, jump, dodge and score, with music off the disc.|Un clone di Rocket League per la prima PlayStation. Gioca contro la CPU o in due a schermo diviso: guida, boost, salti, dodge e gol, con la musica del disco." \
		--describe "CELESTE COLLECTION=Both Celeste Classic games, rebuilt as native PlayStation software with no emulation. The collection is complete: both games and their launcher fit in less than half a megabyte.|I due Celeste Classic riscritti come software nativo PlayStation, senza emulazione. La raccolta e completa: entrambi i giochi e il menu stanno in meno di mezzo megabyte." \
		--describe "PSXCEL=A working Microsoft Excel clone for the original PlayStation, controlled with a joypad. This build is fully functional, with formulas, charts, themes and memory-card saves.|Un clone funzionante di Microsoft Excel per PlayStation, controllato col joypad. Questa versione e completa e include formule, grafici, temi e salvataggi su memory card." \
		--describe "GH-PSX=A Guitar Hero-style rhythm game for the original PlayStation. This is a bare-bones, one-song prototype: the full loop works, but sustains, star power and polish are still missing.|Un gioco in stile Guitar Hero per la prima PlayStation. E un prototipo essenziale con una sola canzone: il ciclo completo funziona, ma mancano note lunghe, star power e rifiniture." \
		--describe "BREAKOUT=A Breakout clone and compact example of the engine used across this disc. This one is complete and fully playable, with input, collision, sound, scoring and a full game loop.|Un clone di Breakout e un esempio compatto del motore usato in questo disco. Il gioco e completo, con input, collisioni, audio, punti e un ciclo di gioco completo." \
		--describe "SPACE INVADERS=A Space Invaders clone and a second engine example. This one is also complete and fully playable, with formations, shields, scoring and enemy fire.|Un clone di Space Invaders e un secondo esempio del motore. Anche questo e completo e giocabile, con formazioni, scudi, punti e fuoco nemico." \
		--describe "MAGIKAAAAARP PONG=Pong with a live CD-audio visualizer. The game is complete and playable: music streams from the disc while pre-analysed frequency bands drive the bars in sync.|Pong con un visualizzatore audio dal vivo. Il gioco e completo: la musica arriva dal CD mentre le frequenze analizzate in anticipo muovono le barre a tempo." \
		--describe "HARDWARE TESTS=A hardware test suite, not a game. The current suite is working and ready to use, displaying real PlayStation measurements as photo-ready codes for checking emulator accuracy.|Una suite di test hardware, non un gioco. E funzionante e pronta all'uso: mostra le misure della vera PlayStation come codici da fotografare per verificare la precisione degli emulatori." \

	$(if $(QUAKE),python3 tools/quake_disc.py receipt \
		--source "$(QUAKE_SRC)" \
		--cue "$(QUAKE_CUE)" \
		--expected-revision "$(QUAKE_EXPECTED_REV)" \
		--expected-cue-sha256 "$(QUAKE_EXPECTED_CUE_SHA256)" \
		--expected-bin-sha256 "$(QUAKE_EXPECTED_BIN_SHA256)" \
		--demo-cue "$(DIST)/$(DISC_NAME).cue" \
		--demo-bin "$(DIST)/$(DISC_NAME).bin" \
		--out "$(DIST)/$(DISC_NAME).quake-provenance.json")


# Keep the browser emulator's copy current. PSoXide's Pages deploy stages
# these files from the rolling `web-disc` release next to the wasm, and the
# frontend streams them on demand; the cue's FILE line is rewritten to the
# stable asset name. Run after pressing a public disc worth shipping -- the
# next PSoXide deploy picks it up.
#
# The release lives on the PSoXide repo, not this one: this repo is private,
# and the Pages workflow's own token can only read releases on the repo it
# runs in.
WEB_DISC_REPO := EBonura/PSoXide
.PHONY: release-web
release-web:
	@test -z "$(HL)" || { echo "release-web: the HL pressing is never distributed"; exit 1; }
	@test -z "$(QUAKE)" || { echo "release-web: Quake shareware needs separate legal and release approval"; exit 1; }
	$(MAKE) disc
	@rm -rf "$(BUILD)/web" && mkdir -p "$(BUILD)/web"
	cp "$(DIST)/$(DISC_NAME).bin" "$(BUILD)/web/demo-disc.bin"
	sed 's/^FILE .*/FILE "demo-disc.bin" BINARY/' "$(DIST)/$(DISC_NAME).cue" > "$(BUILD)/web/demo-disc.cue"
	python3 tools/web-delivery.py "$(BUILD)/web/demo-disc.cue" "$(BUILD)/web/demo-disc.bin" "$(BUILD)/web" \
		"KNUCKLE DUST" "RUSTED HAMMER" "CHAINSAW HEART" "NIGHT CRAWLER" \
		"CORTEX IGNITION" "GH-PSX" "HARDWARE TESTS"
	gh release view web-disc --repo $(WEB_DISC_REPO) >/dev/null 2>&1 || gh release create web-disc \
		--repo $(WEB_DISC_REPO) \
		--title "PSoXide Demo Disc (disc image)" \
		--notes "The public pressing as a raw disc image plus the browser emulator's split delivery."
	gh release upload web-disc --repo $(WEB_DISC_REPO) "$(BUILD)/web/demo-disc.bin" "$(BUILD)/web/demo-disc.cue" \
		"$(BUILD)/web/web-manifest.txt" "$(BUILD)/web/demo-data.bin.gz" $(BUILD)/web/track-*.flac --clobber

# Push the public pressing to itch.io from this machine: the ~200 MB bin is
# too big for GitHub, so CI cannot carry it. Needs butler on PATH and a
# one-time `butler login`. The HL pressing is never distributed -- the music
# permission is scoped to the disc without it.
.PHONY: itch
itch:
	@test -z "$(HL)" || { echo "itch: the HL pressing is never distributed"; exit 1; }
	@test -z "$(QUAKE)" || { echo "itch: Quake shareware needs separate legal and release approval"; exit 1; }
	@command -v butler >/dev/null || { echo "itch: install butler and run 'butler login' first"; exit 1; }
	$(MAKE) disc
	@rm -rf "$(BUILD)/itch" && mkdir -p "$(BUILD)/itch"
	cp "$(DIST)/$(DISC_NAME).bin" "$(DIST)/$(DISC_NAME).cue" release/README.txt "$(BUILD)/itch/"
	butler push --userversion "$(DISC_VERSION)" "$(BUILD)/itch" bonnie-studios/psoxide-demo-disc:psx

check: sdk-coherence check-locks
	cd carousel && cargo test
	cd disc-toc && cargo test
	cd tools/mkdisc && cargo test
	python3 -m unittest discover -s tools -p 'test_quake_disc.py'

# Every program on this disc has to be built against one SDK.
#
# It was not always so. The games carrying their own pin drifted, and on
# 2026-08-03 three of them were eight commits behind a measured SPU fix with
# nothing to say so -- the disc pressed several SDKs and looked fine. `programs`
# passes PSOXIDE_FROM to stop that; this checks it actually took, by reading the
# marker psoxide-link leaves in each hydrated tree.
#
# Only games that have been hydrated are checked: a tree nobody has built yet
# has no marker, and demanding one would fail a fresh clone for no reason.
# Every tracked lock resolves, and every PSoXide pin names the revision its
# lockfile actually resolved.
#
# sdk-coherence answers a different question: it proves the HYDRATED trees agree
# after `make programs`. A 2026-08-03 audit found 21 of 56 tracked locks stale
# while sdk-coherence was green, because a fresh clone of one game honouring its
# own manifest is not something a marker in .psoxide can speak to.
.PHONY: check-locks
check-locks:
	@./tools/check-locks.sh

.PHONY: sdk-coherence
sdk-coherence:
	@expected="local:$(PSOXIDE)"; bad=0; seen=0; \
	for m in $(GAMES)/*/.psoxide/.psoxide-source; do \
		[ -f "$$m" ] || continue; \
		seen=$$((seen+1)); \
		got=$$(cat "$$m"); \
		name=$$(basename $$(dirname $$(dirname "$$m"))); \
		if [ "$$got" != "$$expected" ]; then \
			echo "sdk-coherence: $$name is on $$got, not $$expected"; \
			bad=1; \
		fi; \
	done; \
	if [ $$bad -ne 0 ]; then \
		echo "sdk-coherence: run 'make programs' to put every game on this tree"; \
		exit 1; \
	fi; \
	echo "sdk-coherence: $$seen game(s) on $(PSOXIDE)"

# hello-pack streams WORLD.PAK off the disc and paints ALL PASS or a failure
# list, which makes it the end-to-end test for the relocation machinery: its
# image lands 200-odd sectors in, every LBA it was cooked with is wrong by that
# much, and psx_io::disc_base has to make up the difference. Run the result
# with the emulator and read the banner.
relocation-check: launcher mkdisc
	$(MAKE) -C $(PSOXIDE) hello-pack-disc
	@mkdir -p $(ROOT)/dist
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out $(ROOT)/dist/relocation.bin --volume PSXRELOC \
		--game "BREAKOUT=$(EXAMPLES)/game-breakout.exe" \
		--image "HELLO PACK=$(EXAMPLES)/hello-pack.cue"
	@echo
	@echo "Now: cd $(PSOXIDE)/emu && cargo run -p frontend --release -- launch \\"
	@echo "       --path $(ROOT)/dist/relocation.cue --steps 200000000 \\"
	@echo "       --press '250:right:8,320:cross:8' --dump-hw /tmp/relocation.ppm"
	@echo "The dumped frame must read ALL PASS."

clean:
	rm -rf $(BUILD) $(ROOT)/dist
