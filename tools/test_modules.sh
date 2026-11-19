#!/usr/bin/env bash
# Run every module's own gates (SPEC-12.2.1): rustfmt without a diff, its native
# tests against the SDK mock, and clippy for wasm32-unknown-unknown without warnings.
# Modules are excluded from the workspace, so `cargo test --workspace` never sees them;
# CI runs this on every platform next to tools/build_modules.sh.
#
# Last, a size ceiling for a page that has one-line text fields only (granskning 0014:7):
# target/wasm/form.wasm, as tools/build_modules.sh built it, is at most 33 300 bytes.
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
# ui::TextBox keeps its editing code in one non-generic `Field` and hands it the line mode as an
# argument, which is the same constant at every call of a page with one-line fields only; the
# compiler folds it and drops the multi-line half (wrapping, the vertical keys, the wheel, the
# scroll bar). Nothing else says that it does, so the size of such a page is the check.
form=target/wasm/form.wasm
form_max=33300
if [ ! -f "$form" ]; then
  printf 'error: %s is missing; run tools/build_modules.sh first\n' "$form" >&2
  status=1
else
  form_size=$(wc -c < "$form" | tr -d ' ')
  if [ "$form_size" -gt "$form_max" ]; then
    cat >&2 <<EOF
form.wasm: $form_size bytes, over its ceiling of $form_max: FAILED
  form has one-line text fields only, so the multi-line half of ui::TextBox (wrap_with,
  fit_prefix, the vertical keys, the wheel, the scroll bar) must not be in the module.
  About 6 500 bytes over what it was (32 636) means the compiler no longer folds the line
  mode away: look for a change that made it a field, a function pointer or a value read at
  run time in crates/vy-sdk/src/ui/textbox.rs, or for changed build settings (opt-level,
  lto, codegen-units, the toolchain); docs/notes/m5-polish.md, punkt 4, has the background.
  A smaller growth means the page or the SDK really grew: raise form_max in
  tools/test_modules.sh in the same commit and say why in its message.
EOF
    status=1
  else
    printf 'form.wasm: %s bytes (ceiling %s): ok\n' "$form_size" "$form_max"
  fi
fi
exit "$status"
