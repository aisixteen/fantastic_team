#!/bin/sh
# sh scripts/checksums.sh <dir> — checksums for the build artifacts in <dir> (#137).
# Writes, in <dir>, sorted by name (LC_ALL=C), for every regular file except these three:
#   SHA256SUMS        `<sha256>  <name>`, the checksum that verify.sh reads
#   MD5SUMS           `<md5>  <name>`, for accidental corruption only, not a security check
#   RELEASE-INFO.txt  commit, patch name and sha256, toolchain, the fantastic commit
# Nothing is signed here. Signing is laptop-only (see docs/install.md, Reproducible builds).
set -eu
ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIR=${1:?usage: sh scripts/checksums.sh <dir>}
LC_ALL=C
export LC_ALL

sha() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'
}
md5() {
  if command -v md5sum >/dev/null 2>&1; then md5sum "$1"
  elif command -v md5 >/dev/null 2>&1; then md5 -r "$1"
  else openssl dgst -md5 -r "$1"; fi | awk '{print $1}'
}

cd "$DIR"
# An artifact is a regular file that is not one of the three files this script writes.
is_artifact() {
  case "$1" in
    SHA256SUMS | MD5SUMS | RELEASE-INFO.txt) return 1 ;;
  esac
  [ -f "$1" ]
}
NAMES=$(for f in *; do is_artifact "$f" && printf '%s\n' "$f"; done | LC_ALL=C sort)

: >SHA256SUMS
: >MD5SUMS
printf '%s\n' "$NAMES" | while IFS= read -r f; do
  [ -n "$f" ] || continue
  printf '%s  %s\n' "$(sha "$f")" "$f" >>SHA256SUMS
  printf '%s  %s\n' "$(md5 "$f")" "$f" >>MD5SUMS
done

COMMIT=$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)
FANTASTIC=$(grep -o 'fantastic?branch=main#[0-9a-f]*' "$ROOT/Cargo.lock" 2>/dev/null | head -1 | sed 's/.*#//' || true)
PIN=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$ROOT/rust-toolchain.toml" 2>/dev/null | head -1 || true)
{
  echo "commit: ${COMMIT:-unknown}"
  echo "patch: patches/github-exec.patch"
  echo "patch sha256: $(sha "$ROOT/patches/github-exec.patch")"
  echo "toolchain pin: ${PIN:-unknown}"
  echo "rustc: $(rustc -V 2>/dev/null || echo unknown)"
  echo "cargo: $(cargo -V 2>/dev/null || echo unknown)"
  echo "fantastic: ${FANTASTIC:-unknown}"
  echo "checksums: SHA256SUMS (sha256), MD5SUMS (md5: accidental corruption only, not a security check)"
  echo "signatures: none in CI; signing happens on the maintainer's laptop"
} >RELEASE-INFO.txt
echo "checksums: wrote $DIR/SHA256SUMS, MD5SUMS, RELEASE-INFO.txt"
