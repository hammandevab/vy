#!/usr/bin/env bash
# Package the conformance suite for clients other than this one (SPEC-12.1.2: a client that
# passes the suite is a Vy client) as <out-dir>/vy-conformance-<version>.zip:
#
#   vy-conformance-<version>/
#     README.md                  how to run the suite and what a client must produce
#     SPEC.md, LICENSE           the spec the suite checks, and its licence
#     conformance/cases.toml     the cases: module, size, scale, grants, resources, events
#     conformance/golden/        the expected image of every case (PNG)
#     conformance/semantic/      the expected semantic tree of the cases that name one (JSON)
#     conformance/manifests/     the manifest draft of every reference module
#     conformance/README.md      the formats, and "Running the suite against another client"
#     target/wasm/<module>.wasm  the reference modules, built from modules/ here
#     modules/<module>/…         the image resources the cases and drafts name
#     SHA256SUMS                 every file above
#
#   tools/package_conformance.sh <version> [out-dir]      out-dir defaults to target
#
# The layout is the repository's, so `vy-cli conformance --root vy-conformance-<version>` runs the
# suite as is (vy-cli comes with the release's vy-<version>-<target>.zip). The zip is written by
# tools/release_files.py, like every other zip of a release: entries sorted, all with the last
# commit's time (SOURCE_DATE_EPOCH if set) and mode 0644, no directory entries and no extra
# fields. The same tree gives the same zip whatever the builder's umask, clock and time zone
# (SPEC-9.7.1), with the same Python (zlib).
set -euo pipefail
cd "$(dirname "$0")/.."

[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: tools/package_conformance.sh <version> [out-dir]" >&2; exit 2; }
version=${1#v}
out=${2:-target}
mkdir -p "$out"
out=$(cd "$out" && pwd)
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || echo 0)}
py=python3
command -v python3 > /dev/null 2>&1 || py=python
# GNU coreutils, or the shasum of macOS: the same "<hash>  <path>" lines.
if command -v sha256sum > /dev/null 2>&1; then sum="sha256sum"; else sum="shasum -a 256"; fi

tools/build_modules.sh > /dev/null

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
name="vy-conformance-$version"
stage="$tmp/$name"
mkdir -p "$stage/conformance" "$stage/target/wasm"

cp SPEC.md LICENSE "$stage/"
cp conformance/cases.toml conformance/README.md "$stage/conformance/"
for dir in golden semantic manifests; do
  mkdir -p "$stage/conformance/$dir"
  find "conformance/$dir" -maxdepth 1 -type f \( -name '*.png' -o -name '*.json' \) \
    -exec cp {} "$stage/conformance/$dir/" \;
done

modules=0
for dir in modules/*/; do
  m=$(basename "$dir")
  cp "target/wasm/$m.wasm" "$stage/target/wasm/"
  modules=$((modules + 1))
done

# Resources: every "modules/…" path cases.toml names, and every path a draft names relative to
# itself ("../../modules/…"), copied to the same place under the root.
{
  grep -o '"modules/[^"]*"' conformance/cases.toml | tr -d '"' || true
  cat conformance/manifests/*.json | grep -o '"path" *: *"[^"]*"' \
    | sed 's/.*"\.\.\/\.\.\/\([^"]*\)"/\1/' | grep '^modules/' || true
} | LC_ALL=C sort -u | while read -r res; do
  [ -f "$res" ] || { echo "error: $res is named but missing" >&2; exit 1; }
  mkdir -p "$stage/$(dirname "$res")"
  cp "$res" "$stage/$res"
done

cases=$(grep -c '^\[\[case\]\]' conformance/cases.toml)
cat > "$stage/README.md" <<EOF
# Vy conformance suite $version

$cases cases over $modules reference modules. A client that passes the suite is a Vy client
(SPEC-12.1.2). What a client must produce for each case, and how pass and fail are decided, is in
conformance/README.md, section "Running the suite against another client".

Run it with this project's own runner (vy-cli is in the release's vy-$version-<target>.zip):

    vy-cli conformance --root $name

Every case prints "ok" or "FAIL" with the share of differing pixels and, for cases with a tree,
whether the semantic tree matched; the exit code is 0 only when every case passed. Shell cases
(shell = true) run the page in a tab process that is vy-cli itself (vy-cli --tab) under this
client's headless shell. SHA256SUMS lists every file of the suite.
EOF

(cd "$stage" && find . -type f ! -name SHA256SUMS | LC_ALL=C sort | sed 's|^\./||' \
  | xargs $sum > SHA256SUMS)

# The one zip writer of a release: it takes neither modes nor times from the files.
rm -f "$out/$name.zip"
SOURCE_DATE_EPOCH=$epoch "$py" tools/release_files.py zip "$tmp" "$out/$name.zip"
printf '%s: %s cases, %s modules, %s bytes\n' "$out/$name.zip" "$cases" "$modules" \
  "$(wc -c < "$out/$name.zip" | tr -d ' ')"
