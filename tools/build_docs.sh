#!/usr/bin/env bash
# Build the SDK documentation for a release into one folder:
#
#   <out>/index.html   a start page linking the three below
#   <out>/api/         rustdoc for vy-sdk (cargo doc -p vy-sdk --no-deps); a rustdoc
#                      warning fails the build
#   <out>/SDK.md       the guide: the mental model, the exports, capabilities, determinism,
#                      the budget, accessibility, a whole page, common mistakes (docs/SDK.md)
#   <out>/README.md    the SDK's README with one section per component
#
#   tools/build_docs.sh [out-dir] [version]     defaults: target/sdk-docs, `git describe`
#
# Nothing is fetched and nothing outside <out> and cargo's target directory is written.
set -euo pipefail
cd "$(dirname "$0")/.."

out=${1:-target/sdk-docs}
version=${2:-$(git describe --tags --always 2>/dev/null || echo dev)}
target=${CARGO_TARGET_DIR:-target}

# A fresh doc folder, so no other crate documented earlier ends up in the index.
rm -rf "$target/doc"
RUSTDOCFLAGS="${RUSTDOCFLAGS:-} -D warnings" cargo doc -p vy-sdk --no-deps --locked --quiet

rm -rf "$out"
mkdir -p "$out"
cp -R "$target/doc" "$out/api"
cp docs/SDK.md "$out/SDK.md"
cp crates/vy-sdk/README.md "$out/README.md"

cat > "$out/index.html" <<EOF
<!DOCTYPE html>
<html lang="sv">
<head><meta charset="utf-8"><title>vy-sdk ${version}</title></head>
<body>
<h1>vy-sdk ${version}</h1>
<ul>
<li><a href="api/vy_sdk/index.html">API-referens</a> (rustdoc; komponenterna under <code>vy_sdk::ui</code>)</li>
<li><a href="SDK.md">Guiden</a>: tankemodellen, exporterna, capabilities, determinism, budgeten,
tillgänglighet, en hel sida och vanliga misstag</li>
<li><a href="README.md">SDK:ts README</a>: ett avsnitt per komponent</li>
</ul>
</body>
</html>
EOF

printf 'SDK docs written to %s\n' "$out"
