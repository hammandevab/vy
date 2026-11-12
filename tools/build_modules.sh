#!/usr/bin/env bash
# Build every module under modules/ for wasm32-unknown-unknown into target/wasm/<name>.wasm,
# or only the ones named: tools/build_modules.sh [name...].
# Modules depend on crates/vy-sdk by path; each has its own Cargo.lock (used as it is,
# --locked) and target dir.
#
# The bytes are the same in any checkout of the same commit (SPEC-9.7.1, docs/RELEASING.md):
# - tools/repro_rustc.rs, compiled with plain rustc into target/tools/, is the RUSTC_WRAPPER:
#   cargo's `-C metadata` for crates/vy-sdk and crates/vy-abi (path dependencies outside each
#   module's own workspace) hashes their absolute path and so orders the functions in the
#   module; the wrapper gives it a value without the path.
# - tools/repro_env.sh: --remap-path-prefix turns the checkout's path into /vy in anything
#   rustc writes; RUSTFLAGS from the environment (CI's -D warnings) are kept.
set -euo pipefail
cd "$(dirname "$0")/.."
. tools/repro_env.sh

mkdir -p target/wasm target/tools
wrapper=target/tools/repro-rustc$exe
if [ ! -f "$wrapper" ] || [ tools/repro_rustc.rs -nt "$wrapper" ]; then
  rustc --edition 2021 -O --crate-name repro_rustc tools/repro_rustc.rs -o "$wrapper.$$$exe"
  mv -f "$wrapper.$$$exe" "$wrapper"
fi
export RUSTC_WRAPPER="$root/$wrapper"

if [ $# -eq 0 ]; then
  set -- $(for dir in modules/*/; do basename "$dir"; done)
fi
for name in "$@"; do
  dir=modules/$name
  [ -f "$dir/Cargo.toml" ] || { echo "error: no module $dir" >&2; exit 1; }
  (cd "$dir" && cargo build --release --locked --target wasm32-unknown-unknown --quiet)
  cp "$dir/target/wasm32-unknown-unknown/release/${name}.wasm" "target/wasm/${name}.wasm"
  # `wc -c` rather than `stat`, whose flags differ between GNU and BSD.
  printf '%s: %s bytes\n' "$name" "$(wc -c < "target/wasm/${name}.wasm" | tr -d ' ')"
done
