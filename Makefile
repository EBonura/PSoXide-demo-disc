# PSoXide demo disc.
#
# `make disc` builds the chain-load blob, the launcher, every program, and lays
# them out into the PSoXide game library as one disc. `make check` runs the
# host tests.
#
# Half-Life cannot be rebuilt from a fresh clone because its cooked assets and
# music live outside Git. Cortex uses an exact PSoXide pin so the active editor
# project remains reproducible as the engine advances.

.PHONY: help disc disc-only quake-verify quake-repin quake-headless-check _quake-headless-check program-headless-check release-frontend release-headless-check _release-headless-check quake-programs quake-programs-verify programs loader launcher examples mkdisc check relocation-check clean

ROOT       := $(CURDIR)
# All programs use the validated shared renderer and asset runtime.
PSOXIDE    ?= $(ROOT)/games/PSoXide-editor
SDK ?= $(ROOT)/games/PSoXide-sdk
EMULATOR ?= $(ROOT)/games/PSoXide-emulator
PROGRAMS_PSOXIDE ?= $(ROOT)/games/PSoXide-editor
PROGRAMS_EXPECTED_PSOXIDE_REV ?= 0946dc892e0e86f190bbb6476874a250935fd5aa
CORTEX_CURRENT_PSOXIDE ?= $(PROGRAMS_PSOXIDE)
CORTEX_CURRENT_EXPECTED_PSOXIDE_REV ?= 0946dc892e0e86f190bbb6476874a250935fd5aa
CORTEX_CURRENT_GUEST_STAGE_ROOT ?= /tmp/psoxide-psx-guest-v1-cortex-current
CORTEX_GUEST_CARGO_HOME ?= /tmp/psoxide-psx-guest-v1/cargo-home
BUILD      := $(ROOT)/build
QUAKE_PROGRAMS_STAMP := $(BUILD)/programs.psoxide-revision
OUT        := $(BUILD)/mipsel-sony-psx/release
EXAMPLES   := $(PROGRAMS_PSOXIDE)/build/examples/mipsel-sony-psx/release
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

# Two pressings. The menu music permission is scoped to the disc without
# Half-Life, and hl-psx is private until its own release, so HL is opt-in:
#   make disc        -> "PSoXide Demo Disc"     (the default pressing)
#   make disc HL=1   -> "PSoXide Demo Disc HL"  (the same disc plus Half-Life)
# The names differ so the two bins cannot be mistaken for each other. Both
# carry Quake shareware; HL=1 adds to the default disc, it does not replace it.
#
# Adding Quake to the Half-Life pressing was the size question. Measured
# 2026-08-11 from one tree with one set of inputs, in 2352-byte sectors:
#
#   default, no Quake   86928   19:19:03   195.0 MiB
#   default             96404   21:25:29   216.2 MiB
#   HL, no Quake       291416   64:45:41   653.7 MiB
#   HL                 300892   66:51:67   674.9 MiB
#
# Quake costs 9476 sectors either way. The HL pressing lands at 83.6% of an
# 80-minute CD-R (359999 sectors), 59107 spare, and still fits a 74-minute
# blank with 32108 to spare. Nothing had to be dropped to make room.
HL ?=
# HL=0 means off, not "0 is a non-empty string, so on".
override HL := $(filter-out 0,$(HL))

# Quake 1.06 shareware Episode 1 is on every pressing. The source revision,
# shipping provenance, and all artifact hashes are checked before layout, and
# the combined image gets a machine-readable provenance receipt beside it, so
# a disc that carries Quake cannot stop saying exactly which Quake it carries.
#
# The owner approved public non-commercial distribution of the canonical
# Quake 1.06 shareware payload on 2026-08-25. The public upload paths still
# refuse the Half-Life pressing; only the standard disc may be published.
#
# The default paths name the canonical checkout. The expected revision,
# sidecar and hashes still gate use; current main may require a deliberate
# repin. The verifier fails closed if any one differs.
QUAKE_SRC ?= $(abspath $(ROOT)/../quake-psx)
QUAKE_CUE ?= $(QUAKE_SRC)/dist/quake-psx.cue
QUAKE_PROVENANCE ?= $(patsubst %.cue,%.provenance.json,$(QUAKE_CUE))
QUAKE_EXPECTED_REV ?= 96a7a0977c7f92ac8f87b9d0a213d28be553c33a
QUAKE_EXPECTED_PSOXIDE_REV ?= 0946dc892e0e86f190bbb6476874a250935fd5aa
QUAKE_EXPECTED_PROVENANCE_SHA256 ?= 9704a176b93bda2a70f19b96f657cd0530203209f25c8496b1cb3129cc0b298d
QUAKE_EXPECTED_CUE_SHA256 ?= 5fa78b12b506d4190246e230183e1eebd677f201ff982a584bff10d88ee2594c
QUAKE_EXPECTED_BIN_SHA256 ?= 711ffd1f1b41ebf6f9fa35b894307acca0c0da9e13ccdc25cf530583a95a33a5
QUAKE_EXPECTED_EXE_SHA256 ?= c61ca4a58200874e4d2779a2a685968591b313c551d30882d1722f09a81b4b14
QUAKE_VERSION := q$(shell printf '%.7s' '$(QUAKE_EXPECTED_REV)')
FRONTEND ?= $(EMULATOR)/target/release/frontend
HLPSX_SOURCE ?= $(GAMES)/hl-psx

