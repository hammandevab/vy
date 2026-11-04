#!/usr/bin/env bash
# Build every module under modules/ for wasm32-unknown-unknown into target/wasm/<name>.wasm.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/wasm
for dir in modules/*/; do
  name=$(basename "$dir")
  (cd "$dir" && cargo build --release --target wasm32-unknown-unknown --quiet)
  cp "$dir/target/wasm32-unknown-unknown/release/${name}.wasm" "target/wasm/${name}.wasm"
  printf '%s: %s bytes\n' "$name" "$(stat -c %s "target/wasm/${name}.wasm")"
done
