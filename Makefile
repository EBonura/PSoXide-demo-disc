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
MENU_CREDIT := Music by Just Music, used by permission
# Tempo and first-beat offset per track, in the same order, measured by
# tools/beatgrid.py. The menu pulses on these; a guessed tempo drifts.
MENU_BEATS  := 176000:34 175000:23 173980:46 174380:342
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
VOXIDE   := $(GAMES)/voxide/game/target/$(PSX_TARGET)/release/voxide.exe
PSXCEL   := $(GAMES)/psxcel/game/target/$(PSX_TARGET)/release/psxcel.exe
CELESTE  := $(GAMES)/pico8-psx/games/celeste-collection/target/$(PSX_TARGET)/release/celeste-collection.exe
GHPSX    := $(GAMES)/gh-psx/dist/gh-psx.cue
HLPSX    := $(GAMES)/hl-psx/dist/hl-psx.cue
CORTEX   := $(PSOXIDE)/editor/projects/cortex_v1/baked/cortex_v1.cue
HWTESTS  := $(EXAMPLES)/hardware-tests.cue

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
	$(MAKE) disc-only

# Just the layout, for when nothing but the text or the audio changed. Also
# the one place the mkdisc invocation lives, so it cannot drift from what
# `make disc` builds.
disc-only: mkdisc
	@mkdir -p "$(DIST)"
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out "$(DIST)/$(DISC_NAME).bin" --volume PSXDEMO \
		--image "CORTEX IGNITION=$(CORTEX)" \
		--image "HALF-LIFE=$(HLPSX)" \
		--game "VOXIDE=$(VOXIDE)" \
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
		--describe "CORTEX IGNITION=A souls-like built from nothing for a machine with two megabytes of memory. The world arrives off the disc as you walk into it, a room at a time, so it can be far larger than the console can hold.|Un souls-like creato da zero per una macchina con due megabyte di memoria. Il mondo arriva dal disco mentre cammini, una stanza alla volta, e per questo supera di molto la memoria della console." \
		--describe "HALF-LIFE=Black Mesa on a PlayStation. The real maps and models, squeezed until they fit and fed off the disc as you move through them. Rebuilt from scratch, and far enough along to walk around in.|Black Mesa su PlayStation. Le mappe e i modelli originali, ridotti quanto basta per entrare in memoria e letti dal disco mentre avanzi. Riscritto da zero, abbastanza avanti da camminarci dentro." \
		--describe "VOXIDE=An endless block world the console invents as you walk. It quietly builds the ground one ring beyond what you can see, so the horizon is always finished by the time you get there.|Un mondo di blocchi infinito che la console inventa mentre cammini. Prepara il terreno un anello oltre quello che vedi, quindi trovi l'orizzonte sempre pronto quando ci arrivi." \
		--describe "CELESTE COLLECTION=Both Celeste Classic games, rebuilt as native PlayStation code rather than emulated. Two complete games and the menu that picks between them, in less space than one photograph off a phone.|I due Celeste Classic, riscritti in codice PlayStation nativo invece che emulati. Due giochi interi e il menu che li sceglie, in meno spazio di una singola foto da telefono." \
		--describe "PSXCEL=A working spreadsheet on a games console, driven entirely with a joypad. The PlayStation cannot do decimal arithmetic at all, so every cell counts in whole numbers and puts the point back afterwards.|Un foglio di calcolo vero su una console, guidato solo col joypad. La PlayStation non sa fare i decimali, quindi ogni cella conta in numeri interi e rimette la virgola alla fine." \
		--describe "GH-PSX=A rhythm game that listens to the disc instead of counting frames. It keeps asking the CD where the needle is, so the notes stay with the music even when the console falls behind.|Un gioco ritmico che ascolta il disco invece di contare i frame. Chiede al CD dove si trova la puntina, quindi le note restano con la musica anche quando la console rimane indietro." \
		--describe "BREAKOUT=The oldest idea in games, built as a sample for the engine everything else here runs on. Small enough to sit in a corner of the disc, complete enough to lose an evening to.|La prima idea dei videogiochi, scritta come esempio per il motore su cui gira tutto il resto. Occupa un angolo del disco, ma basta e avanza per perderci una serata." \
		--describe "SPACE INVADERS=A wall of aliens coming down a row at a time. Another engine sample, here to show how little you actually need before something stops being a demo and starts being a game.|Un muro di alieni che scende una fila alla volta. Un altro esempio del motore, qui per mostrare quanto poco serve prima che una demo diventi davvero un gioco." \
		--describe "MAGIKAAAAARP PONG=Pong, written as an excuse to torture the CD drive: music playing off the disc while the game reads from it. The bars are the song itself, measured beforehand and replayed in step.|Pong, scritto come scusa per torturare il lettore CD: la musica suona dal disco mentre il gioco legge dallo stesso disco. Le barre di lato sono la canzone, misurata prima e riprodotta a tempo." \
		--describe "HARDWARE TESTS=Not a game. It measures what this particular console actually does, prints the answers as codes you can photograph, and hands them back so the emulator can be held to the same standard.|Questo non lo si gioca. Misura quello che fa davvero questa console, stampa le risposte come codici da fotografare e le restituisce per mettere alla prova l'emulatore."

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
