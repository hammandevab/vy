#!/usr/bin/env bash
# Build Studio (modules/studio) for wasm32 and pack it as <out>, by default
# target/studio.vyp, where `vy-dev serve` finds it when run from a checkout
# (it looks next to its binary, then one directory up: target/debug/vy-dev and
# target/release/vy-dev both see target/studio.vyp). A release puts studio.vyp
# next to the vy-dev binary instead.
#
#   tools/build_studio.sh [out]
#
# The package is signed with a throwaway key: `vy-dev serve` writes its own
# origin and per-run token into the module, sets `net` to that origin and signs
# the result with the developer key every time it starts. So two builds differ
# only in the signature: their package identity (the unsigned manifest, which
# names the module's and resources' hashes; `vy-publish inspect`) is the same
# (SPEC-9.7.1). VY_PUBLISH names a vy-publish binary to use instead of
# `cargo run -p vy-publish`.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-target/studio.vyp}

# The same reproducible module build as every other page (SPEC-9.7.1).
tools/build_modules.sh studio > /dev/null
wasm=target/wasm/studio.wasm

if [ -n "${VY_PUBLISH:-}" ]; then
  publish=("$VY_PUBLISH")
else
  publish=(cargo run --quiet -p vy-publish --)
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"${publish[@]}" keygen --out "$tmp/throwaway.key" > /dev/null
mkdir -p "$(dirname "$out")"
"${publish[@]}" build --manifest conformance/manifests/studio.json --module "$wasm" \
  --key "$tmp/throwaway.key" --out "$out"
echo "wrote $out"
