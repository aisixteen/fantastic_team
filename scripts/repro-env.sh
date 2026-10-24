# sh-sourced, not run: `. scripts/repro-env.sh` (#137). The environment that makes a build
# reproducible: a fixed time, a fixed locale and zone, and the source, cargo and rustup paths
# remapped out of the binary. Sourced by scripts/build.sh and scripts/build-github.sh.
#   REPRO_SRC   the tree being built; its path is remapped to /src (default: the current dir)
#   REPRO_REPO  the git work tree that gives the commit time (default: REPRO_SRC)
#   SOURCE_DATE_EPOCH  set it to override the commit time (default: last commit's %ct, else 0)

: "${REPRO_SRC:=$(pwd)}"
: "${REPRO_REPO:=$REPRO_SRC}"
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  SOURCE_DATE_EPOCH=$(git -C "$REPRO_REPO" log -1 --format=%ct 2>/dev/null || true)
  [ -n "$SOURCE_DATE_EPOCH" ] || SOURCE_DATE_EPOCH=0
fi
export SOURCE_DATE_EPOCH
export ZERO_AR_DATE=1 TZ=UTC LC_ALL=C

# Remap the paths that would otherwise land in the binary (panic locations, debug info).
# Cargo's home and rustup's home come from the environment when set. The target dir is
# remapped too (REPRO_TARGET, else CARGO_TARGET_DIR): build-script output under it can land in
# the binary, and each side of the repro job has its own target dir. It is listed first.
REPRO_CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
REPRO_RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup}
REPRO_TARGET=${REPRO_TARGET:-${CARGO_TARGET_DIR:-}}
REPRO_TARGET_REMAP=
if [ -n "$REPRO_TARGET" ]; then REPRO_TARGET_REMAP="--remap-path-prefix=$REPRO_TARGET=/target "; fi
RUSTFLAGS="${RUSTFLAGS:-} ${REPRO_TARGET_REMAP}--remap-path-prefix=$REPRO_SRC=/src --remap-path-prefix=$REPRO_CARGO_HOME=/cargo --remap-path-prefix=$REPRO_RUSTUP_HOME=/rustup"
export RUSTFLAGS

# Hash helpers: sha256sum on Linux and Git-for-Windows, shasum on macOS.
repro_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'
}
