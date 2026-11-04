#!/usr/bin/env bash
# Build every module under modules/ for wasm32-unknown-unknown into target/wasm/<name>.wasm.
# Modules depend on crates/vy-sdk by path; each has its own Cargo.lock and target dir.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/wasm
for dir in modules/*/; do
  name=$(basename "$dir")
  (cd "$dir" && cargo build --release --target wasm32-unknown-unknown --quiet)
  cp "$dir/target/wasm32-unknown-unknown/release/${name}.wasm" "target/wasm/${name}.wasm"
  # `wc -c` rather than `stat`, whose flags differ between GNU and BSD.
  printf '%s: %s bytes\n' "$name" "$(wc -c < "target/wasm/${name}.wasm" | tr -d ' ')"
done
