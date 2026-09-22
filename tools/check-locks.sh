#!/bin/bash
# Every tracked Cargo.lock must resolve without Cargo wanting to change it, and
# every game's components.lock.json must select the disc's component tuple.
#
# sdk-coherence proves the HYDRATED tree is consistent: it reads the marker
# psoxide-link leaves in each .psoxide. That says nothing about whether a fresh
# clone of one game can honour its own manifest and lock. A 2026-08-03 audit
# found 21 of 56 tracked locks stale while sdk-coherence was green, because the
# two checks answer different questions.
#
# Enumerate per repository, not from the parent index: `git ls-files` at the top
# level does not descend into submodules, and 16 of the 21 stale locks lived
# inside them.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
REPOS=(. games/PSoXide-editor games/PSoXide-emulator games/PSoXide-sdk games/nitroxide games/voxide games/pico8-psx games/psxcel games/gh-psx games/hl-psx games/psoxide-arcade)
offline="--offline"
[ "${CHECK_LOCKS_ONLINE:-}" = "1" ] && offline=""

pass=0; fail=0
diagnostics=$(mktemp)
trap 'rm -f "$diagnostics"' EXIT
for repo in "${REPOS[@]}"; do
  while IFS= read -r lock; do
    [ -z "$lock" ] && continue
    manifest="$repo/${lock%Cargo.lock}Cargo.toml"
    [ -f "$manifest" ] || continue
    if cargo metadata --manifest-path "$manifest" --format-version 1 --locked $offline >/dev/null 2>"$diagnostics"; then
      pass=$((pass + 1))
    else
      echo "check-locks: lock or dependency resolution failed: $repo/$lock"
      cat "$diagnostics"
      fail=$((fail + 1))
    fi
  done < <(git -C "$repo" ls-files '*Cargo.lock' 2>/dev/null)
done

python3 tools/components.py --check --game-locks || fail=$((fail + 1))

if [ "$fail" -ne 0 ]; then
  echo "check-locks: $fail problem(s). Refresh with: cargo metadata --manifest-path <the manifest> --format-version 1"
  exit 1
fi
echo "check-locks: $pass tracked lock(s) reproducible, every game lock agrees with the disc"
