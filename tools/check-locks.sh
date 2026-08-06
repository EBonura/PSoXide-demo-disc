#!/bin/bash
# Every tracked Cargo.lock must resolve without Cargo wanting to change it, and
# every PSoXide pin must name the revision its lockfile actually resolved.
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
REPOS=(. games/PSoXide games/nitroxide games/voxide games/pico8-psx games/psxcel games/gh-psx games/hl-psx)
offline="--offline"
[ "${CHECK_LOCKS_ONLINE:-}" = "1" ] && offline=""

pass=0; fail=0
for repo in "${REPOS[@]}"; do
  while IFS= read -r lock; do
    [ -z "$lock" ] && continue
    manifest="$repo/${lock%Cargo.lock}Cargo.toml"
    [ -f "$manifest" ] || continue
    if cargo metadata --manifest-path "$manifest" --format-version 1 --locked $offline >/dev/null 2>&1; then
      pass=$((pass + 1))
    else
      echo "check-locks: stale lock, cargo wants to change it: $repo/$lock"
      fail=$((fail + 1))
    fi
  done < <(git -C "$repo" ls-files '*Cargo.lock' 2>/dev/null)
done

# A pin crate can name one revision in its manifest and resolve another in its
# lock. That compiles, and silently builds a game against an SDK nobody chose.
for game in nitroxide voxide pico8-psx psxcel gh-psx; do
  manifest="games/$game/psoxide-pin/Cargo.toml"
  lock="games/$game/psoxide-pin/Cargo.lock"
  [ -f "$manifest" ] || continue
  want=$(grep -oE '[a-f0-9]{40}' "$manifest" | head -1)
  got=$(grep -oE 'rev=[a-f0-9]{40}' "$lock" 2>/dev/null | head -1 | cut -d= -f2)
  if [ "$want" != "$got" ]; then
    echo "check-locks: $game pins ${want:0:8} but its lock resolved ${got:0:8}"
    fail=$((fail + 1))
  fi
  # And the constant the bootstrap stamps the hydrated tree with has to agree,
  # or an unchanged pin silently skips the copy and reuses the wrong tree.
  code=$(grep -oE '[a-f0-9]{40}' "games/$game/psoxide-pin/src/main.rs" 2>/dev/null | head -1)
  if [ -n "$code" ] && [ "$code" != "$want" ]; then
    echo "check-locks: $game REV constant ${code:0:8} disagrees with its manifest ${want:0:8}"
    fail=$((fail + 1))
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "check-locks: $fail problem(s). Refresh with: cargo metadata --manifest-path <the manifest> --format-version 1"
  exit 1
fi
echo "check-locks: $pass tracked lock(s) reproducible, 5 pins consistent"
