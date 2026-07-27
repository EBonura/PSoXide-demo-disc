# PSoXide demo disc.
#
# `make disc` builds the chain-load blob, the launcher, every program, and lays
# them out into the PSoXide game library as one disc. `make check` runs the
# host tests.
#
# Two of the nine cannot be rebuilt from a fresh clone: Cortex Ignition's
# project lives under editor/projects/, which PSoXide gitignores, and hl-psx
# keeps its cooked assets and music outside git too. Both are staged from the
# sibling working trees. See PLAN.md.

.PHONY: help disc programs loader launcher examples mkdisc check relocation-check clean

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
MENU_CREDIT := Music by Just Music, used by permission

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
VOXIDE   := $(GAMES)/voxide/game/target/$(PSX_TARGET)/release/voxide.exe
PSXCEL   := $(GAMES)/psxcel/game/target/$(PSX_TARGET)/release/psxcel.exe
CELESTE  := $(GAMES)/pico8-psx/games/celeste-collection/target/$(PSX_TARGET)/release/celeste-collection.exe
GHPSX    := $(GAMES)/gh-psx/dist/gh-psx.cue
HLPSX    := $(GAMES)/hl-psx/dist/hl-psx.cue
CORTEX   := $(PSOXIDE)/editor/projects/cortex_v1/baked/cortex_v1.cue
HWTESTS  := $(EXAMPLES)/hardware-tests.cue

help:
	@echo "make disc             - build everything into \"$(DIST)\""
	@echo "make check            - host tests (disc-toc, mkdisc)"
	@echo "make relocation-check - disc that proves a relocated game still finds its data"
	@echo "make clean            - drop build/ (the disc in the library is left alone)"

loader:
	cd loader && CARGO_TARGET_DIR=$(BUILD) \
		RUSTFLAGS="-Clink-arg=-Tloader.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

# The launcher embeds the blob, so it always rebuilds after it.
launcher: loader
	cd launcher && CARGO_TARGET_DIR=$(BUILD) LOADER_BLOB=$(LOADER_EXE) \
		RUSTFLAGS="-Clink-arg=-T$(PSOXIDE)/sdk/psoxide.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

examples:
	$(MAKE) -C $(PSOXIDE) game-breakout game-invaders game-magikaaaaaarp-pong
	$(MAKE) -C $(PSOXIDE) hardware-tests-disc

# voxide, PSXcel and the Celeste collection never read the disc after boot, so
# they ride as bare EXEs and do not care which SDK they were built against.
# gh-psx plays CD-DA, so it ships its whole image and needs the SDK with
# psx_io::disc_base (PSoXide branch demo-disc-lba-base, pinned in its own
# third_party/PSoXide).
programs: examples
	$(MAKE) -C $(GAMES)/voxide compile
	$(MAKE) -C $(GAMES)/psxcel build
	$(MAKE) -C $(GAMES)/pico8-psx collection
	$(MAKE) -C $(GAMES)/gh-psx disc
	$(MAKE) -C $(PSOXIDE) cortex-ignition-v1-project-disc
	cd $(GAMES)/hl-psx && cargo run --release -- disc --psoxide $(PSOXIDE)

mkdisc:
	cd tools/mkdisc && cargo build --release

disc: launcher programs mkdisc
	@mkdir -p "$(DIST)"
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out "$(DIST)/$(DISC_NAME).bin" --volume PSXDEMO \
		--image "CORTEX IGNITION=$(CORTEX)" \
		--image "HALF-LIFE=$(HLPSX)" \
		--game "VOXIDE=$(VOXIDE)" \
		--game "CELESTE COLLECTION=$(CELESTE)" \
		--game "PSXCEL=$(PSXCEL)" \
		--image "GUITAR HERO=$(GHPSX)" \
		--game "BREAKOUT=$(EXAMPLES)/game-breakout.exe" \
		--game "SPACE INVADERS=$(EXAMPLES)/game-invaders.exe" \
		--game "MAGIKAAAAARP PONG=$(EXAMPLES)/game-magikaaaaaarp-pong.exe" \
		--image "HARDWARE TESTS=$(HWTESTS)" \
		$(foreach t,$(MENU_CDDA),--menu-cdda "$(t)") --credit "$(MENU_CREDIT)" \
		--share-cdda "MAGIKAAAAARP PONG=GUITAR HERO" \
		--describe "CORTEX IGNITION=Explore a derelict complex in this original 3D action game|Esplora un complesso abbandonato in un gioco d'azione 3D" \
		--describe "HALF-LIFE=Half-Life rebuilt for the PlayStation, streamed off this disc|Half-Life ricostruito per PlayStation, caricato da questo disco" \
		--describe "VOXIDE=Mine blocks and build in a world generated as you walk|Scava blocchi e costruisci in un mondo generato mentre cammini" \
		--describe "CELESTE COLLECTION=Both Celeste Classic games, climbed one screen at a time|I due Celeste Classic, scalati uno schermo alla volta" \
		--describe "PSXCEL=A working spreadsheet you drive entirely with a gamepad|Un vero foglio di calcolo che usi solo col controller" \
		--describe "GUITAR HERO=Hit the notes as the fretboard rushes at you, in CD time|Colpisci le note sul manico che scorre, a tempo col CD" \
		--describe "BREAKOUT=Clear every brick without losing the ball off the bottom|Elimina tutti i mattoni senza perdere la palla in fondo" \
		--describe "SPACE INVADERS=Hold the line as the wave drops lower with every pass|Resisti mentre l'ondata scende a ogni passaggio" \
		--describe "MAGIKAAAAARP PONG=Pong for two players, with a fish where the ball should be|Pong per due giocatori, con un pesce al posto della palla" \
		--describe "HARDWARE TESTS=The suite that checks this console against the emulator|La suite che verifica questa console contro l'emulatore"

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
	@echo "       --press '150:down,220:cross:8' --dump-hw /tmp/relocation.ppm"
	@echo "The dumped frame must read ALL PASS."

clean:
	rm -rf $(BUILD) $(ROOT)/dist
