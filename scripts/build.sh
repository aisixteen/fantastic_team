#!/bin/sh
# sh scripts/build.sh [cargo args] — the general build of Fantastic Team (Team): stock exec.
# Reproducible (#137): the toolchain comes from rust-toolchain.toml, --locked, and the
# environment from scripts/repro-env.sh. Prints the versioned copy's path last
# (`built: <path>`) and its SHA-256. Extra arguments go to cargo build.
# The GitHub build is scripts/build-github.sh (patches/github-exec.patch).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
REPRO_SRC=$ROOT
REPRO_REPO=$ROOT
. "$ROOT/scripts/repro-env.sh"
cargo build --release --locked -p fantastic-team "$@"
DIR=${CARGO_TARGET_DIR:-$ROOT/target}
case "$(uname -s 2>/dev/null)" in MINGW* | MSYS* | CYGWIN*) EXE=.exe ;; *) EXE= ;; esac
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
TARGET=$(rustc -vV | sed -n 's/^host: //p')
OUT="$DIR/release/fantastic_team-$VER-$TARGET$EXE"
cp "$DIR/release/fantastic_team$EXE" "$OUT"
echo "sha256: $(repro_sha256 "$OUT")  $(basename "$OUT")"
echo "built: $OUT"
