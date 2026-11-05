#!/usr/bin/env bash
# Publish the reference site: build every module, sign the drafts in
# conformance/manifests/ into <out-dir>/<name>.vyp, render a preview of each
# page into <out-dir>/previews/<name>.png and write the publisher key file to
# <out-dir>/dev.key.pub. The Dockerfile runs this script; so can you:
#
#   tools/publish_site.sh <out-dir> [key]
#
# <key> is the file `vy-publish keygen --out <key>` writes (a 32-byte seed) or
# that seed as 64 hex characters, which is what a build-system variable can
# hold. Without a key a temporary one is generated and discarded, so every run
# signs with a new key; a client that pinned the previous one sees the pages as
# untrusted until it forgets the pin (SPEC-7.4.2, 7.4.3). Keys are never
# written into <out-dir>; `*.key` is git-ignored.
#
# Previews are the first frame at 1280x800, scale 1, with the draft's resources
# in order (SPEC-4.5.1) and the capabilities it requests; `vy-cli render` is
# deterministic, so the PNG is the same on every machine (SPEC-11.3.1).
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
  echo "usage: tools/publish_site.sh <out-dir> [key]" >&2
  exit 2
}
[ $# -ge 1 ] && [ $# -le 2 ] || usage
out=$1
key=${2:-}

# Resource lines "path<TAB>w<TAB>h" of a draft in order; w and h are empty for
# a resource that is not an image. A draft is JSON where every resource is one
# `{...}` object without nested braces (vy_manifest::DraftResource is flat), so
# the objects can be cut out without a JSON parser; the count is checked
# against what vy-publish built.
draft_resources() {
  tr -d '\n\r' < "$1" | { grep -o '{[^{}]*"path"[^{}]*}' || true; } | while read -r obj; do
    path=$(printf '%s' "$obj" | sed -n 's/.*"path" *: *"\([^"]*\)".*/\1/p')
    w=$(printf '%s' "$obj" | sed -n 's/.*"w" *: *\([0-9][0-9]*\).*/\1/p')
    h=$(printf '%s' "$obj" | sed -n 's/.*"h" *: *\([0-9][0-9]*\).*/\1/p')
    printf '%s\t%s\t%s\n' "$path" "$w" "$h"
  done
}

# The capability names a draft requests, comma-separated, in draft order:
# the keys of the `capabilities` map, whose values are objects at most one
# level deep (SPEC-8.2.1).
draft_capabilities() {
  tr -d '\n\r' < "$1" \
    | sed -n 's/.*"capabilities" *: *\({\([^{}]*{[^{}]*}\)*[^{}]*}\).*/\1/p' \
    | { grep -o '"[a-z_]*" *: *{' || true; } \
    | sed 's/^"\([a-z_]*\)".*/\1/' \
    | paste -sd, -
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# The key is checked before the builds so a wrong one fails in a second.
if [ -n "$key" ] && [ ! -f "$key" ]; then
  echo "error: $key: no such file" >&2
  exit 1
fi
if [ -n "$key" ] && [ "$(wc -c < "$key" | tr -d ' ')" -ne 32 ]; then
  hex=$(tr -d ' \n\r\t' < "$key")
  case "$hex" in
    *[!0-9a-fA-F]* | '')
      echo "error: $key is neither a 32-byte seed nor 64 hex characters" >&2
      exit 1
      ;;
  esac
  if [ "${#hex}" -ne 64 ]; then
    echo "error: $key: expected 64 hex characters, found ${#hex}" >&2
    exit 1
  fi
  (umask 077 && : > "$tmp/dev.key")
  for ((i = 0; i < 64; i += 2)); do printf "\\x${hex:i:2}"; done > "$tmp/dev.key"
  key=$tmp/dev.key
fi

cargo build --release -p vy-publish -p vy-cli
bin=target/release
tools/build_modules.sh

if [ -z "$key" ]; then
  "$bin/vy-publish" keygen --out "$tmp/dev.key" > /dev/null
  key=$tmp/dev.key
  echo "no key given: signing with a temporary key (vy-publish keygen --out dev.key for a stable one)"
fi

mkdir -p "$out/previews"
"$bin/vy-publish" pubkey --key "$key" --out "$out/dev.key.pub"

count=0
for draft in conformance/manifests/*.json; do
  name=$(basename "$draft" .json)
  module=target/wasm/$name.wasm
  report=$("$bin/vy-publish" build --manifest "$draft" --module "$module" --key "$key" --out "$out/$name.vyp")
  printf '%s\n' "$report"
  built=$(printf '%s\n' "$report" | sed -n 's/.*, \([0-9][0-9]*\) resources,.*/\1/p')

  args=()
  found=0
  while IFS=$'\t' read -r path w h; do
    args+=(--resource "$(dirname "$draft")/$path")
    if [ -n "$w" ]; then
      args+=(--resource-size "${w}x${h}")
    fi
    found=$((found + 1))
  done < <(draft_resources "$draft")
  if [ -z "$built" ] || [ "$found" -ne "$built" ]; then
    echo "error: $draft: read $found resources but vy-publish built ${built:-?}" >&2
    exit 1
  fi
  caps=$(draft_capabilities "$draft")
  if [ -n "$caps" ]; then
    args+=(--grant "$caps")
  fi
  # Module and host log lines go to stderr; shown only when the render fails.
  if ! "$bin/vy-cli" render "$module" --out "$out/previews/$name.png" --size 1280x800 --scale 1 \
      ${args[@]+"${args[@]}"} 2> "$tmp/render.log"; then
    cat "$tmp/render.log" >&2
    exit 1
  fi
  count=$((count + 1))
done

echo
echo "$count packages, $count previews and dev.key.pub in $out. Serve them with:"
echo "  cargo run --release -p vy-serve -- --dir $out --public-key $out/dev.key.pub --port 8080"
echo "then open http://localhost:8080/ in a browser or vy://localhost:8080/hello in Vy."