# The disc lands directly in PSoXide's game library as <library>/<Name>.{bin,cue},
# no per-disc subfolder (Manny, 2026-09-03).
ifneq ($(HL),)
DISC_NAME ?= PSoXide Demo Disc HL
else
DISC_NAME ?= PSoXide Demo Disc
endif
PSOXIDE_LIB ?= $(HOME)/Downloads/ps1 games
DIST ?= $(PSOXIDE_LIB)
RELEASE_RECEIPT ?= $(DIST)/$(DISC_NAME).release-receipt.json
RELEASE_BUILD_COMMAND ?= make disc HL=1 DIST=$(DIST)

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
ARCADE   := $(GAMES)/psoxide-arcade/dist/psoxide-arcade.cue
HLPSX    := $(GAMES)/hl-psx/dist/hl-psx.cue
CORTEX_CURRENT_SOURCE := $(CORTEX_CURRENT_PSOXIDE)/editor/projects/default
CORTEX_CURRENT_PROJECT := $(BUILD)/cortex-current-04b
CORTEX_CURRENT := $(CORTEX_CURRENT_PROJECT)/baked/cortex_ignition_tech_demo_0_4b.cue
CORTEX_CURRENT_MAP := $(CORTEX_CURRENT_PROJECT)/baked/cortex_ignition_tech_demo_0_4b.map
CORTEX_CURRENT_REV_STAMP := $(CORTEX_CURRENT_PROJECT)/baked/.psoxide-revision
HWTESTS  := $(EXAMPLES)/hardware-tests.cue

NITROXIDE_SRC ?= $(GAMES)/nitroxide
NITROXIDE_BUILD := $(BUILD)/nitroxide
NITROXIDE       := $(NITROXIDE_BUILD)/NitroXide/NitroXide.cue

help:
	@echo "make disc             - build everything into \"$(DIST)\" (the default pressing, Quake shareware included)"
	@echo "make disc HL=1        - the same disc plus Half-Life, for the Palermo Comicon"
	@echo "make disc-only        - relay out the disc without rebuilding the programs"
	@echo "make check            - host tests (disc-toc, mkdisc) and the Quake pin check"
	@echo "make quake-verify     - check the pinned Quake input on its own"
	@echo "make quake-repin      - print the pin values a built Quake tree implies"
	@echo "make quake-headless-check - prove the default disc chain-loads Quake twice without images"
	@echo "make program-headless-check - boot the independent games and all three Arcade guests"
	@echo "make release-headless-check - build the private HL pressing and deterministically chain-load the release-critical entries"
	@echo "make relocation-check - disc that proves a relocated game still finds its data"
	@echo "make clean            - drop build/ (the disc in the library is left alone)"

loader:
	cd loader && CARGO_TARGET_DIR=$(BUILD) \
		RUSTFLAGS="-Cllvm-args=-disable-mips-df-backward-search -Clink-arg=-Tloader.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

# Each program's own version, read from the source that declares it rather than
# restated here. The disc could always say which pressing it was, but not which
# build of anything it carried: the programs come from several repositories at
# different moments, so the header's tag answered a question nobody was asking
# when one game looked wrong.
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
V_ARCADE    := $(shell awk '/^VERSION :=/{print $$3; exit}' $(GAMES)/psoxide-arcade/Makefile 2>/dev/null)
V_HLPSX     := $(call cargo_version,$(GAMES)/hl-psx/game/Cargo.toml)
# The hardware suite already versions itself on screen; take that same string so
# the carousel and the suite header cannot disagree.
V_HWTESTS   := $(shell awk -F'"' '/SUITE_VERSION: &str/{print $$2; exit}' $(PROGRAMS_PSOXIDE)/engine/examples/hardware-tests/src/main.rs 2>/dev/null | sed 's/HWTEST v//')
# Build revisions are recorded in the release receipt.
V_CORTEX_CURRENT := Tech demo

