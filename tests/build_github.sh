#!/bin/sh
# sh tests/build_github.sh — tests for scripts/build-github.sh (no cargo build runs).
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
B="$ROOT/scripts/build-github.sh"
T=$(mktemp -d "${TMPDIR:-/tmp}/build_github.XXXXXX")
trap 'rm -rf "$T"' EXIT
PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); echo "ok   $1"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL $1${2:+ — $2}"; }

# The CI github leg runs this inside the patched copy: the patch is already applied there.
if grep -q 'WORKER_ALLOWED: &str = r#"(?!' "$ROOT/crates/team/src/worker_exec.rs"; then
  ok "general_source_has_no_forge_refusal (skipped: the tree is the patched copy)"
  ok "the_real_patch_applies (skipped: the tree is the patched copy)"
  ok "github_build_prints_patch_hash (skipped: the tree is the patched copy)"
else
# 1. general_source_has_no_forge_refusal
if ! grep -qE '\(\?!|gh\|glab\|hub' "$ROOT/crates/team/src/worker_exec.rs"; then ok general_source_has_no_forge_refusal
else bad general_source_has_no_forge_refusal; fi

# 2. the_real_patch_applies
if [ -f "$ROOT/patches/github-exec.patch" ] && sh "$B" --check >"$T/out" 2>&1 \
  && grep -q 'patches/github-exec.patch' "$T/out"; then ok the_real_patch_applies
else bad the_real_patch_applies "$(cat "$T/out" 2>/dev/null)"; fi
# 2b. github_build_prints_patch_hash
if command -v sha256sum >/dev/null 2>&1; then want_hash=$(sha256sum "$ROOT/patches/github-exec.patch" | awk '{print $1}')
else want_hash=$(shasum -a 256 "$ROOT/patches/github-exec.patch" | awk '{print $1}'); fi
if sh "$B" --check >"$T/hash.out" 2>&1 && grep -qF "patch sha256: $want_hash" "$T/hash.out"; then ok github_build_prints_patch_hash
else bad github_build_prints_patch_hash "$(cat "$T/hash.out" 2>/dev/null)"; fi
fi

# 3. a_bad_patch_fails_and_leaves_the_tree_alone
cat >"$T/bad.patch" <<'PATCH'
diff --git a/crates/team/src/worker_exec.rs b/crates/team/src/worker_exec.rs
--- a/crates/team/src/worker_exec.rs
+++ b/crates/team/src/worker_exec.rs
@@ -1,3 +1,3 @@
-this line is not in the file
+neither is this one
 nor this
 nor that
PATCH
before=$(cd "$ROOT" && git status --porcelain)
GITHUB_EXEC_PATCH="$T/bad.patch" sh "$B" --check >"$T/bad.out" 2>&1
code=$?
after=$(cd "$ROOT" && git status --porcelain)
if [ "$code" -ne 0 ] && grep -q 'does not apply' "$T/bad.out"; then ok a_bad_patch_fails_with_a_message
else bad a_bad_patch_fails_with_a_message "exit $code: $(cat "$T/bad.out")"; fi
if [ "$before" = "$after" ]; then ok the_source_tree_is_unchanged
else bad the_source_tree_is_unchanged; fi

echo "build_github: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
