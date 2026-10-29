# PSoXide demo disc.
#
# Every pressing is a lineup pressing: `make lineup-disc LINEUP=release/lineup-<version>.json`
# takes each program as it was already built and played (a cue and BIN named in the
# lineup with their sha256, source revision and build receipt), refuses any input
# whose hash moved, and lays them out behind the launcher this repository builds.
# Nothing is rebuilt from the game submodules. The release channels below
# (`press-public`, `press-private`, gates, install, release) wrap it for one VERSION.
# `make check` runs the host tests.
#
# The non-lineup `make disc` path (rebuild every program from the submodule pins)
# was retired on 2026-10-29: lineups have been the only real path since v0.45, and
# the old path carried stale pins nobody could press from.

.PHONY: help disc-tools disc-budget program-headless-check celeste-navigation-check loader launcher mkdisc check relocation-check chainload-probe clean

ROOT       := $(CURDIR)
# The launcher and the loader are built from the pinned editor tree.
EDITOR     ?= $(ROOT)/games/PSoXide-editor
SDK        ?= $(ROOT)/games/PSoXide-sdk
EMULATOR   ?= $(ROOT)/games/PSoXide-emulator
BUILD      := $(ROOT)/build
OUT        := $(BUILD)/mipsel-sony-psx/release
EXAMPLES   := $(EDITOR)/build/examples/mipsel-sony-psx/release
MKDISC     := $(ROOT)/tools/mkdisc/target/release/mkdisc
DISC_TOOLS := $(ROOT)/tools/disc-tools/target/release/disc-tools

# Menu music: six songs by Mulula VGM Loops (Ambient Jungle Vol 1, Vol 2 and
# Ambient Jungle D&B Vol 3), CC BY 4.0, mixed into pieces and packed as XA
# files that ride in the launcher's own ISO (audio/launcher). The credit has
# to ship with the disc and stay one tap away in the menu: the launcher's
# credits card carries the full attribution, and this short form is the
# table's. Keep audio/README.md, README.md, CHANGELOG.md and release/README.txt
# in step with it.
MENU_XA_DIR   := $(ROOT)/audio/launcher
MENU_XA       := $(MENU_XA_DIR)/MUSIC1.XA $(MENU_XA_DIR)/MUSIC2.XA
MENU_SONGS_RS := $(MENU_XA_DIR)/songs.rs
MENU_SPECTRUM := $(MENU_XA_DIR)/menu.spectrum
MENU_CREDIT   := Music: Mulula VGM Loops, CC BY 4.0
# The song titles, in the order the songs were packed (the table's song
# numbers). Written out rather than looped: make's foreach splits on
# whitespace, which takes the titles apart at their spaces.
#
# No tempo grid (--menu-beat) is pressed for these: they are loops of
# different tempos chained end to end, so a song has no single beat to pulse
# on (`disc-tools beatgrid` on three windows of each gave tempos that
# disagreed by 20 to 50 BPM). The menu follows the level-meter data instead,
# which is measured from the audio and needs no tempo.

# Re-author the music (only when the songs change): pack the mixed WAVs into
# XA files, write the song table the launcher compiles in, and analyse the
# level meter. The WAVs are not in the repository (115 MB); the XA files and
# the analysis are, as the .cdda files were. XA_COOK is a psx-audio-cook
# that has `xa-pack` (on the SDK's main since 2026-10-16, and in the editor
# workspace's `psx-audio-cook`; build the one `tools/audio-cook` of NitroXide
# wraps if the pinned SDK predates it).
#   make music MUSIC_WAVS="a.wav b.wav ..." XA_COOK=/path/to/psx-audio-cook
#   make music-check <same arguments>   cook again, fail if any committed byte changes
.PHONY: music
music: mkdisc
	@test -n "$(MUSIC_WAVS)" -a -x "$(XA_COOK)" || { echo "music: set MUSIC_WAVS (the song WAVs, in song order) and XA_COOK (a psx-audio-cook with xa-pack)"; exit 1; }
	$(XA_COOK) xa-pack "$(MENU_XA_DIR)" MUSIC $(MUSIC_WAVS) --manifest "$(MENU_XA_DIR)/music.json" --rust "$(MENU_SONGS_RS)"
	$(MKDISC) analyse "$(MENU_SPECTRUM)" $(MUSIC_WAVS)

# Maintainer proof that audio/launcher is what the WAVs cook to.
.PHONY: music-check
music-check: music
	@git diff --exit-code -- "$(MENU_XA_DIR)" && echo "music-check: the cook is byte-identical to the committed music"


