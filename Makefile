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

.PHONY: help disc disc-only programs loader launcher examples mkdisc check relocation-check clean

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

# The disc lands in PSoXide's game library, laid out the way every other
# homebrew entry there is: <library>/<Name>/<Name>.{bin,cue}.
DISC_NAME ?= PSoXide Demo Disc
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
NITROXIDE     := $(NITROXIDE_SRC)/game/target/$(PSX_TARGET)/release/nitroxide.exe

help:
	@echo "make disc             - build everything into \"$(DIST)\""
	@echo "make disc-only        - relay out the disc without rebuilding the programs"
	@echo "make check            - host tests (disc-toc, mkdisc)"
	@echo "make relocation-check - disc that proves a relocated game still finds its data"
	@echo "make clean            - drop build/ (the disc in the library is left alone)"

loader:
	cd loader && CARGO_TARGET_DIR=$(BUILD) \
		RUSTFLAGS="-Clink-arg=-Tloader.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

# Which pressing this is, drawn in the launcher's header. Tag a burn
# (`git tag v0.3 && make disc`) and the disc identifies itself on camera.
DISC_VERSION := $(shell git -C $(ROOT) describe --tags --always --dirty 2>/dev/null)

# The launcher embeds the blob, so it always rebuilds after it.
launcher: loader
	cd launcher && CARGO_TARGET_DIR=$(BUILD) LOADER_BLOB=$(LOADER_EXE) DISC_VERSION=$(DISC_VERSION) \
		RUSTFLAGS="-Clink-arg=-T$(PSOXIDE)/sdk/psoxide.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

examples:
	$(MAKE) -C $(PSOXIDE) game-breakout game-invaders game-magikaaaaaarp-pong
	$(MAKE) -C $(PSOXIDE) hardware-tests-disc

# NitroXide, PSXcel and the Celeste collection never read the disc after
# boot, so they ride as bare EXEs and do not care which SDK they were built
# against.
# voxide (V0.1.5) streams its sfx bank from WORLD.PAK at boot, so it ships
# its whole image and rides the same disc_base relocation as hello-pack.
# gh-psx plays CD-DA, so it ships its whole image and needs the SDK with
# psx_io::disc_base (PSoXide branch demo-disc-lba-base, pinned in its own
# third_party/PSoXide).
programs: examples
	$(MAKE) -C $(GAMES)/voxide disc
	$(MAKE) -C $(NITROXIDE_SRC) build
	$(MAKE) -C $(GAMES)/psxcel build
	$(MAKE) -C $(GAMES)/pico8-psx collection
	$(MAKE) -C $(GAMES)/gh-psx disc
	@$(MAKE) cortex-if-stale
	cd $(GAMES)/hl-psx && cargo run --release -- disc --psoxide $(PSOXIDE)

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

disc: launcher programs mkdisc
	$(MAKE) disc-only