# Which pressing this is, drawn in the launcher's header. Tag a burn
# (`git tag v0.3 && make disc`) and the disc identifies itself on camera.
# Only v* tags name pressings; the rolling `web-disc` release tag that feeds
# the browser emulator would otherwise leak into every version string.
DISC_VERSION := $(shell git -C $(ROOT) describe --tags --match 'v*' --always --dirty 2>/dev/null)

# The launcher embeds the blob, so it always rebuilds after it.
launcher: loader
	cd launcher && CARGO_TARGET_DIR=$(BUILD) LOADER_BLOB=$(LOADER_EXE) DISC_VERSION=$(DISC_VERSION) \
		RUSTFLAGS="-Cllvm-args=-disable-mips-df-backward-search -Clink-arg=-T$(SDK)/sdk/psoxide.ld -Clink-arg=--oformat=binary" \
		cargo build $(PSX_FLAGS)

examples:
	$(MAKE) -C $(PROGRAMS_PSOXIDE) hardware-tests-disc \
		ENGINE_EXAMPLE_CARGO_ENV='CARGO_TARGET_DIR=$(PROGRAMS_PSOXIDE)/build/examples RUSTFLAGS="-Cllvm-args=-disable-mips-df-backward-search -Clink-arg=-T../../../sdk/psoxide.ld -Clink-arg=--oformat=binary"'
	python3 $(SDK)/tools/hazard_scan.py $(EXAMPLES)/hardware-tests.exe

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
# pin is overridden here and all ordinary non-Cortex programs come off the
# shared submodule.
# hl-psx's --psoxide below is the same idea under an older spelling.
programs: examples
	$(MAKE) -C $(GAMES)/voxide disc PSOXIDE_FROM=$(PROGRAMS_PSOXIDE) \
		DIST=$(GAMES)/voxide/dist GAMES_DIR=$(BUILD)/game-library
	$(MAKE) -C $(NITROXIDE_SRC) disc PSOXIDE_FROM=$(PROGRAMS_PSOXIDE) GAMES_DIR=$(NITROXIDE_BUILD)
	$(MAKE) -C $(GAMES)/psxcel build PSOXIDE_FROM=$(PROGRAMS_PSOXIDE)
	$(MAKE) -C $(GAMES)/pico8-psx collection PSOXIDE_FROM=$(PROGRAMS_PSOXIDE)
	$(MAKE) -C $(GAMES)/gh-psx disc PSOXIDE_FROM=$(PROGRAMS_PSOXIDE) DIST=$(GAMES)/gh-psx/dist \
		CDDA_LIST=$(ROOT)/audio/no-cdda.txt
	$(MAKE) -C $(GAMES)/psoxide-arcade disc PSOXIDE_FROM=$(PROGRAMS_PSOXIDE) \
		DIST=$(GAMES)/psoxide-arcade/dist
	@$(MAKE) cortex-if-stale
ifneq ($(HL),)
	cd $(GAMES)/hl-psx && cargo run --release -- assets --psoxide $(PROGRAMS_PSOXIDE)
	cd $(GAMES)/hl-psx && cargo run --release -- pack --psoxide $(PROGRAMS_PSOXIDE)
endif

# Every disc build must establish that its ordinary demo-disc programs were
# rebuilt from their own exact clean shared-runtime revision. Quake's separate
# artifact pin is verified independently below.
# Remove the prior stamp before rebuilding, check the hydration markers after
# every program recipe finishes, then write the exact clean checkout revision.
#
# The name is historical: this was the Quake lane's extra step back when Quake
# was opt-in. Quake ships on every pressing now, so it is simply how the disc's
# programs get built, and the stamp is what binds them to one clean SDK.
quake-programs:
	@rm -f "$(QUAKE_PROGRAMS_STAMP)"
	$(MAKE) programs
	$(MAKE) sdk-coherence
	@revision=$$(git -C "$(PROGRAMS_PSOXIDE)" rev-parse --verify 'HEAD^{commit}') || exit 1; \
	dirty=$$(git -C "$(PROGRAMS_PSOXIDE)" status --porcelain=v1 --untracked-files=normal) || exit 1; \
	if [ -n "$$dirty" ]; then \
		echo "quake-programs: PSoXide checkout is dirty; refusing SDK stamp"; \
		exit 1; \
	fi; \
	mkdir -p "$(BUILD)" || exit 1; \
	temporary="$(QUAKE_PROGRAMS_STAMP).tmp"; \
	printf '%s\n' "$$revision" > "$$temporary" || exit 1; \
	mv "$$temporary" "$(QUAKE_PROGRAMS_STAMP)"

