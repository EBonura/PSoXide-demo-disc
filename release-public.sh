#!/bin/sh
# Release the public demo disc to both itch.io channels. See "Releasing to itch"
# in README.md. All arguments go to `disc-tools release`.
set -eu
cd "$(dirname "$0")"
exec cargo run --release -q --manifest-path tools/disc-tools/Cargo.toml -- release "$@"
