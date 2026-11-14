#!/usr/bin/env bash
# The release of one target, as .github/workflows/release.yml builds it and as anyone can
# build it again from the same commit (SPEC-9.7.1, docs/RELEASING.md):
#
#   tools/build_release.sh <target> <stage-dir>
#
# builds vy-cli, vy-dev, vy-publish and vy-serve in one cargo call and vy-shell in a second
# (features unify within a call, and vy-cli's vy-shell has no window), --release --locked
# --target <target>, and puts the five binaries, studio.vyp (tools/build_studio.sh) and
# LICENSE in <stage-dir>, which must be empty or not exist. tools/repro_env.sh makes the
# bytes independent of the checkout's path, the user and the time; two builds of the same
# commit on the same class of machine give the same binaries. studio.vyp is signed with a
# throwaway key, so it is the same package identity (`vy-publish inspect`), not the same
# bytes.
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: tools/build_release.sh <target> <stage-dir>" >&2; exit 2; }
target=$1
mkdir -p "$2"
stage=$(cd "$2" && pwd)
cd "$(dirname "$0")/.."
. tools/repro_env.sh
if [ -n "$(ls -A "$stage")" ]; then
  echo "error: $stage is not empty" >&2
  exit 1
fi

ext=""
case "$target" in *windows*) ext=.exe ;; esac
# MSVC's linker stamps the PE header with the time; /Brepro puts a hash of the
# image there instead. Only the two native builds get the flag: exported, it would
# reach the wasm link of studio below, and rust-lld takes /Brepro for a file.
native_flags=$CARGO_ENCODED_RUSTFLAGS
case "$target" in
  *-windows-msvc) native_flags="$native_flags$repro_sep-Clink-arg=/Brepro" ;;
esac
CARGO_ENCODED_RUSTFLAGS=$native_flags \
  cargo build --release --locked --target "$target" -p vy-cli -p vy-dev -p vy-publish -p vy-serve
CARGO_ENCODED_RUSTFLAGS=$native_flags \
  cargo build --release --locked --target "$target" -p vy-shell
out=${CARGO_TARGET_DIR:-target}/$target/release
for bin in vy-cli vy-dev vy-publish vy-serve vy-shell; do
  cp "$out/$bin$ext" "$stage/"
done
VY_PUBLISH="$out/vy-publish$ext" tools/build_studio.sh "$stage/studio.vyp" > /dev/null
cp LICENSE "$stage/"
ls -l "$stage"