# disc-only deliberately reuses binaries. That is permitted only after
# quake-programs recorded this checkout's full revision. This also makes
# quake-headless-check fail closed instead of replaying stale ordinary games.
quake-programs-verify:
	@if [ ! -f "$(QUAKE_PROGRAMS_STAMP)" ]; then \
		echo "quake-programs: missing $(QUAKE_PROGRAMS_STAMP); run 'make disc'"; \
		exit 1; \
	fi; \
	lines=$$(wc -l < "$(QUAKE_PROGRAMS_STAMP)" | tr -d '[:space:]'); \
	stamped=$$(sed -n '1p' "$(QUAKE_PROGRAMS_STAMP)"); \
	if [ "$$lines" != 1 ] || ! printf '%s\n' "$$stamped" | grep -Eq '^[0-9a-f]{40}$$'; then \
		echo "quake-programs: malformed SDK revision stamp $(QUAKE_PROGRAMS_STAMP)"; \
		exit 1; \
	fi; \
	current=$$(git -C "$(PROGRAMS_PSOXIDE)" rev-parse --verify 'HEAD^{commit}') || exit 1; \
	if [ "$$stamped" != "$$current" ]; then \
		echo "quake-programs: SDK revision stamp is $$stamped, but PSoXide is $$current; run 'make disc'"; \
		exit 1; \
	fi

# Bake Cortex only when its tracked project or exact engine pin changed. The
# project is copied under build/ before cooking, so generated packs and baked
# images never dirty the PSoXide submodule. The complete
# image is what mkdisc consumes: UI pack, world pack, CDDA and scene-residency
# layout all remain exactly as build-project-disc authored them.
CORTEX_FORCE   ?=

.PHONY: cortex-if-stale cortex-current-if-stale cortex-symbol-check
cortex-if-stale: cortex-current-if-stale

