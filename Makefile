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
		--describe "CORTEX IGNITION=Souls-like on the PSoXide engine. World streams off disc in chunks, sim at 60 Hz, render at 30.|Souls-like sul motore PSoXide. Il mondo arriva dal disco a blocchi, sim a 60 Hz, render a 30." \
		--describe "HALF-LIFE=Half-Life's real maps and models, cut to fit 2 MB of RAM and streamed off this disc. Rust, no C.|Mappe e modelli veri di Half-Life, ridotti a 2 MB di RAM e letti da questo disco. In Rust, senza C." \
		--describe "VOXIDE=Voxel world generated as you walk. Chunks are meshed a ring ahead of the draw distance. Tech-demo.|Mondo a voxel generato mentre cammini. I blocchi sono preparati un anello oltre la vista. Tech-demo." \
		--describe "CELESTE COLLECTION=Both PICO-8 carts rebuilt as native PS1 code. Two games and a menu in one 490 KB executable.|Le due cartucce PICO-8 riscritte in codice PS1 nativo. Due giochi e un menu in 490 KB." \
		--describe "PSXCEL=A spreadsheet on a machine with no floating point: every cell is decimal fixed-point in i64.|Un foglio di calcolo su una macchina senza virgola mobile: ogni cella e in fixed-point i64." \
		--describe "GH-PSX=Note timing anchored to the CD's playback position, not the frame counter, so it cannot drift.|Le note seguono la posizione di lettura del CD, non il contatore dei frame, quindi non sfasano." \
		--describe "BREAKOUT=Engine sample: the whole 2D path, sprites to collision, in 180 KB of PS1 executable.|Esempio del motore: tutto il percorso 2D, dagli sprite alle collisioni, in 180 KB." \
		--describe "SPACE INVADERS=Engine sample: a wave of sprites and per-enemy state, in 188 KB of PS1 executable.|Esempio del motore: un'ondata di sprite con stato per nemico, in 188 KB." \
		--describe "MAGIKAAAAARP PONG=Built to test CD audio under read load. The bars are a spectrum analysed offline, 16 bands at 30 Hz.|Fatto per provare l'audio da CD sotto carico. Le barre sono uno spettro analizzato prima, 16 bande." \
		--describe "HARDWARE TESTS=Measures this console and prints the results as QR pages, so silicon can be diffed against the emulator.|Misura questa console e stampa i risultati come QR, per confrontarla con l'emulatore."

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
