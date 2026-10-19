#!/bin/sh
# Prove that the chain loader hands the CD drive over quiet.
#
# A Pause answers twice, and on the console the second answer (the drive has
# stopped) comes 1.2 to 3.9 million system clocks after the first. A loader that
# stops its last read with a plain Pause and jumps leaves that INT2 to land
# inside the next program. This presses a one-entry disc whose only program is
# tools/chainload-probe, chain-loads it from the launcher on the emulator, and
# requires the probe's own verdict: no response latched at entry, none arriving
# in an 8 million clock watch with no command sent, an immediate read that works,
# and (the control that keeps the probe honest) a plain stop() that DOES leave an
# INT2 behind.
#
# The FRONTEND must carry the emulator's silicon Pause model (PSoXide-emulator
# dddd281 or later); an older one answers the second response within 30,000
# clocks and the control fails, which says so rather than passing.
#
# Run as: sh tools/chainload_probe.sh <launcher.exe> <frontend> [workdir]
# `make chainload-probe` builds the launcher and runs it. To see it fail, build the
# launcher over a games/PSoXide-editor whose engine/crates/psx-chainloader is
# from before the quiet handoff (editor e22f3508).
set -eu
LAUNCHER="${1:?usage: chainload_probe.sh <launcher.exe> <frontend> [workdir]}"
FRONTEND="${2:?missing frontend}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="${3:-$ROOT/build/chainload-probe}"
SDK="${SDK:-$ROOT/games/PSoXide-sdk}"
EDITOR="${EDITOR_TREE:-$ROOT/games/PSoXide-editor}"
MKDISC="$ROOT/tools/mkdisc/target/release/mkdisc"
mkdir -p "$WORK"

(cd "$ROOT/tools/chainload-probe" && CARGO_TARGET_DIR="$WORK/target" \
    RUSTFLAGS="-Cllvm-args=-disable-mips-df-backward-search -Clink-arg=-T$SDK/sdk/psoxide.ld -Clink-arg=--oformat=binary" \
    cargo build --release --target mipsel-sony-psx -Zbuild-std=core -Zbuild-std-features=compiler-builtins-mem)
PROBE="$WORK/target/mipsel-sony-psx/release/chainload-probe.exe"

# The probe reads its own image through the disc base, so it rides as an image.
# MKISOPSX names a built mkisopsx to use; otherwise it is built from the editor tree.
if [ -n "${MKISOPSX:-}" ]; then
    "$MKISOPSX" --exe "$PROBE" --out "$WORK/chainload-probe.bin" --volume CHAINPROBE >/dev/null
else
    (cd "$EDITOR" && CARGO_TARGET_DIR="$WORK/host" cargo run -q --release -p mkisopsx -- \
        --exe "$PROBE" --out "$WORK/chainload-probe.bin" --volume CHAINPROBE >/dev/null)
fi

# The launcher exe is a flat file the linker leaves short of the whole sectors its
# header counts; the frontend refuses a short one, so pad a copy.
cp "$LAUNCHER" "$WORK/launcher.exe"
payload=$(od -An -tu4 -j28 -N4 "$WORK/launcher.exe" | tr -d ' ')
size=$(wc -c <"$WORK/launcher.exe" | tr -d ' ')
if [ "$size" -lt $((2048 + payload)) ]; then truncate -s $((2048 + payload)) "$WORK/launcher.exe"; fi

"$MKDISC" --launcher "$WORK/launcher.exe" --out "$WORK/disc.bin" --volume PSXDEMO \
    --image "CHAIN PROBE=$WORK/chainload-probe.cue" \
    --menu-xa "$ROOT/audio/launcher/MUSIC1.XA" --menu-xa "$ROOT/audio/launcher/MUSIC2.XA" \
    --menu-spectrum "$ROOT/audio/launcher/menu.spectrum" \
    --menu-title "JUNGLE 1 A" --menu-title "JUNGLE 1 B" --menu-title "JUNGLE 2 A" \
    --menu-title "JUNGLE 2 B" --menu-title "JUNGLE DNB A" --menu-title "JUNGLE DNB B" \
    --credit "Music: Mulula VGM Loops, CC BY 4.0" --version-of "CHAIN PROBE=0.1.0" \
    --max-sectors 359999 >/dev/null

# One entry plus the credits card: CROSS on the first card chain-loads it.
"$FRONTEND" launch --embedded-playtest --path "$WORK/disc.cue" --steps 700000000 \
    --press "1000:cross:12" >"$WORK/out.txt" 2>&1 || {
    cat "$WORK/out.txt" >&2
    echo "chainload-probe: FAIL: the emulator exited with an error" >&2
    exit 1
}
grep -E '^(launcher: chain-loading|chainload-probe:)' "$WORK/out.txt"
if grep -q '^chainload-probe: PASS' "$WORK/out.txt"; then
    echo "chainload-probe-gate: PASS"
else
    echo "chainload-probe-gate: FAIL" >&2
    exit 1
fi
