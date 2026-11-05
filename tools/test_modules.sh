#!/usr/bin/env bash
# Run every module's own gates (SPEC-12.2.1): rustfmt without a diff, its native
# tests against the SDK mock, and clippy for wasm32-unknown-unknown without warnings.
# Modules are excluded from the workspace, so `cargo test --workspace` never sees them;
# CI runs this on every platform next to tools/build_modules.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
status=0
for dir in modules/*/; do
  name=$(basename "$dir")
  if (cd "$dir" \
      && cargo fmt --check --quiet \
      && cargo test --quiet 2>&1 | grep -E '^test result' \
      && cargo clippy --quiet --target wasm32-unknown-unknown -- -D warnings); then
    printf '%s: ok\n' "$name"
  else
    printf '%s: FAILED\n' "$name" >&2
    status=1
  fi
done
exit "$status"
