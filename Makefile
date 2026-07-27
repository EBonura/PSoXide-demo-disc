# PSoXide demo disc.
#
# `make disc` builds the chain-load blob, the launcher, the programs, and lays
# them out into dist/demo.{bin,cue}. `make check` runs the host tests.
#
# Only the small PSoXide examples are wired up so far; the six full games need
# their per-disc base LBA / CD-DA track knobs first (see PLAN.md).

.PHONY: help disc loader launcher examples mkdisc check clean

ROOT       := $(CURDIR)
PSOXIDE    := $(ROOT)/games/PSoXide
BUILD      := $(ROOT)/build
OUT        := $(BUILD)/mipsel-sony-psx/release
DIST       := $(ROOT)/dist
EXAMPLES   := $(PSOXIDE)/build/examples/mipsel-sony-psx/release
MKDISC     := $(ROOT)/tools/mkdisc/target/release/mkdisc

PSX_TARGET  := mipsel-sony-psx
PSX_FLAGS   := --release --target $(PSX_TARGET) -Zbuild-std=core -Zbuild-std-features=compiler-builtins-mem
LOADER_EXE  := $(OUT)/loader.exe
LAUNCHER_EXE := $(OUT)/launcher.exe

help:
	@echo "make disc     - build everything and lay out dist/demo.{bin,cue}"
	@echo "make check    - host tests (disc-toc, mkdisc)"
	@echo "make clean    - drop build/ and dist/"

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

mkdisc:
	cd tools/mkdisc && cargo build --release

disc: launcher examples mkdisc
	@mkdir -p $(DIST)
	$(MKDISC) --launcher $(LAUNCHER_EXE) --out $(DIST)/demo.bin --volume PSXDEMO \
		--game "BREAKOUT=$(EXAMPLES)/game-breakout.exe" \
		--game "SPACE INVADERS=$(EXAMPLES)/game-invaders.exe" \
		--game "MAGIKAAAAARP PONG=$(EXAMPLES)/game-magikaaaaaarp-pong.exe"

check:
	cd disc-toc && cargo test
	cd tools/mkdisc && cargo test

clean:
	rm -rf $(BUILD) $(DIST)
