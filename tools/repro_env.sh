# Sourced by tools/build_modules.sh and tools/build_release.sh, from the checkout's root:
# the environment that makes a build the same bytes in any checkout, for any user and at any
# time (SPEC-9.7.1, docs/RELEASING.md). Sets:
#
#   root     the checkout as rustc sees it (a Windows path under Git Bash)
#   exe      the host's executable suffix
#   CARGO_ENCODED_RUSTFLAGS
#            the RUSTFLAGS (or CARGO_ENCODED_RUSTFLAGS) already set, then --remap-path-prefix
#            for CARGO_HOME (-> /cargo: registry sources), CARGO_TARGET_DIR when set
#            (-> /target: build-script output) and the checkout (-> /vy), so that no build path
#            is embedded in a panic location or anywhere else rustc writes. The encoded form
#            keeps paths with spaces whole; RUSTFLAGS is unset, since cargo prefers the other.
#   CFLAGS   -ffile-prefix-map for the same paths, for the C in ring and zstd-sys (__FILE__),
#            except under MSVC, whose cl has no such flag (and the release has no C asserts
#            there to embed a path; see docs/RELEASING.md)
#   SOURCE_DATE_EPOCH
#            the commit's time unless already set: the timestamp of every zip entry
#   MSYS2_ENV_CONV_EXCL
#            Git Bash on Windows rewrites what looks like a POSIX path in the environment
#            of a native program: `=/vy`, `=/cargo`, `=/target` and the `/Brepro` that
#            tools/build_release.sh adds would become paths under Git's own directory, the
#            remapping would name the builder's machine and the link would fail. The flag
#            variables are listed here, after any already listed, and reach cargo as written.
#            The other values set here have nothing to rewrite: SOURCE_DATE_EPOCH is a
#            number, and CFLAGS is not set on Windows.
#
# Cargo's own hashes do not see these flags (RUSTFLAGS are not part of -C metadata).

# A path as cargo hands it to rustc: Windows form under Git Bash, else as given (cargo does
# not resolve symlinks in CARGO_HOME or CARGO_TARGET_DIR; the checkout is the physical cwd).
native() {
  if command -v cygpath > /dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf '%s' "$1"
  fi
}

root=$(pwd -W 2> /dev/null || pwd -P)
exe=""
case "$(uname -s)" in MINGW* | MSYS* | CYGWIN*) exe=.exe ;; esac

repro_maps=("$(native "${CARGO_HOME:-$HOME/.cargo}")=/cargo")
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  repro_maps+=("$(native "$CARGO_TARGET_DIR")=/target")
fi
# Last: rustc applies the last mapping that matches.
repro_maps+=("$root=/vy")

repro_sep=$'\x1f'
if [ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]; then
  repro_flags=$CARGO_ENCODED_RUSTFLAGS$repro_sep
else
  repro_flags=""
  read -r -a repro_words <<< "${RUSTFLAGS:-}" || true
  for w in ${repro_words[@]+"${repro_words[@]}"}; do
    repro_flags+=$w$repro_sep
  done
fi
repro_cflags=""
for m in "${repro_maps[@]}"; do
  repro_flags+="--remap-path-prefix=$m$repro_sep"
  repro_cflags+=" -ffile-prefix-map=$m"
done
export CARGO_ENCODED_RUSTFLAGS=${repro_flags%"$repro_sep"}
unset RUSTFLAGS
export MSYS2_ENV_CONV_EXCL="${MSYS2_ENV_CONV_EXCL:+$MSYS2_ENV_CONV_EXCL;}CARGO_ENCODED_RUSTFLAGS;CFLAGS"
if [ -z "$exe" ]; then
  export CFLAGS="${CFLAGS:+$CFLAGS }${repro_cflags# }"
fi

if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  SOURCE_DATE_EPOCH=$(git log -1 --format=%ct 2> /dev/null || echo 315532800)
fi
export SOURCE_DATE_EPOCH
