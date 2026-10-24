#!/bin/sh
# sh tests/repro_scripts.sh — tests for the reproducible-build scripts (#137):
# scripts/repro-env.sh, scripts/checksums.sh and scripts/verify.sh. No cargo build runs.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/repro_scripts.XXXXXX")
trap 'rm -rf "$T"' EXIT
PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); echo "ok   $1"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL $1${2:+ — $2}"; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'
}

# 1. build_scripts_are_locked_and_remapped
# The cargo command lines (not the # comments) must each carry --locked. A header comment
# that mentions --locked does not count. $2 is the least number of cargo build/test lines.
cargo_lines_locked() {
  n=$(grep -v '^[[:space:]]*#' "$1" | grep -c 'cargo \(build\|test\)')
  m=$(grep -v '^[[:space:]]*#' "$1" | grep 'cargo \(build\|test\)' | grep -c -- '--locked')
  [ "$n" -ge "$2" ] && [ "$n" = "$m" ]
}
ok_locked=1
cargo_lines_locked "$ROOT/scripts/build.sh" 1 || ok_locked=0
cargo_lines_locked "$ROOT/scripts/build-github.sh" 2 || ok_locked=0
for s in build.sh build-github.sh; do
  grep -q 'repro-env.sh' "$ROOT/scripts/$s" || ok_locked=0
done
env_out=$(cd "$ROOT" && sh -c '. scripts/repro-env.sh; echo $SOURCE_DATE_EPOCH $RUSTFLAGS' 2>"$T/env.err")
epoch=${env_out%% *}
case "$epoch" in
  '' | *[!0-9]*) epoch_ok=0 ;;
  *) epoch_ok=1 ;;
esac
# With CARGO_TARGET_DIR set, the target dir is remapped too (a different target dir per side
# must not change the binary).
tgt_out=$(cd "$ROOT" && CARGO_TARGET_DIR=/x/tgt sh -c '. scripts/repro-env.sh; echo $RUSTFLAGS' 2>"$T/tgt.err")
tgt_ok=1
printf '%s\n' "$tgt_out" | grep -qF -- "--remap-path-prefix=/x/tgt=/target" || tgt_ok=0
if [ "$ok_locked" = 1 ] && [ "$epoch_ok" = 1 ] && [ "$tgt_ok" = 1 ] \
  && printf '%s\n' "$env_out" | grep -qF -- "--remap-path-prefix=$ROOT=/src"; then
  ok build_scripts_are_locked_and_remapped
else
  bad build_scripts_are_locked_and_remapped "locked=$ok_locked epoch='$epoch' target_remap=$tgt_ok env='$env_out' tgt='$tgt_out' $(cat "$T/env.err" "$T/tgt.err" 2>/dev/null)"
fi

# 2. checksums_lists_sha256_and_md5_sorted
mkdir -p "$T/art"
printf 'beta' >"$T/art/b.bin"
printf 'alpha' >"$T/art/a.bin"
if (cd "$ROOT" && sh scripts/checksums.sh "$T/art") >"$T/cs.out" 2>&1; then
  cat >"$T/want.sha256" <<'WANT'
8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8  a.bin
f44e64e75f3948e9f73f8dfa94721c4ce8cbb4f265c4790c702b2d41cfbf2753  b.bin
WANT
  cat >"$T/want.md5" <<'WANT'
2c1743a391305fbf367df8e4f069f9f9  a.bin
987bcab01b929eb2c07877b224215c92  b.bin
WANT
  want_patch="patch sha256: $(sha256_of "$ROOT/patches/github-exec.patch")"
  sums_ok=0
  md5_ok=0
  patch_ok=0
  cmp -s "$T/want.sha256" "$T/art/SHA256SUMS" && sums_ok=1
  cmp -s "$T/want.md5" "$T/art/MD5SUMS" && md5_ok=1
  grep -qxF "$want_patch" "$T/art/RELEASE-INFO.txt" && patch_ok=1
  if [ "$sums_ok" = 1 ] && [ "$md5_ok" = 1 ] && [ "$patch_ok" = 1 ]; then
    ok checksums_lists_sha256_and_md5_sorted
  else
    bad checksums_lists_sha256_and_md5_sorted "sums=$sums_ok md5=$md5_ok patch=$patch_ok: $(cat "$T/art/SHA256SUMS" 2>/dev/null)"
  fi
else
  bad checksums_lists_sha256_and_md5_sorted "checksums.sh failed: $(cat "$T/cs.out")"
fi

# 3. verify_accepts_matching_binary_and_rejects_one_changed_byte
mkdir -p "$T/rel"
printf 'fake-binary-bytes' >"$T/rel/fantastic_team-0.1.0-x86_64-unknown-linux-gnu"
(cd "$ROOT" && sh scripts/checksums.sh "$T/rel") >/dev/null 2>&1
BIN="$T/rel/fantastic_team-0.1.0-x86_64-unknown-linux-gnu"
sh "$ROOT/scripts/verify.sh" "$BIN" >"$T/v1.out" 2>&1
v1=$?
printf 'F' | dd of="$BIN" bs=1 seek=0 conv=notrunc 2>/dev/null
sh "$ROOT/scripts/verify.sh" "$BIN" >"$T/v2.out" 2>&1
v2=$?
printf 'fake-binary-bytes' >"$T/rel/other-name"
sh "$ROOT/scripts/verify.sh" "$T/rel/other-name" >"$T/v3.out" 2>&1
v3=$?
if [ "$v1" -eq 0 ] && [ "$v2" -eq 1 ] && grep -qi 'mismatch' "$T/v2.out" \
  && [ "$v3" -eq 1 ]; then
  ok verify_accepts_matching_binary_and_rejects_one_changed_byte
else
  bad verify_accepts_matching_binary_and_rejects_one_changed_byte "exit $v1/$v2/$v3: $(cat "$T/v1.out" "$T/v2.out" "$T/v3.out")"
fi

echo "repro_scripts: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