# Replacing the project deletes its linker map. Invalidate the staged guest
# executable too, so Cargo relinks and emits a matching map for the symbol gate.
cortex-current-if-stale:
	@current_rev=$$(git -C "$(CORTEX_CURRENT_PSOXIDE)" rev-parse --verify 'HEAD^{commit}') || exit 1; \
	dirty=$$(git -C "$(CORTEX_CURRENT_PSOXIDE)" status --porcelain=v1 --untracked-files=normal) || exit 1; \
	if [ "$$current_rev" != "$(CORTEX_CURRENT_EXPECTED_PSOXIDE_REV)" ]; then \
		echo "cortex-current: PSoXide pin is $$current_rev, expected $(CORTEX_CURRENT_EXPECTED_PSOXIDE_REV)"; \
		exit 1; \
	fi; \
	if [ -n "$$dirty" ]; then \
		echo "cortex-current: pinned PSoXide checkout is dirty"; \
		exit 1; \
	fi; \
	stamped_rev=$$(sed -n '1p' "$(CORTEX_CURRENT_REV_STAMP)" 2>/dev/null || true); \
	if [ -n "$(CORTEX_FORCE)" ] || [ ! -f "$(CORTEX_CURRENT)" ] || [ ! -s "$(CORTEX_CURRENT_MAP)" ] || [ "$$stamped_rev" != "$$current_rev" ] || [ -n "$$(find "$(CORTEX_CURRENT_SOURCE)/" -type f -newer "$(CORTEX_CURRENT)" -print -quit 2>/dev/null)" ]; then \
		echo "cortex-current: project or PSoXide revision changed (or forced) -- baking"; \
		case "$(BUILD)" in ""|"/") echo "cortex-current: unsafe build root $(BUILD)"; exit 1 ;; esac; \
		case "$(CORTEX_CURRENT_PROJECT)" in "$(BUILD)"/*) ;; *) echo "cortex-current: unsafe staging path $(CORTEX_CURRENT_PROJECT)"; exit 1 ;; esac; \
		rm -rf "$(CORTEX_CURRENT_PROJECT)" || exit 1; \
		mkdir -p "$(CORTEX_CURRENT_PROJECT)" || exit 1; \
		cp -R "$(CORTEX_CURRENT_SOURCE)/." "$(CORTEX_CURRENT_PROJECT)/" || exit 1; \
		if [ -f "$(CORTEX_CURRENT_GUEST_STAGE_ROOT)/stage/engine/examples/editor-playtest/Cargo.toml" ]; then \
			CARGO_HOME="$(CORTEX_GUEST_CARGO_HOME)" cargo clean --offline --release --target mipsel-sony-psx -Zjson-target-spec --package editor-playtest --manifest-path "$(CORTEX_CURRENT_GUEST_STAGE_ROOT)/stage/engine/examples/editor-playtest/Cargo.toml" --target-dir "$(CORTEX_CURRENT_GUEST_STAGE_ROOT)/stage/build/examples" || exit 1; \
		fi; \
		(cd "$(CORTEX_CURRENT_PSOXIDE)/emu" && PSOXIDE_GUEST_STAGE_ROOT="$(CORTEX_CURRENT_GUEST_STAGE_ROOT)" PSOXIDE_GUEST_CARGO_HOME="$(CORTEX_GUEST_CARGO_HOME)" PSOXIDE_GUEST_LINK_MAP="$(CORTEX_CURRENT_MAP)" cargo run -p frontend --release -- build-project-disc --project "$(CORTEX_CURRENT_PROJECT)") || exit 1; \
		printf '%s\n' "$$current_rev" > "$(CORTEX_CURRENT_REV_STAMP)" || exit 1; \
	else \
		echo "cortex-current: project unchanged -- reusing $(CORTEX_CURRENT)"; \
	fi
	$(MAKE) cortex-symbol-check

cortex-symbol-check:
	@test -s "$(CORTEX_CURRENT_MAP)" || { echo "cortex-current: missing or empty guest linker map: $(CORTEX_CURRENT_MAP)" >&2; exit 1; }
	sh "$(CORTEX_CURRENT_PSOXIDE)/tools/guest_symbol_gate.sh" "$(CORTEX_CURRENT_MAP)"

mkdisc:
	cd tools/mkdisc && cargo build --release

# Cortex Ignition is on the carousel of both pressings since 2026-09-03 (it
# used to sit behind the Konami unlock on the standard one). The launcher's
# unlock sequence still works; nothing is gated by default.
CORTEX_GATE_ARGS =

# The three HALF-LIFE arguments travel together: an image without its version
# and description would press, but announce itself wrong.
ifneq ($(HL),)
HL_ARGS = --image "HALF-LIFE=$(HLPSX)" \
	--shot "HALF-LIFE=$(SHOTS_OUT)/halflife.shot" \
	--version-of "HALF-LIFE=$(V_HLPSX)" \
	--describe "HALF-LIFE=A from-scratch PlayStation port of Half-Life. The full campaign has been converted and much of the game works, but it is not yet playable from start to finish.|Half-Life portato su PlayStation da zero. L'intera campagna e stata convertita e gran parte del gioco funziona, ma non e ancora giocabile dall'inizio alla fine."
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
QUAKE_PREREQS = quake-programs-verify quake-verify

disc: launcher quake-programs mkdisc
	$(MAKE) disc-only

quake-headless-check:
	$(MAKE) disc-only
	$(MAKE) _quake-headless-check

# Smoke-test every independently maintained program outside the three large
# ports. The script reads the pressed table and navigates by name, so adding or
# reordering carousel cards does not silently point a route at the wrong game.
# It also enters the Arcade collection and starts each of its three guests.
program-headless-check:
	python3 tools/check_program_headless.py \
		--frontend "$(FRONTEND)" \
		--cue "$(DIST)/$(DISC_NAME).cue" \
		--out "$(BUILD)/program-headless" \
		--jobs 3

_quake-headless-check:
	python3 tools/check_release_chainloads.py \
		--frontend "$(FRONTEND)" \
		--cue "$(DIST)/$(DISC_NAME).cue" \
		--target "QUAKE SHAREWARE"

# Burn gate for the one-disc private pressing.  Rebuild first: a release gate
# that accepted `disc-only` could prove a perfectly deterministic stale guest.
# Run both recursive makes with HL=1 so DISC_NAME/DIST and the carousel layout
# are computed in the same variant that verifies them.  The checker launches
# each release-critical entry twice and requires byte-identical route/CD/GPU/PC
# logs as well as a final PC and sampled execution inside that entry's
# checksummed PS-X EXE.
release-frontend:
	cd $(EMULATOR) && cargo build --locked --release -p frontend

release-headless-check: release-frontend
	$(MAKE) disc HL=1
	$(MAKE) _release-headless-check HL=1

_release-headless-check:
	python3 tools/release_receipt.py verify --receipt "$(RELEASE_RECEIPT)"
	python3 tools/check_release_chainloads.py \
		--frontend "$(FRONTEND)" \
		--cue "$(DIST)/$(DISC_NAME).cue"

# Repin. The six QUAKE_EXPECTED_* values above and the PSoXide submodule
# pointer are the whole contract, and they all come out of a built Quake tree:
#
#   make quake-repin                          # from the default QUAKE_SRC
#   make quake-repin QUAKE_SRC=/path/to/tree  # from somewhere else
#
# It prints the six lines to paste over the ones above, plus the commands that
# follow them, and writes nothing. Editing by hand is the point: the diff then
# shows exactly which contract moved, and a repin that rewrote the pins itself
# would be a verifier agreeing with whatever it was handed. README.md has the
# full procedure. The chain-load gate resolves Quake by its pressed name.
quake-repin:
	@python3 tools/quake_disc.py repin \
		--source "$(QUAKE_SRC)" \
		--cue "$(QUAKE_CUE)" \
		--provenance "$(QUAKE_PROVENANCE)"

quake-verify:
	python3 tools/quake_disc.py verify \
		--source "$(QUAKE_SRC)" \
		--psoxide "$(PSOXIDE)" \
		--programs-psoxide "$(PROGRAMS_PSOXIDE)" \
		--programs-psoxide-stamp "$(QUAKE_PROGRAMS_STAMP)" \
		--cue "$(QUAKE_CUE)" \
		--provenance "$(QUAKE_PROVENANCE)" \
		--expected-revision "$(QUAKE_EXPECTED_REV)" \
		--expected-psoxide-revision "$(QUAKE_EXPECTED_PSOXIDE_REV)" \
		--expected-programs-psoxide-revision "$(PROGRAMS_EXPECTED_PSOXIDE_REV)" \
		--expected-provenance-sha256 "$(QUAKE_EXPECTED_PROVENANCE_SHA256)" \
		--expected-cue-sha256 "$(QUAKE_EXPECTED_CUE_SHA256)" \
		--expected-bin-sha256 "$(QUAKE_EXPECTED_BIN_SHA256)" \
		--expected-exe-sha256 "$(QUAKE_EXPECTED_EXE_SHA256)"

# Menu backdrops: the same in-game captures the itch pages use, cooked from
# assets/shots into the 8bpp CLUT blobs the launcher uploads to VRAM.
# Cooked at build time (needs PIL, like the other tools) so the pressed
# pixels always come from the PNGs actually in the repo.
SHOTS_SRC := $(ROOT)/assets/shots
SHOTS_OUT := $(BUILD)/shots
SHOT_NAMES := cortex-current-menu cortex-current-gameplay \
              voxide-day voxide-night nitroxide-boost \
              nitroxide-aerial nitroxide-goal celeste celeste2 psxcel-chart \
              psxcel-editing ghpsx ghpsx2 breakout breakout2 invaders \
              invaders2 pong pong2 hwtests hwtests2 halflife quake-menu \
              quake-gameplay
SHOT_FILES := $(foreach n,$(SHOT_NAMES),$(SHOTS_OUT)/$(n).shot)

$(SHOTS_OUT)/%.shot: $(SHOTS_SRC)/%.png tools/cook-shots.py
	@mkdir -p "$(SHOTS_OUT)"
	python3 tools/cook-shots.py "$<" "$@"

# Just the layout, for when nothing but the text or the audio changed. Also
# the one place the mkdisc invocation lives, so it cannot drift from what
# `make disc` builds.
ifneq ($(HL),)
disc-only: release-frontend
endif
disc-only: mkdisc $(SHOT_FILES) $(QUAKE_PREREQS)
	@mkdir -p "$(DIST)"
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out "$(DIST)/$(DISC_NAME).bin" --volume PSXDEMO \
		--image "CORTEX IGNITION=$(CORTEX_CURRENT)" \
		$(QUAKE_ARGS) \
		$(HL_ARGS) \
		--image "VOXIDE=$(VOXIDE)" \
		--image "NITROXIDE=$(NITROXIDE)" \
		--game "CELESTE COLLECTION=$(CELESTE)" \
		--image "PSOXIDE ARCADE=$(ARCADE)" \
		--image "GH-PSX=$(GHPSX)" \
		--game "PSXCEL=$(PSXCEL)" \
		--image "HARDWARE TESTS=$(HWTESTS)" \
		$(foreach t,$(MENU_CDDA),--menu-cdda "$(t)") \
		$(foreach b,$(MENU_BEATS),--menu-beat $(b)) \
		--menu-title "KNUCKLE DUST" --menu-title "RUSTED HAMMER" \
		--menu-title "CHAINSAW HEART" --menu-title "NIGHT CRAWLER" \
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
		--shot "PSXCEL=$(SHOTS_OUT)/psxcel-chart.shot" \
		--shot "PSXCEL=$(SHOTS_OUT)/psxcel-editing.shot" \
		--shot "GH-PSX=$(SHOTS_OUT)/ghpsx.shot" \
		--shot "GH-PSX=$(SHOTS_OUT)/ghpsx2.shot" \
		--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/breakout.shot" \
		--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/breakout2.shot" \
		--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/invaders.shot" \
		--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/invaders2.shot" \
		--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/pong.shot" \
		--shot "PSOXIDE ARCADE=$(SHOTS_OUT)/pong2.shot" \
		--shot "HARDWARE TESTS=$(SHOTS_OUT)/hwtests.shot" \
		--shot "HARDWARE TESTS=$(SHOTS_OUT)/hwtests2.shot" \
		--share-cdda "GH-PSX=PSOXIDE ARCADE" \
		--version-of "CORTEX IGNITION=$(V_CORTEX_CURRENT)" \
		--version-of "VOXIDE=$(V_VOXIDE)" \
		--version-of "NITROXIDE=$(V_NITROXIDE)" \
		--version-of "PSXCEL=$(V_PSXCEL)" \
		--version-of "CELESTE COLLECTION=$(V_CELESTE)" \
		--version-of "GH-PSX=$(V_GHPSX)" \
		--version-of "PSOXIDE ARCADE=$(V_ARCADE)" \
		--version-of "HARDWARE TESTS=$(V_HWTESTS)" \
		$(CORTEX_GATE_ARGS) \
		--describe "CORTEX IGNITION=A Souls-like built from the ground up for the original PlayStation. This is an early tech demo.|Un souls-like sviluppato da zero per la prima PlayStation. Questo e un primo tech demo." \
		--describe "VOXIDE=A Minecraft clone built for the original PlayStation. This is an early playable build: world generation, mining, crafting and survival work.|Un clone di Minecraft per la prima PlayStation. Prima versione giocabile: generazione del mondo, scavo, crafting e sopravvivenza funzionano." \
		--describe "NITROXIDE=A Rocket League clone built for the original PlayStation. Supports 2 players in split screen.|Un clone di Rocket League per la prima PlayStation. Supporta 2 giocatori a schermo diviso." \
		--describe "CELESTE COLLECTION=Both Celeste Classic games, rebuilt from PICO-8 as native PlayStation games.|I due Celeste Classic, ricostruiti da PICO-8 come giochi nativi PlayStation." \
		--describe "PSXCEL=A working Microsoft Excel clone for the original PlayStation, controlled with a joypad. This build is fully functional, with formulas, charts, themes and memory-card saves.|Un clone funzionante di Microsoft Excel per PlayStation, controllato col joypad. Questa versione e completa e include formule, grafici, temi e salvataggi su memory card." \
		--describe "GH-PSX=A Guitar Hero-style rhythm game for the original PlayStation. This is a bare-bones, one-song prototype.|Un gioco in stile Guitar Hero per la prima PlayStation. E un prototipo essenziale con una sola canzone." \
		--describe "PSOXIDE ARCADE=Three complete native PlayStation arcade games in one collection: Breakout, Space Invaders and Magikarp Pong, with its own live CD-audio visualizer.|Tre giochi arcade completi e nativi per PlayStation in una raccolta: Breakout, Space Invaders e Magikarp Pong, con visualizzatore CD audio." \
		--describe "HARDWARE TESTS=A hardware test suite, not a game. The current suite is working and ready to use, displaying real PlayStation measurements as photo-ready codes for checking emulator accuracy.|Una suite di test hardware, non un gioco. E funzionante e pronta all'uso: mostra le misure della vera PlayStation come codici da fotografare per verificare la precisione degli emulatori." \

	python3 tools/quake_disc.py receipt \
		--source "$(QUAKE_SRC)" \
		--psoxide "$(PSOXIDE)" \
		--programs-psoxide "$(PROGRAMS_PSOXIDE)" \
		--programs-psoxide-stamp "$(QUAKE_PROGRAMS_STAMP)" \
		--cue "$(QUAKE_CUE)" \
		--provenance "$(QUAKE_PROVENANCE)" \
		--expected-revision "$(QUAKE_EXPECTED_REV)" \
		--expected-psoxide-revision "$(QUAKE_EXPECTED_PSOXIDE_REV)" \
		--expected-programs-psoxide-revision "$(PROGRAMS_EXPECTED_PSOXIDE_REV)" \
		--expected-provenance-sha256 "$(QUAKE_EXPECTED_PROVENANCE_SHA256)" \
		--expected-cue-sha256 "$(QUAKE_EXPECTED_CUE_SHA256)" \
		--expected-bin-sha256 "$(QUAKE_EXPECTED_BIN_SHA256)" \
		--expected-exe-sha256 "$(QUAKE_EXPECTED_EXE_SHA256)" \
		--demo-cue "$(DIST)/$(DISC_NAME).cue" \
		--demo-bin "$(DIST)/$(DISC_NAME).bin" \
		--out "$(DIST)/$(DISC_NAME).quake-provenance.json"

ifneq ($(HL),)
	python3 tools/release_receipt.py create \
		--combined-cue "$(DIST)/$(DISC_NAME).cue" \
		--frontend "$(FRONTEND)" \
		--build-command "$(RELEASE_BUILD_COMMAND)" \
		--program "CORTEX IGNITION=$(CORTEX_CURRENT)" \
		--source "CORTEX IGNITION=$(CORTEX_CURRENT_PSOXIDE)" \
		--program "HALF-LIFE=$(HLPSX)" \
		--source "HALF-LIFE=$(HLPSX_SOURCE)" \
		--program "HARDWARE TESTS=$(HWTESTS)" \
		--source "HARDWARE TESTS=$(PROGRAMS_PSOXIDE)" \
		--program "QUAKE SHAREWARE=$(QUAKE_CUE)" \
		--source "QUAKE SHAREWARE=$(QUAKE_SRC)" \
		--out "$(RELEASE_RECEIPT)"
endif
	python3 tools/components.py --check --receipt "$(DIST)/$(DISC_NAME).components.json" --cue "$(DIST)/$(DISC_NAME).cue" --frontend "$(FRONTEND)"


# The standard pressing may be published. Both upload recipes fail closed when
# HL is set so the private Half-Life pressing cannot reach either public path.

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
	@command -v butler >/dev/null || { echo "itch: install butler and run 'butler login' first"; exit 1; }
	$(MAKE) disc
	@rm -rf "$(BUILD)/itch" && mkdir -p "$(BUILD)/itch"
	cp "$(DIST)/$(DISC_NAME).bin" "$(DIST)/$(DISC_NAME).cue" release/README.txt "$(BUILD)/itch/"
	butler push --userversion "$(DISC_VERSION)" "$(BUILD)/itch" bonnie-studios/psoxide-demo-disc:psx

# The Quake pin check is part of the ordinary check, not a lane of its own: a
# tree whose pinned Quake payload is missing, stale, dirty, or built against a
# different PSoXide cannot press a disc, so it should not pass its tests either.
# The chain-load half of the proof needs a built disc and lives in
# `make quake-headless-check`.
check: sdk-on-main sdk-coherence check-locks quake-verify
	cargo test --manifest-path games/PSoXide-editor/engine/Cargo.toml -p psx-carousel
	cargo test --manifest-path games/PSoXide-editor/engine/Cargo.toml -p psx-disc-toc
	cd tools/mkdisc && cargo test
	python3 -m unittest discover -s tools -p 'test_quake_disc.py'
	python3 -m unittest discover -s tools -p 'test_components.py'
	python3 -m unittest tools/test_release_receipt.py tools/test_release_chainloads.py tools/test_check_program_headless.py

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

# The ordinary-program SDK and Quake's declared PSoXide
# revision must both be on PSoXide main. Quake's SDK work once lived on a side
# branch that this disc pinned as the SDK while main moved on; the split cost a
# reconciliation merge. GitHub's compare API answers "identical" or "ahead"
# when main contains the revision. DEMO_DISC_ALLOW_PSOXIDE_OFF_MAIN=1 skips the
# check for a deliberate side-branch pressing and says so.
.PHONY: sdk-on-main
sdk-on-main: components
	@python3 tools/components.py --check-main

.PHONY: sdk-coherence
sdk-coherence:
	python3 tools/components.py --check --games $(if $(HL),--hl,)

# hello-pack streams WORLD.PAK off the disc and paints ALL PASS or a failure
# list, which makes it the end-to-end test for the relocation machinery: its
# image lands 200-odd sectors in, every LBA it was cooked with is wrong by that
# much, and psx_io::disc_base has to make up the difference. Run the result
# with the emulator and read the banner.
relocation-check: launcher mkdisc
	$(MAKE) -C $(PROGRAMS_PSOXIDE) hello-tri hello-pack-disc
	@mkdir -p $(ROOT)/dist
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out $(ROOT)/dist/relocation.bin --volume PSXRELOC \
		--game "HELLO TRI=$(EXAMPLES)/hello-tri.exe" \
		--image "HELLO PACK=$(EXAMPLES)/hello-pack.cue"
	@echo
	@echo "Now: cd $(PROGRAMS_PSOXIDE)/emu && cargo run -p frontend --release -- launch \\"
	@echo "       --path $(ROOT)/dist/relocation.cue --steps 200000000 \\"
	@echo "       --press '250:right:8,320:cross:8' --dump-hw /tmp/relocation.ppm"
	@echo "The dumped frame must read ALL PASS."

clean:
	rm -rf $(BUILD) $(ROOT)/dist

# Fetching is explicit and pinned. Both authoring and validation use their
# own repositories; ordinary guests receive the bootstrapped engine tree.
.PHONY: components verify-components
components:
	python3 tools/components.py
verify-components:
	python3 tools/components.py --check
loader examples programs mkdisc release-frontend cortex-current-if-stale sdk-coherence: components
