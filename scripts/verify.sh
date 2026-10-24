#!/bin/sh
# sh scripts/verify.sh [--sums FILE] [--rebuild] <binary> — checks a Fantastic Team binary (#137).
#   (a) the binary's SHA-256 against its line in SHA256SUMS (next to it, or --sums FILE)
#   --rebuild  also rebuilds from this checkout and compares the hash: scripts/build-github.sh
#              for a fantastic_team-github* name, else scripts/build.sh. Check out the tag first.
# Exit 0: the hash matches. Exit 1: mismatch, no entry for the name, or the rebuild differs.
# Exit 2: usage, an unreadable file, or a failed rebuild.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)

usage() {
  echo "usage: sh scripts/verify.sh [--sums FILE] [--rebuild] <binary>" >&2
  exit "$1"
}
sha() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'
}

SUMS=""
REBUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --sums) [ $# -ge 2 ] || usage 2; SUMS=$2; shift 2 ;;
    --rebuild) REBUILD=1; shift ;;
    -h | --help) usage 0 ;;
    --) shift; break ;;
    -*) usage 2 ;;
    *) break ;;
  esac
done
BIN=${1:-}
[ -n "$BIN" ] || usage 2
[ -f "$BIN" ] || { echo "verify: no such file: $BIN" >&2; exit 2; }
NAME=$(basename "$BIN")
[ -n "$SUMS" ] || SUMS="$(dirname "$BIN")/SHA256SUMS"
[ -f "$SUMS" ] || { echo "verify: no SHA256SUMS at $SUMS" >&2; exit 2; }

WANT=$(awk -v n="$NAME" '{ f = $2; sub(/^\*/, "", f); if (f == n) { print $1; exit } }' "$SUMS")
if [ -z "$WANT" ]; then
  echo "verify: no entry for $NAME in $SUMS" >&2
  exit 1
fi
GOT=$(sha "$BIN")
if [ "$GOT" != "$WANT" ]; then
  echo "verify: mismatch for $NAME: SHA256SUMS has $WANT, the file is $GOT" >&2
  exit 1
fi
echo "verify: ok $NAME $GOT"

if [ "$REBUILD" = 1 ]; then
  case "$NAME" in
    fantastic_team-github*) BUILD="$ROOT/scripts/build-github.sh" ;;
    *) BUILD="$ROOT/scripts/build.sh" ;;
  esac
  LOG=$(mktemp "${TMPDIR:-/tmp}/verify.XXXXXX") || exit 2
  if ! sh "$BUILD" >"$LOG" 2>&1; then
    cat "$LOG" >&2
    rm -f "$LOG"
    echo "verify: the rebuild failed" >&2
    exit 2
  fi
  REBUILT=$(sed -n 's/^built: //p' "$LOG" | tail -1)
  rm -f "$LOG"
  if [ -z "$REBUILT" ] || [ ! -f "$REBUILT" ]; then
    echo "verify: the rebuild printed no binary" >&2
    exit 2
  fi
  RE=$(sha "$REBUILT")
  if [ "$RE" != "$GOT" ]; then
    echo "verify: rebuild mismatch for $NAME: the file is $GOT, the rebuild is $RE" >&2
    exit 1
  fi
  echo "verify: rebuild matches $NAME"
fi
exit 0