# Just the layout, for when nothing but the text or the audio changed. Also
# the one place the mkdisc invocation lives, so it cannot drift from what
# `make disc` builds.
disc-only: mkdisc
	@mkdir -p "$(DIST)"
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out "$(DIST)/$(DISC_NAME).bin" --volume PSXDEMO \
		--image "CORTEX IGNITION=$(CORTEX)" \
		--image "HALF-LIFE=$(HLPSX)" \
		--image "VOXIDE=$(VOXIDE)" \
		--game "NITROXIDE=$(NITROXIDE)" \
		--game "CELESTE COLLECTION=$(CELESTE)" \
		--game "PSXCEL=$(PSXCEL)" \
		--image "GH-PSX=$(GHPSX)" \
		--game "BREAKOUT=$(EXAMPLES)/game-breakout.exe" \
		--game "SPACE INVADERS=$(EXAMPLES)/game-invaders.exe" \
		--game "MAGIKAAAAARP PONG=$(EXAMPLES)/game-magikaaaaaarp-pong.exe" \
		--image "HARDWARE TESTS=$(HWTESTS)" \
		$(foreach t,$(MENU_CDDA),--menu-cdda "$(t)") \
		$(foreach b,$(MENU_BEATS),--menu-beat $(b)) \
		--menu-title "KNUCKLE DUST" --menu-title "RUSTED HAMMER" \
		--menu-title "CHAINSAW HEART" --menu-title "NIGHT CRAWLER" \
		--credit "$(MENU_CREDIT)" \
		--share-cdda "MAGIKAAAAARP PONG=GH-PSX" \
		--describe "CORTEX IGNITION=An original souls-like, built from scratch for the PlayStation. The current build is a PSoXide engine tech demo rather than a complete game, with streamed rooms, combat and animation.|Un souls-like originale, creato da zero per PlayStation. La versione attuale e una tech demo del motore PSoXide, non un gioco completo, con stanze in streaming, combattimento e animazioni." \
		--describe "HALF-LIFE=A from-scratch PlayStation port of Half-Life. The full campaign has been converted and much of the game works, but it is not yet playable from start to finish.|Half-Life portato su PlayStation da zero. L'intera campagna e stata convertita e gran parte del gioco funziona, ma non e ancora giocabile dall'inizio alla fine." \
		--describe "VOXIDE=A Minecraft clone built for the original PlayStation. This is an early playable build: world generation, mining, crafting and survival work, but much of the game is still unfinished.|Un clone di Minecraft per la prima PlayStation. Questa e una prima versione giocabile: generazione del mondo, scavo, crafting e sopravvivenza funzionano, ma gran parte del gioco e ancora incompleta." \
		--describe "NITROXIDE=A Rocket League clone built for the original PlayStation. An early build: drive, boost and score, with two players on a split screen. No AI opponent, sound or aerial play yet.|Un clone di Rocket League per la prima PlayStation. E una prima versione: guida, boost e gol, anche in due a schermo diviso. Non ci sono ancora avversari CPU, audio o gioco aereo." \
		--describe "CELESTE COLLECTION=Both Celeste Classic games, rebuilt as native PlayStation software with no emulation. The collection is complete: both games and their launcher fit in less than half a megabyte.|I due Celeste Classic riscritti come software nativo PlayStation, senza emulazione. La raccolta e completa: entrambi i giochi e il menu stanno in meno di mezzo megabyte." \
		--describe "PSXCEL=A working Microsoft Excel clone for the original PlayStation, controlled with a joypad. This build is fully functional, with formulas, charts, themes and memory-card saves.|Un clone funzionante di Microsoft Excel per PlayStation, controllato col joypad. Questa versione e completa e include formule, grafici, temi e salvataggi su memory card." \
		--describe "GH-PSX=A Guitar Hero-style rhythm game for the original PlayStation. This is a bare-bones, one-song prototype: the full loop works, but sustains, star power and polish are still missing.|Un gioco in stile Guitar Hero per la prima PlayStation. E un prototipo essenziale con una sola canzone: il ciclo completo funziona, ma mancano note lunghe, star power e rifiniture." \
		--describe "BREAKOUT=A Breakout clone and compact example of the engine used across this disc. This one is complete and fully playable, with input, collision, sound, scoring and a full game loop.|Un clone di Breakout e un esempio compatto del motore usato in questo disco. Il gioco e completo, con input, collisioni, audio, punti e un ciclo di gioco completo." \
		--describe "SPACE INVADERS=A Space Invaders clone and a second engine example. This one is also complete and fully playable, with formations, shields, scoring and enemy fire.|Un clone di Space Invaders e un secondo esempio del motore. Anche questo e completo e giocabile, con formazioni, scudi, punti e fuoco nemico." \
		--describe "MAGIKAAAAARP PONG=Pong with a live CD-audio visualizer. The game is complete and playable: music streams from the disc while pre-analysed frequency bands drive the bars in sync.|Pong con un visualizzatore audio dal vivo. Il gioco e completo: la musica arriva dal CD mentre le frequenze analizzate in anticipo muovono le barre a tempo." \
		--describe "HARDWARE TESTS=A hardware test suite, not a game. The current suite is working and ready to use, displaying real PlayStation measurements as photo-ready codes for checking emulator accuracy.|Una suite di test hardware, non un gioco. E funzionante e pronta all'uso: mostra le misure della vera PlayStation come codici da fotografare per verificare la precisione degli emulatori." \


check:
	cd carousel && cargo test
	cd disc-toc && cargo test
	cd tools/mkdisc && cargo test

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
