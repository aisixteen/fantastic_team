#!/bin/sh
# sh scripts/build-github.sh [--check|--test] [cargo args] — the GitHub build (Team GitHub).
# Copies the tracked tree to a temporary folder, applies patches/github-exec.patch there
# and builds the copy. The source tree is never touched.
#   (no flag)  cargo build --release -p fantastic-team in the copy; the binary is copied to
#              $CARGO_TARGET_DIR/release/fantastic_team-github (default target/github).
#              --test builds into $CARGO_TARGET_DIR or target/github-test.
#   --check    copy and apply the patch only; no build.
#   --test     cargo test -p fantastic-team in the copy (the patched tests run).
# GITHUB_EXEC_PATCH names another patch file (the failure test feeds a bad one).
# Exit 1 when the patch does not apply.
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PATCH=${GITHUB_EXEC_PATCH:-$ROOT/patches/github-exec.patch}
MODE=build
case "${1:-}" in
  --check) MODE=check; shift ;;
  --test) MODE=test; shift ;;
esac
TMP=$(mktemp -d "${TMPDIR:-/tmp}/team_github.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
COPY="$TMP/tree"
mkdir -p "$COPY"
(
  cd "$ROOT"
  git ls-files --cached --others --exclude-standard | while IFS= read -r f; do
    [ -e "$f" ] && printf '%s\n' "$f"
  done | tar -cf - -T -
) | tar -xf - -C "$COPY"
if ! (cd "$COPY" && git init -q . && git apply --check "$PATCH") 2>"$TMP/err"; then
  cat "$TMP/err" >&2
  echo "build-github: patches/github-exec.patch does not apply" >&2
  exit 1
fi
(cd "$COPY" && git apply "$PATCH")
echo "applied patch: $PATCH"
cd "$COPY"
case "$MODE" in
  check) exit 0 ;;
  test)
    CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target/github-test} cargo test -p fantastic-team "$@"
    exit $?
    ;;
esac
OUT=${CARGO_TARGET_DIR:-$ROOT/target/github}
case "$(uname -s 2>/dev/null)" in MINGW* | MSYS* | CYGWIN*) EXE=.exe ;; *) EXE= ;; esac
CARGO_TARGET_DIR="$OUT" cargo build --release -p fantastic-team "$@"
cp "$OUT/release/fantastic_team$EXE" "$OUT/release/fantastic_team-github$EXE"
echo "built: $OUT/release/fantastic_team-github$EXE"
