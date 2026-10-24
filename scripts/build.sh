#!/bin/sh
# sh scripts/build.sh [cargo args] — the general build of Fantastic Team (Team): stock exec.
# Prints the path of the binary. Extra arguments go to cargo build.
# The GitHub build is scripts/build-github.sh (patches/github-exec.patch).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
cargo build --release -p fantastic-team "$@"
DIR=${CARGO_TARGET_DIR:-$ROOT/target}
case "$(uname -s 2>/dev/null)" in MINGW* | MSYS* | CYGWIN*) EXE=.exe ;; *) EXE= ;; esac
echo "built: $DIR/release/fantastic_team$EXE"