# Which optional programs a pressing keeps. A lineup lists every program its
# edition can carry; the flags choose among the optional ones. Half-Life,
# Hollow Knight, Counter-Strike, WipEout, Ocarina of Time and the hardware tests
# are never on the Public edition: any flag makes the pressing Private (the
# launcher is built with its `private` feature, and the public upload paths
# refuse it). The release channels below set them per edition.
HL ?=
CS ?=
HK ?=
# WipEout: PSoXide Edition rides the lineup pressings only (make lineup-disc ... WO=1):
# it is built in its own repository and has no submodule here.
WO ?=
# The hardware test suite is likewise lineup-only (HWT=1).
HWT ?=
# Ocarina of Time rides the lineup pressings only (make lineup-disc ... OOT=1):
# it is built from the owner's own ROM in its own repository, has no submodule
# here, and is on the Private edition only.
OOT ?=
# HL=0 means off, not "0 is a non-empty string, so on". Same for CS and HK.
override HL := $(filter-out 0,$(HL))
override CS := $(filter-out 0,$(CS))
override HK := $(filter-out 0,$(HK))
override WO := $(filter-out 0,$(WO))
override HWT := $(filter-out 0,$(HWT))
override OOT := $(filter-out 0,$(OOT))
PRIVATE := $(strip $(HL)$(CS)$(HK)$(WO)$(HWT)$(OOT))

# The pressing has to fit the blank it is burned on. mkdisc adds up every
# sector before it writes anything and fails with each program's share when
# the layout is over this. 359999 is an 80-minute CD-R; a 74-minute blank is
# 333000. `make lineup-budget` prints the table without writing an image. The
# Private release channel presses for a real 80-minute blank (359844 sectors).
DISC_MAX_SECTORS ?= 359999

# The emulator package was renamed from `frontend` to `psoxide-emulator` after the
# component tuple's pin. Name the package and its binary from whichever emulator
# is checked out. Releases use the PGO build of the one emulator checkout instead
# (RELEASE_FRONTEND below).
EMULATOR_PACKAGE ?= $(or $(shell sed -n 's/^name *= *"\(.*\)"/\1/p' $(EMULATOR)/emu/crates/frontend/Cargo.toml 2>/dev/null | head -1),frontend)
FRONTEND ?= $(EMULATOR)/target/release/$(EMULATOR_PACKAGE)


# A pressing lands in DIST as <DISC_NAME>.{bin,cue,...}, by default the PSoXide
# games library. A lineup pressing is named for the disc it replaces: "HL" when it
# carries Half-Life, then the pressing version. The release channels press into
# build/press/<version>/<edition> instead and install into the library only on
# request (`install-public`, `install-private`).
ifneq ($(LINEUP),)
DISC_NAME ?= $(strip PSoXide Demo Disc $(if $(HL),HL) $(DISC_VERSION))
endif
PSOXIDE_LIB ?= $(HOME)/Downloads/ps1 games
DIST ?= $(PSOXIDE_LIB)
RELEASE_RECEIPT ?= $(DIST)/$(DISC_NAME).release-receipt.json
PRESSING_FLAGS = HL=$(if $(HL),1,0) CS=$(if $(CS),1,0) HK=$(if $(HK),1,0) WO=$(if $(WO),1,0) HWT=$(if $(HWT),1,0) OOT=$(if $(OOT),1,0)

PSX_TARGET  := mipsel-sony-psx
PSX_FLAGS   := --release --target $(PSX_TARGET) -Zbuild-std=core -Zbuild-std-features=compiler-builtins-mem
LOADER_EXE  := $(OUT)/loader.exe
LAUNCHER_EXE := $(OUT)/launcher.exe

help:
	@echo "Release channels (VERSION=v0.NN names release/lineup-<VERSION>.json and -private.json):"
	@echo "  make release-inputs-check VERSION=v0.52   every input under RELEASE_INPUTS, present, hashes verified"
	@echo "  make press-public  VERSION=v0.52          the Public disc into build/press/<VERSION>/public"
	@echo "  make press-private VERSION=v0.52          the Private disc (the one that is burned) into .../private"
	@echo "  make gates-public / gates-private         preburn check, program boots, chain-loads (and Celeste navigation)"
	@echo "  make install-public / install-private     copy the pressing into the games library (temp name, verify, rename)"
	@echo "  make release-public-dry-run VERSION=...   stage and test the itch packages from the library Public disc; pushes nothing"
	@echo "  make release-public VERSION=...           the same, then asks for the version and runs butler for both itch channels"
	@echo "Lower level:"
	@echo "  make lineup-disc LINEUP=release/lineup-<version>.json   press one lineup"
	@echo "  make lineup-check LINEUP=...              prove a lineup is self-contained"
	@echo "  make lineup-budget LINEUP=...             print the sector budget"
	@echo "  make check                                host tests"
	@echo "  make clean                                drop build/"

loader:
	cd loader && CARGO_TARGET_DIR=$(BUILD) \
		RUSTFLAGS="-Cllvm-args=-disable-mips-df-backward-search -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

# Which pressing this is, drawn in the launcher's header. Tag a burn
# (`git tag v0.3 && make disc`) and the disc identifies itself on camera.
# Only v* tags name pressings; the rolling `web-disc` release tag that feeds
# the browser emulator would otherwise leak into every version string.
DISC_TITLE ?= DEMO DISC
DISC_VERSION ?= $(shell git -C $(ROOT) describe --tags --match 'v*' --always --dirty 2>/dev/null)

