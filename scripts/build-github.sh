#!/bin/sh
# sh scripts/build-github.sh [--check|--test] [cargo args] — the GitHub build (Team GitHub).
# Copies the tracked tree to a temporary folder, applies patches/github-exec.patch there
# and builds the copy. The source tree is never touched.
#   (no flag)  cargo build --release --locked -p fantastic-team in the copy; the binary is
#              copied to $CARGO_TARGET_DIR/release/fantastic_team-github (default target/github)
#              and to the versioned name fantastic_team-github-<ver>-<target> (#137).
#              --test builds into $CARGO_TARGET_DIR or target/github-test.
#   --check    copy and apply the patch only; no build. Prints the patch's sha256.
#   --test     cargo test -p fantastic-team in the copy (the patched tests run).
# GITHUB_EXEC_PATCH names another patch file (the failure test feeds a bad one).
# Reproducible (#137): the toolchain comes from rust-toolchain.toml (the copy has it too),
# and the environment from scripts/repro-env.sh with the copy's path remapped to /src.
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
if command -v sha256sum >/dev/null 2>&1; then PATCH_SHA=$(sha256sum "$PATCH" | awk '{print $1}')
else PATCH_SHA=$(shasum -a 256 "$PATCH" | awk '{print $1}'); fi
echo "patch sha256: $PATCH_SHA"
cd "$COPY"
case "$MODE" in
  check) exit 0 ;;
esac
# The copy is the source: its path is what gets remapped; the commit time comes from ROOT.
REPRO_SRC=$COPY
REPRO_REPO=$ROOT
. "$ROOT/scripts/repro-env.sh"
case "$MODE" in
  test)
    CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target/github-test} cargo test --locked -p fantastic-team "$@"
    exit $?
    ;;
esac
OUT=${CARGO_TARGET_DIR:-$ROOT/target/github}
case "$(uname -s 2>/dev/null)" in MINGW* | MSYS* | CYGWIN*) EXE=.exe ;; *) EXE= ;; esac
CARGO_TARGET_DIR="$OUT" cargo build --release --locked -p fantastic-team "$@"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' "$COPY/Cargo.toml" | head -1)
TARGET=$(rustc -vV | sed -n 's/^host: //p')
cp "$OUT/release/fantastic_team$EXE" "$OUT/release/fantastic_team-github$EXE"
VERSIONED="$OUT/release/fantastic_team-github-$VER-$TARGET$EXE"
cp "$OUT/release/fantastic_team$EXE" "$VERSIONED"
echo "sha256: $(repro_sha256 "$VERSIONED")  $(basename "$VERSIONED")"
echo "built: $VERSIONED"