# The launcher embeds the blob, so it always rebuilds after it.
launcher: loader
	cd launcher && CARGO_TARGET_DIR=$(BUILD) LOADER_BLOB=$(LOADER_EXE) XA_SONGS_RS=$(MENU_SONGS_RS) DISC_VERSION=$(DISC_VERSION) DISC_TITLE="$(DISC_TITLE)" \
		RUSTFLAGS="-Cllvm-args=-disable-mips-df-backward-search -Clink-arg=-T$(SDK)/sdk/psoxide.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS) $(if $(PRIVATE),--features private)

mkdisc:
	cd tools/mkdisc && cargo build --release

# The host tools: pressing receipts, release checks, asset cooking. Plain
# cargo, no submodule needed, so this builds on a fresh clone before
# `components` has run. Rebuilds only when its sources change.
DISC_TOOLS_SRC := $(wildcard $(ROOT)/tools/disc-tools/src/*.rs) $(ROOT)/tools/disc-tools/Cargo.toml
$(DISC_TOOLS): $(DISC_TOOLS_SRC)
	cd tools/disc-tools && cargo build --release
disc-tools: $(DISC_TOOLS)


# Which programs a pressing carries is decided here, when the disc is built
# (the flags below and the lineup file); the launcher shows what the disc's
# table lists and has no way to reveal anything else. Cortex Ignition has been
# on both pressings since 2026-09-03.

# The three HALF-LIFE arguments travel together: an image without its version
# and description would press, but announce itself wrong.
ifneq ($(HL),)
HL_ARGS = --image "HALF-LIFE=$(HLPSX)" \
	--shot "HALF-LIFE=$(SHOTS_OUT)/halflife.shot" \
	--version-of "HALF-LIFE=$(V_HLPSX)" \
	--describe "HALF-LIFE=A from-scratch PlayStation port of Half-Life. The full campaign has been converted and much of the game works, but it is not yet playable from start to finish.|Half-Life portato su PlayStation da zero. L'intera campagna e stata convertita e gran parte del gioco funziona, ma non e ancora giocabile dall'inizio alla fine."
endif
ifneq ($(CS),)
CS_ARGS = --image "COUNTER-STRIKE=$(CSPSX)" \
	--shot "COUNTER-STRIKE=$(SHOTS_OUT)/counterstrike.shot" \
	--version-of "COUNTER-STRIKE=$(V_CSPSX)" \
	--describe "COUNTER-STRIKE=A from-scratch PlayStation port of Counter-Strike 1.6, cooked from your own copy. Bots, the buy menu, bomb and hostage rounds, and two-player split screen. Still in development.|Counter-Strike 1.6 portato su PlayStation da zero, partendo dalla propria copia del gioco. Bot, menu acquisti, bomba, ostaggi e schermo diviso per due giocatori. In sviluppo."
endif
ifneq ($(HWT),)
HWT_ARGS = --image "HARDWARE TESTS=$(HWTESTS)" \
	--shot "HARDWARE TESTS=$(SHOTS_OUT)/hwtests.shot" \
	--shot "HARDWARE TESTS=$(SHOTS_OUT)/hwtests2.shot" \
	--version-of "HARDWARE TESTS=$(V_HWTESTS)" \
	--describe "HARDWARE TESTS=A hardware test suite, not a game. The current suite is working and ready to use, displaying real PlayStation measurements as photo-ready codes for checking emulator accuracy.|Una suite di test hardware, non un gioco. E funzionante e pronta all'uso: mostra le misure della vera PlayStation come codici da fotografare per verificare la precisione degli emulatori."
endif
ifneq ($(WO),)
WO_ARGS = --image "WIPEOUT PSOXIDE=$(WIPEOUT)" \
	--shot "WIPEOUT PSOXIDE=$(SHOTS_OUT)/wipeout1.shot" \
	--shot "WIPEOUT PSOXIDE=$(SHOTS_OUT)/wipeout2.shot" \
	--version-of "WIPEOUT PSOXIDE=$(V_WIPEOUT)" \
	--describe "WIPEOUT PSOXIDE=WipEout (1995) rewritten in Rust for the original PlayStation, cooked from your own copy of the game. Tracks, ships, weapons and two-player split screen.|WipEout (1995) riscritto in Rust per la prima PlayStation, partendo dalla propria copia del gioco. Piste, navi, armi e schermo diviso per due."
endif
ifneq ($(OOT),)
OOT_ARGS = --image "OCARINA OF TIME=$(OOTPSX)" \
	--shot "OCARINA OF TIME=$(SHOTS_OUT)/oot1.shot" \
	--shot "OCARINA OF TIME=$(SHOTS_OUT)/oot2.shot" \
	--version-of "OCARINA OF TIME=$(V_OOTPSX)" \
	--describe "OCARINA OF TIME=An early from-scratch PlayStation port of Ocarina of Time, cooked from your own ROM. The opening and Kokiri Forest can be explored. Still in development and slow.|Un primo port di Ocarina of Time per PlayStation, fatto da zero partendo dalla propria ROM. Si esplorano l'inizio e la Foresta Kokiri. In sviluppo e lento."
endif
ifneq ($(HK),)
HK_ARGS = --image "HOLLOW KNIGHT=$(HKPSX)" \
	--shot "HOLLOW KNIGHT=$(SHOTS_OUT)/hollowknight.shot" \
	--version-of "HOLLOW KNIGHT=$(V_HKPSX)" \
	--describe "HOLLOW KNIGHT=An early from-scratch PlayStation port of Hollow Knight, cooked from your own copy. King's Pass, Dirtmouth and the Forgotten Crossroads can be explored, with enemies, benches and saves.|Un primo port di Hollow Knight per PlayStation, fatto da zero partendo dalla propria copia. Si esplorano King's Pass, Dirtmouth e il Crocevia Dimenticato, con nemici, panchine e salvataggi."
endif

# The QUAKE SHAREWARE arguments travel together for the same reason the
# HALF-LIFE ones do, and they are not conditional: there is no pressing without
# Quake on it. QUAKE_PREREQS is what stops a disc being laid out around an
# unverified payload. disc-only cannot run until the stamp and the pins check.
QUAKE_ARGS = --image "QUAKE SHAREWARE=$(QUAKE_CUE)" \
	--shot "QUAKE SHAREWARE=$(SHOTS_OUT)/quake-menu.shot" \
	--shot "QUAKE SHAREWARE=$(SHOTS_OUT)/quake-gameplay.shot" \
	--version-of "QUAKE SHAREWARE=$(QUAKE_VERSION)" \
	--describe "QUAKE SHAREWARE=Quake 1.06 shareware for PS1, built on PSoXide with id's GPL source and the QuakePSX C port as references. All nine maps, the full arsenal and Chthon. Still in development.|Quake 1.06 shareware per PS1, costruito su PSoXide usando il sorgente GPL di id e il port C QuakePSX come riferimenti. Nove mappe, arsenale completo e Chthon. In sviluppo."

# Smoke-test every independently maintained program outside the three large
# ports. The script reads the pressed table and navigates by name, so adding or
# reordering carousel cards does not silently point a route at the wrong game.
# It also enters the Arcade collection and starts each of its three guests.
program-headless-check: $(DISC_TOOLS)
	$(DISC_TOOLS) programs \
		--frontend "$(FRONTEND)" \
		--cue "$(DIST)/$(DISC_NAME).cue" \
		--out "$(BUILD)/program-headless" \
		--jobs 3

# Return paths inside the linked Celeste collection, reached through this
# pressing's carousel. Use a fresh output directory for each evidence run.
.PHONY: celeste-navigation-check
CELESTE_NAV_CUE ?= $(DIST)/$(DISC_NAME).cue
CELESTE_NAV_OUT ?= $(BUILD)/celeste-navigation
celeste-navigation-check: $(DISC_TOOLS)
	$(DISC_TOOLS) celeste-nav \
		--frontend "$(FRONTEND)" --cue "$(CELESTE_NAV_CUE)" \
		--out "$(CELESTE_NAV_OUT)"


# ---------------------------------------------------------------------------
# Release channels. A release is one VERSION (v0.NN) and its lineup pair:
# release/lineup-$(VERSION).json (Public, for itch) and
# release/lineup-$(VERSION)-private.json (Private, the disc that is burned).
# These replace the release-public.sh wrapper and the per-lane press, gate,
# install and web scripts. Nothing here publishes by itself: `release-public`
# is the only target that can, and it stops to ask for the version first.
#
#   make release-inputs-check VERSION=v0.53
#   make press-public  VERSION=v0.53
#   make press-private VERSION=v0.53
#   make gates-public  VERSION=v0.53       (and gates-private)
#   make install-public VERSION=v0.53      (and install-private)
#   make release-public-dry-run VERSION=v0.53
#
# The Private pressing is the one console burns use; the Public edition is never
# burned. Each pressing writes its image and receipts to build/press/<VERSION>/<edition>.
# ---------------------------------------------------------------------------
VERSION ?=
RELEASE_FRONTEND ?= $(HOME)/Desktop/repos/PSoXide-emulator/target/pgo/release/psoxide-emulator
PRESS_ROOT ?= $(BUILD)/press
PUBLIC_NAME  = PSoXide Demo Disc $(VERSION)
PRIVATE_NAME = PSoXide Demo Disc $(VERSION) Private
PUBLIC_LINEUP  ?= release/lineup-$(VERSION).json
PRIVATE_LINEUP ?= release/lineup-$(VERSION)-private.json
PUBLIC_DIST  = $(PRESS_ROOT)/$(VERSION)/public
PRIVATE_DIST = $(PRESS_ROOT)/$(VERSION)/private
# What the Private disc carries since v0.52: Half-Life, Hollow Knight, WipEout and
# Ocarina of Time on top of the shared programs, no Arcade or PSXcel, no Counter-Strike
# and no hardware tests. It is pressed for an 80-minute blank (359844 sectors) with a
# 4500 sector zero filler after the last program, so all game data ends well inside
# the radius the console reads cleanly.
PRIVATE_PRESS_FLAGS := HL=1 CS=0 HK=1 WO=1 OOT=1 HWT=0 TAIL_FILLER=4500 DISC_MAX_SECTORS=359844 LEAVE_OUT="ARCADE PSXCEL"
RELEASE_ARGS ?=

.PHONY: need-version release-inputs-check press-public press-private gates-public gates-private install-public install-private release-public-dry-run release-public
need-version:
	@test -n "$(VERSION)" || { echo "set VERSION=v0.NN (release/lineup-<VERSION>.json must exist)"; exit 1; }

release-inputs-check: need-version
	$(MAKE) lineup-check LINEUP=$(PUBLIC_LINEUP)
	$(MAKE) lineup-check LINEUP=$(PRIVATE_LINEUP)

press-public: need-version
	rm -rf "$(PUBLIC_DIST)"
	PSOXIDE_CHECK_GPU_LL_FIFO=1 $(MAKE) lineup-disc LINEUP=$(PUBLIC_LINEUP) DISC_NAME="$(PUBLIC_NAME)" DISC_VERSION=$(VERSION) \
		DIST="$(PUBLIC_DIST)" FRONTEND="$(RELEASE_FRONTEND)"
	@shasum -a 256 "$(PUBLIC_DIST)/$(PUBLIC_NAME).bin" "$(PUBLIC_DIST)/$(PUBLIC_NAME).cue"

press-private: need-version
	rm -rf "$(PRIVATE_DIST)"
	PSOXIDE_CHECK_GPU_LL_FIFO=1 $(MAKE) lineup-disc LINEUP=$(PRIVATE_LINEUP) $(PRIVATE_PRESS_FLAGS) DISC_NAME="$(PRIVATE_NAME)" \
		DISC_VERSION=$(VERSION) DIST="$(PRIVATE_DIST)" FRONTEND="$(RELEASE_FRONTEND)"
	@shasum -a 256 "$(PRIVATE_DIST)/$(PRIVATE_NAME).bin" "$(PRIVATE_DIST)/$(PRIVATE_NAME).cue"

# Gates on a pressed image: the console pre-burn check, every independent program
# boots, every chain-load replays twice byte-identical, and (Public) the Celeste
# collection's return paths. Evidence lands beside the pressing.
GATE_JOBS ?= 2
gates-public: need-version $(DISC_TOOLS)
	@mkdir -p "$(PUBLIC_DIST)/evidence"
	PSOXIDE_CHECK_GPU_LL_FIFO=1 PSOXIDE_CHECK_CC_DMA=1 "$(RELEASE_FRONTEND)" preburn-check --cue "$(PUBLIC_DIST)/$(PUBLIC_NAME).cue"
	PSOXIDE_CHECK_GPU_LL_FIFO=1 PSOXIDE_CHECK_CC_DMA=1 $(DISC_TOOLS) programs --frontend "$(RELEASE_FRONTEND)" \
		--cue "$(PUBLIC_DIST)/$(PUBLIC_NAME).cue" --out "$(PUBLIC_DIST)/evidence/programs" --jobs $(GATE_JOBS)
	PSOXIDE_CHECK_GPU_LL_FIFO=1 PSOXIDE_CHECK_CC_DMA=1 $(DISC_TOOLS) chainloads --frontend "$(RELEASE_FRONTEND)" \
		--cue "$(PUBLIC_DIST)/$(PUBLIC_NAME).cue" --artifact-dir "$(PUBLIC_DIST)/evidence/chainloads"
	PSOXIDE_CHECK_GPU_LL_FIFO=1 PSOXIDE_CHECK_CC_DMA=1 $(DISC_TOOLS) celeste-nav --frontend "$(RELEASE_FRONTEND)" \
		--cue "$(PUBLIC_DIST)/$(PUBLIC_NAME).cue" --out "$(PUBLIC_DIST)/evidence/celeste-nav"

gates-private: need-version $(DISC_TOOLS)
	@mkdir -p "$(PRIVATE_DIST)/evidence"
	PSOXIDE_CHECK_GPU_LL_FIFO=1 PSOXIDE_CHECK_CC_DMA=1 "$(RELEASE_FRONTEND)" preburn-check --cue "$(PRIVATE_DIST)/$(PRIVATE_NAME).cue"
	PSOXIDE_CHECK_GPU_LL_FIFO=1 PSOXIDE_CHECK_CC_DMA=1 $(DISC_TOOLS) programs --frontend "$(RELEASE_FRONTEND)" \
		--cue "$(PRIVATE_DIST)/$(PRIVATE_NAME).cue" --out "$(PRIVATE_DIST)/evidence/programs" --jobs $(GATE_JOBS)
	PSOXIDE_CHECK_GPU_LL_FIFO=1 PSOXIDE_CHECK_CC_DMA=1 $(DISC_TOOLS) chainloads --frontend "$(RELEASE_FRONTEND)" \
		--cue "$(PRIVATE_DIST)/$(PRIVATE_NAME).cue" --artifact-dir "$(PRIVATE_DIST)/evidence/chainloads"

# Copy a pressing into the games library: each file under a temporary name, its
# sha256 checked against the pressing, then renamed over the old one, plus the
# provenance file. Older pressings are reported, never removed.
install-public: need-version $(DISC_TOOLS)
	$(DISC_TOOLS) install --dist "$(PUBLIC_DIST)" --name "$(PUBLIC_NAME)" --library "$(PSOXIDE_LIB)" --lineup "$(PUBLIC_LINEUP)" --edition Public

install-private: need-version $(DISC_TOOLS)
	$(DISC_TOOLS) install --dist "$(PRIVATE_DIST)" --name "$(PRIVATE_NAME)" --library "$(PSOXIDE_LIB)" --lineup "$(PRIVATE_LINEUP)" --edition Private

# The two itch.io pages from the library's Public disc (README, "Releasing to itch"):
# the dry run stages and tests the browser and download packages and prints the exact
# butler commands. `release-public` runs the same checks, then waits for the version to
# be typed and pushes both channels. RELEASE_ARGS go to `disc-tools release` (for
# example --work <dir>, --chrome <path>, --jobs <n>, --smoke). Node 22 and Chrome are needed.
release-public-dry-run: need-version $(DISC_TOOLS)
	$(DISC_TOOLS) release --version $(VERSION) --cue "$(PSOXIDE_LIB)/$(PUBLIC_NAME).cue" --dry-run $(RELEASE_ARGS)

release-public: need-version $(DISC_TOOLS)
	$(DISC_TOOLS) release --version $(VERSION) --cue "$(PSOXIDE_LIB)/$(PUBLIC_NAME).cue" $(RELEASE_ARGS)

# Menu backdrops: the same in-game captures the itch pages use, cooked from
# assets/shots into the 8bpp CLUT blobs the launcher uploads to VRAM.
# Cooked at build time (needs PIL, like the other tools) so the pressed
# pixels always come from the PNGs actually in the repo.
SHOTS_SRC := $(ROOT)/assets/shots
SHOTS_OUT := $(BUILD)/shots
SHOT_NAMES := cortex-current-menu cortex-current-gameplay \
              voxide-day voxide-night nitroxide-boost \
              nitroxide-aerial nitroxide-goal celeste celeste2 psxcel-chart \
              psxcel-editing breakout breakout2 invaders \
              invaders2 pong pong2 halflife quake-menu \
              quake-gameplay $(if $(CS),counterstrike) $(if $(HK),hollowknight) \
              $(if $(WO),wipeout1 wipeout2) $(if $(OOT),oot1 oot2) $(if $(HWT),hwtests hwtests2)
SHOT_FILES := $(foreach n,$(SHOT_NAMES),$(SHOTS_OUT)/$(n).shot)

$(SHOTS_OUT)/%.shot: $(SHOTS_SRC)/%.png $(DISC_TOOLS)
	@mkdir -p "$(SHOTS_OUT)"
	$(DISC_TOOLS) cook-shot "$<" "$@"

# A lineup pressing can leave a program out: LEAVE_OUT="ARCADE PSXCEL" drops those
# entries with their shots, version and description (the lineup file must not carry them).
LEAVE_OUT ?=
ifeq ($(filter ARCADE,$(LEAVE_OUT)),)
ARCADE_ARGS = --image "PSOXIDE ARCADE=$(ARCADE)" \
	--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/breakout.shot" \
	--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/breakout2.shot" \
	--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/invaders.shot" \
	--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/invaders2.shot" \
	--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/pong.shot" \
	--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/pong2.shot" \
	--version-of "PSOXIDE ARCADE=$(V_ARCADE)" \
	--describe "PSOXIDE ARCADE=Three complete native PlayStation arcade games in one collection: Breakout, Space Invaders and Magikarp Pong, with its own live CD-audio visualizer.|Tre giochi arcade completi e nativi per PlayStation in una raccolta: Breakout, Space Invaders e Magikarp Pong, con visualizzatore CD audio."
endif
ifeq ($(filter PSXCEL,$(LEAVE_OUT)),)
PSXCEL_ARGS = --game "PSXCEL=$(PSXCEL)" \
	--shot "PSXCEL=$(SHOTS_OUT)/psxcel-chart.shot" \
	--shot "PSXCEL=$(SHOTS_OUT)/psxcel-editing.shot" \
	--version-of "PSXCEL=$(V_PSXCEL)" \
	--describe "PSXCEL=A working Microsoft Excel clone for the original PlayStation, controlled with a joypad. This build is fully functional, with formulas, charts, themes and memory-card saves.|Un clone funzionante di Microsoft Excel per PlayStation, controllato col joypad. Questa versione e completa e include formule, grafici, temi e salvataggi su memory card."
endif

# Just the layout, for when nothing but the text or the audio changed. Also
# the one place the mkdisc invocation lives, so it cannot drift from what
# `make disc` builds. The program order here is the carousel order: the
# reverse of v0.39 (Manny, 2026-09-26), with PSXcel moved from first to last,
# just before CREDITS (Manny, 2026-10-05).
MKDISC_ARGS = --launcher $(LAUNCHER_EXE) --out "$(DIST)/$(DISC_NAME).bin" --volume PSXDEMO \
		$(ARCADE_ARGS) \
		--game "CELESTE COLLECTION=$(CELESTE)" \
		--image "NITROXIDE=$(NITROXIDE)" \
		--image "VOXIDE=$(VOXIDE)" \
		$(WO_ARGS) \
		$(HK_ARGS) \
		$(CS_ARGS) \
		$(HL_ARGS) \
		$(OOT_ARGS) \
		$(QUAKE_ARGS) \
		--image "CORTEX IGNITION=$(CORTEX_CURRENT)" \
		$(HWT_ARGS) \
		$(PSXCEL_ARGS) \
		$(foreach t,$(MENU_XA),--menu-xa "$(t)") \
		--menu-spectrum "$(MENU_SPECTRUM)" \
		--menu-title "JUNGLE 1 A" --menu-title "JUNGLE 1 B" \
		--menu-title "JUNGLE 2 A" --menu-title "JUNGLE 2 B" \
		--menu-title "JUNGLE DNB A" --menu-title "JUNGLE DNB B" \
		--credit "$(MENU_CREDIT)" \
		--shot "CORTEX IGNITION=$(SHOTS_OUT)/cortex-current-menu.shot" \
		--shot "CORTEX IGNITION=$(SHOTS_OUT)/cortex-current-gameplay.shot" \
		--shot "VOXIDE=$(SHOTS_OUT)/voxide-day.shot" \
		--shot "VOXIDE=$(SHOTS_OUT)/voxide-night.shot" \
		--shot "NITROXIDE=$(SHOTS_OUT)/nitroxide-boost.shot" \
		--shot "NITROXIDE=$(SHOTS_OUT)/nitroxide-aerial.shot" \
		--shot "NITROXIDE=$(SHOTS_OUT)/nitroxide-goal.shot" \
		--shot "CELESTE COLLECTION=$(SHOTS_OUT)/celeste.shot" \
		--shot "CELESTE COLLECTION=$(SHOTS_OUT)/celeste2.shot" \
		--version-of "CORTEX IGNITION=$(V_CORTEX_CURRENT)" \
		--version-of "VOXIDE=$(V_VOXIDE)" \
		--version-of "NITROXIDE=$(V_NITROXIDE)" \
		--version-of "CELESTE COLLECTION=$(V_CELESTE)" \
		--describe "CORTEX IGNITION=A Souls-like built from the ground up for the original PlayStation. This is an early tech demo.|Un souls-like sviluppato da zero per la prima PlayStation. Questo e un primo tech demo." \
		--describe "VOXIDE=A Minecraft clone built for the original PlayStation. This is an early playable build: world generation, mining, crafting and survival work.|Un clone di Minecraft per la prima PlayStation. Prima versione giocabile: generazione del mondo, scavo, crafting e sopravvivenza funzionano." \
		--describe "NITROXIDE=A Rocket League clone built for the original PlayStation. Supports 2 players in split screen.|Un clone di Rocket League per la prima PlayStation. Supporta 2 giocatori a schermo diviso." \
		--describe "CELESTE COLLECTION=Both Celeste Classic games, rebuilt from PICO-8 as native PlayStation games.|I due Celeste Classic, ricostruiti da PICO-8 come giochi nativi PlayStation." \
		$(if $(TAIL_FILLER),--tail-filler $(TAIL_FILLER)) \
		--max-sectors $(DISC_MAX_SECTORS)


# A lineup pressing takes every program as it was already built and played,
# instead of rebuilding it from the submodules: the Half-Life, Hollow Knight
# and Counter-Strike builds Manny tests come off branches the submodule pins do
# not name, and rebuilding them would press something nobody has played.
#
#   make lineup-disc LINEUP=release/lineup-v0.40.json HL=1 CS=1 HK=1 \
#        DIST=<dir> FRONTEND=<frontend>
#
# The lineup file names each input with its sha256, source revision and build
# receipt; `disc-tools lineup` refuses any input whose hash moved, derives the few
# that need it (a data track alone, a boot EXE, an entry lifted from an older
# pressing) and writes build/lineup/lineup.mk, which points the program and
# version variables above at them. The launcher, the layout and the mkdisc
# arguments are the same ones `make disc` uses. The receipts it writes beside
# the image prove every pressed entry is its lineup input.
LINEUP_OUT := $(BUILD)/lineup
# Every input of a release lineup is copied under RELEASE_INPUTS/<version>[-private]
# (and its source mirror under RELEASE_INPUTS/src), so cleaning a lane directory
# cannot break a re-press. `lineup-prepare` refuses a lineup that reaches outside
# it; SELF_CONTAINED=0 lifts that for an old lineup (v0.51 and earlier).
RELEASE_INPUTS ?= $(HOME)/Desktop/repos/campaign-2026-10-15/release-inputs
SELF_CONTAINED ?= 1
ifneq ($(LINEUP),)
-include $(LINEUP_OUT)/lineup.mk
endif

.PHONY: lineup-prepare lineup-disc _lineup-lay lineup-budget
lineup-prepare: $(DISC_TOOLS)
	@test -n "$(LINEUP)" || { echo "lineup: set LINEUP=release/lineup-<version>.json"; exit 1; }
	$(DISC_TOOLS) lineup prepare --lineup "$(LINEUP)" --out "$(LINEUP_OUT)" $(if $(filter 0,$(SELF_CONTAINED)),,--inputs-root "$(RELEASE_INPUTS)")

# Prove a lineup can be re-pressed from the inputs root alone: every input and
# source mirror under RELEASE_INPUTS, present, and hash-gated. Presses nothing.
.PHONY: lineup-check
lineup-check: $(DISC_TOOLS)
	@test -n "$(LINEUP)" || { echo "lineup: set LINEUP=release/lineup-<version>.json"; exit 1; }
	$(DISC_TOOLS) lineup check --exists --lineup "$(LINEUP)" --inputs-root "$(RELEASE_INPUTS)"

lineup-disc: launcher mkdisc lineup-prepare
	$(MAKE) _lineup-lay

lineup-budget: mkdisc lineup-prepare
	$(MAKE) disc-budget

_lineup-lay: $(DISC_TOOLS) $(SHOT_FILES)
	@mkdir -p "$(DIST)"
	$(MKDISC) $(MKDISC_ARGS)
	$(DISC_TOOLS) lineup receipt --lineup "$(LINEUP)" --mk "$(LINEUP_OUT)/lineup.mk" \
		--cue "$(DIST)/$(DISC_NAME).cue" --frontend "$(FRONTEND)" \
		--build-command "make lineup-disc LINEUP=$(LINEUP) $(PRESSING_FLAGS)$(if $(LEAVE_OUT), LEAVE_OUT='$(LEAVE_OUT)')" \
		$(if $(HL),,--omit HALF-LIFE) $(if $(CS),,--omit COUNTER-STRIKE) $(if $(HK),,--omit "HOLLOW KNIGHT") $(if $(WO),,--omit "WIPEOUT PSOXIDE") $(if $(OOT),,--omit "OCARINA OF TIME") $(if $(HWT),,--omit "HARDWARE TESTS") \
		--release-out "$(RELEASE_RECEIPT)" \
		--components-out "$(DIST)/$(DISC_NAME).components.json"

# The sector budget of the pressing these flags select, from inputs already
# built, without writing an image.
disc-budget: mkdisc $(SHOT_FILES)
	$(MKDISC) $(MKDISC_ARGS) --dry-run

# The host tests: the carousel and the disc table crates, the chain-load blob's
# layout, mkdisc and disc-tools (which includes the check that the newest
# lineups are self-contained).
check:
	cargo test --manifest-path $(EDITOR)/engine/Cargo.toml -p psx-carousel
	cargo test --manifest-path $(EDITOR)/engine/Cargo.toml -p psx-disc-toc
	cd tools/mkdisc && cargo test
	cd tools/disc-tools && cargo test

# The disc's SDK, editor and emulator pins must be on their repositories' main.
# GitHub's compare API answers "identical" or "ahead" when main contains the
# revision. DEMO_DISC_ALLOW_PSOXIDE_OFF_MAIN=1 skips the check for a deliberate
# side-branch pressing and says so.
.PHONY: sdk-on-main
sdk-on-main: components
	@$(DISC_TOOLS) components --check-main

# hello-pack streams WORLD.PAK off the disc and paints ALL PASS or a failure
# list, which makes it the end-to-end test for the relocation machinery: its
# image lands 200-odd sectors in, every LBA it was cooked with is wrong by that
# much, and psx_io::disc_base has to make up the difference. Run the result
# with the emulator and read the banner.
relocation-check: launcher mkdisc
	$(MAKE) -C $(EDITOR) hello-tri hello-pack-disc
	@mkdir -p $(ROOT)/dist
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out $(ROOT)/dist/relocation.bin --volume PSXRELOC \
		--game "HELLO TRI=$(EXAMPLES)/hello-tri.exe" \
		--image "HELLO PACK=$(EXAMPLES)/hello-pack.cue"
	@echo
	@echo "Now: cd $(EDITOR)/emu && cargo run -p frontend --release -- launch \\"
	@echo "       --path $(ROOT)/dist/relocation.cue --steps 200000000 \\"
	@echo "       --press '250:right:8,320:cross:8' --dump-hw /tmp/relocation.ppm"
	@echo "The dumped frame must read ALL PASS."

# The chain loader must hand the CD drive over quiet: a plain Pause leaves its
# second response (INT2, 1.2 to 3.9 million clocks later on the console) to land
# in the next program. tools/chainload-probe is the payload that checks for it
# after the launcher chain-loads it. FRONTEND must carry the emulator's silicon
# Pause model (PSoXide-emulator dddd281 or later).
chainload-probe: launcher mkdisc
	@test -n "$(FRONTEND)" || { echo "chainload-probe: set FRONTEND to a psoxide-emulator with the silicon Pause model"; exit 1; }
	sh tools/chainload_probe.sh "$(LAUNCHER_EXE)" "$(FRONTEND)"

clean:
	rm -rf $(BUILD) $(ROOT)/dist

# Fetching is explicit and pinned. Both authoring and validation use their
# own repositories; ordinary guests receive the bootstrapped engine tree.
.PHONY: components verify-components
components: $(DISC_TOOLS)
	$(DISC_TOOLS) components
verify-components: $(DISC_TOOLS)
	$(DISC_TOOLS) components --check
loader mkdisc: components
