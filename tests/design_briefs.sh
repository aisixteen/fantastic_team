#!/bin/sh
# sh tests/design_briefs.sh — tests for scripts/design-sync.sh and templates/design-briefs.md.
# Runs in a mktemp dir; no network, no git, no model. Each test builds a staging dir of briefs.
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
G="$ROOT/scripts/design-sync.sh"
TPL="$ROOT/templates/design-briefs.md"
T=$(mktemp -d "${TMPDIR:-/tmp}/design_briefs.XXXXXX")
trap 'rm -rf "$T"' EXIT
PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); echo "ok   $1"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL $1${2:+ — $2}"; }
has() { grep -qF -e "$2" "$1"; }
S="$T/staging"
O="$T/design"
R="$T/out"

# brief NAME [EXTRA] — a complete brief with every required section.
brief() {
    printf '# %s\n\n## Purpose and audience\nShows one screen for the operator.\n\n## Data shown\n- `GET /api/tree`: rows sorted by name, e.g. `alpha`, `beta`.\n\n## States\n- loading, empty, error, many items, a stalled row.\n\n## Actions\n- open a row: goes to the group canvas.\n\n## Shared rules\n- light and dark, phone and desktop, same page in a browser and in the app.\n' "$1"
    [ -z "${2:-}" ] || printf '%s\n' "$2"
}
mkstage() { # mkstage NAME... — fresh staging dir with one brief per name
    rm -rf "$S"
    mkdir -p "$S"
    for n in "$@"; do brief "$n" >"$S/$n.md"; done
}
run() { # run [args] — the script at this commit and app, with a faked date
    DESIGN_SYNC_DATE=${DATE_FAKE:-2026-10-24} sh "$G" --app fantastic_team --commit abc1234 --staging "$S" --out "$O" "$@"
}
snap() { (cd "$O" && find . -type f | LC_ALL=C sort | xargs cksum); }

# 1. first_run_writes_everything
mkstage 00-overview 01-x 02-group-canvas
run >"$R" 2>&1
rc=$?
rows=$(grep -cE '^\| [0-9][0-9]-[a-z-]+ \| [0-9a-f]{12} \| new \| no \|$' "$O/manifest.md" 2>/dev/null || true)
if [ "$rc" = 0 ] && [ -f "$O/README.md" ] && [ -f "$O/manifest.md" ] \
    && [ -f "$O/00-overview.md" ] && [ -f "$O/01-x.md" ] && [ -f "$O/02-group-canvas.md" ] \
    && [ "$rows" = 3 ] && has "$O/manifest.md" 'app: fantastic_team' \
    && has "$O/manifest.md" 'source commit: abc1234' && has "$O/manifest.md" 'last run: 2026-10-24' \
    && [ ! -e "$O/html" ] && cmp -s "$S/01-x.md" "$O/01-x.md"; then
    ok first_run_writes_everything
else bad first_run_writes_everything "rc=$rc rows=$rows $(cat "$R")"; fi

# 2. second_run_changes_nothing
before=$(snap)
cp "$O/manifest.md" "$T/m1"
sleep 1
touch "$T/marker"
DATE_FAKE=2026-12-01 run >"$R" 2>&1
rc=$?
after=$(snap)
newer=$(find "$O" -type f -newer "$T/marker")
if [ "$rc" = 0 ] && [ "$(cat "$R")" = "no change" ] && [ "$before" = "$after" ] \
    && [ -z "$newer" ] && cmp -s "$T/m1" "$O/manifest.md"; then
    ok second_run_changes_nothing
else bad second_run_changes_nothing "rc=$rc out=$(cat "$R") newer=$newer"; fi

# 3. one_changed_screen_touches_only_that_brief
sleep 1
printf 'Changed: the group now shows a stalled count.\n' >>"$S/02-group-canvas.md"
touch "$T/marker"
run >"$R" 2>&1
rc=$?
newer=$(cd "$O" && find . -type f -newer "$T/marker" | LC_ALL=C sort | tr '\n' ' ')
if [ "$rc" = 0 ] && [ "$(cat "$R")" = "changed 02-group-canvas" ] \
    && [ "$newer" = "./02-group-canvas.md ./manifest.md " ] \
    && grep -qE '^\| 02-group-canvas \| [0-9a-f]{12} \| brief changed, html older than brief \| no \|$' "$O/manifest.md" \
    && [ "$(grep -cE '^\| (00-overview|01-x) \| [0-9a-f]{12} \| ok \| no \|$' "$O/manifest.md")" = 2 ]; then
    ok one_changed_screen_touches_only_that_brief
else bad one_changed_screen_touches_only_that_brief "rc=$rc out=$(cat "$R") newer=$newer"; fi

# 4. html_dir_is_never_touched
mkstage 00-overview 01-x 02-group-canvas
rm -rf "$O"
run >/dev/null 2>&1
mkdir -p "$O/html"
printf '<html>pasted by hand</html>\n' >"$O/html/01-x.html"
printf 'keep me\n' >"$O/html/keep.txt"
cp "$O/html/01-x.html" "$T/x.html"
cp "$O/html/keep.txt" "$T/keep.txt"
sleep 1
touch "$T/marker"
run >"$R" 2>&1
rc=$?
same=0
cmp -s "$T/x.html" "$O/html/01-x.html" && cmp -s "$T/keep.txt" "$O/html/keep.txt" || same=1
r1=$(grep -E '^\| 01-x ' "$O/manifest.md")
if [ "$rc" = 0 ] && [ "$same" = 0 ] && [ -z "$(find "$O/html" -newer "$T/marker")" ] \
    && [ "$(ls "$O/html" | LC_ALL=C sort | tr '\n' ' ')" = "01-x.html keep.txt " ] \
    && echo "$r1" | grep -qE '^\| 01-x \| [0-9a-f]{12} \| new \| yes \|$'; then
    ok html_dir_is_never_touched_by_an_unchanged_run
else bad html_dir_is_never_touched_by_an_unchanged_run "rc=$rc same=$same row=$r1"; fi
sleep 1
touch "$T/marker"
brief 01-x "Changed: a new empty state." >"$S/01-x.md"
run >"$R" 2>&1
rc=$?
same=0
cmp -s "$T/x.html" "$O/html/01-x.html" && cmp -s "$T/keep.txt" "$O/html/keep.txt" || same=1
newer=$(find "$O/html" -newer "$T/marker")
if [ "$rc" = 0 ] && [ "$(cat "$R")" = "changed 01-x" ] && [ "$same" = 0 ] && [ -z "$newer" ] \
    && [ "$(ls "$O/html" | LC_ALL=C sort | tr '\n' ' ')" = "01-x.html keep.txt " ] \
    && grep -qE '^\| 01-x \| [0-9a-f]{12} \| brief changed, html older than brief \| yes \|$' "$O/manifest.md"; then
    ok html_dir_is_never_touched
else bad html_dir_is_never_touched "rc=$rc out=$(cat "$R") same=$same newer=$newer"; fi

# 5. removed_screen_is_marked_and_kept
mkstage 00-overview 01-dispatcher 02-group-canvas
rm -rf "$O"
run >/dev/null 2>&1
mkstage 00-overview 02-group-canvas
run >"$R" 2>&1
rc=$?
if [ "$rc" = 0 ] && [ "$(cat "$R")" = "removed 01-dispatcher" ] && [ -f "$O/01-dispatcher.md" ] \
    && grep -qE '^\| 01-dispatcher \| [0-9a-f]{12} \| removed \| no \|$' "$O/manifest.md"; then
    ok removed_screen_is_marked_and_kept
else bad removed_screen_is_marked_and_kept "rc=$rc out=$(cat "$R")"; fi
mkstage 00-overview 01-connections 02-group-canvas
run >"$R" 2>&1
rc=$?
if [ "$rc" = 2 ] && has "$R" "01-connections reuses number 01 of 01-dispatcher"; then
    ok a_reused_number_is_refused
else bad a_reused_number_is_refused "rc=$rc $(cat "$R")"; fi
mkstage 00-overview 02-group-canvas 03-escalations
run >"$R" 2>&1
rc=$?
if [ "$rc" = 0 ] && [ "$(cat "$R")" = "new 03-escalations" ] \
    && grep -qE '^\| 01-dispatcher \| [0-9a-f]{12} \| removed \| no \|$' "$O/manifest.md"; then
    ok a_new_screen_takes_the_next_number
else bad a_new_screen_takes_the_next_number "rc=$rc out=$(cat "$R")"; fi
mkstage 00-overview 02-group-canvas 03-escalations
run --removed 01-dispatcher >"$R" 2>&1
rc=$?
if [ "$rc" = 0 ] && [ "$(cat "$R")" = "no change" ]; then
    ok removed_flag_names_a_dropped_screen
else bad removed_flag_names_a_dropped_screen "rc=$rc out=$(cat "$R")"; fi
mkstage 00-overview 01-dispatcher 02-group-canvas
run --removed 01-dispatcher >"$R" 2>&1
rc=$?
if [ "$rc" = 2 ] && has "$R" "01-dispatcher is still staged"; then
    ok removed_flag_refuses_a_staged_screen
else bad removed_flag_refuses_a_staged_screen "rc=$rc $(cat "$R")"; fi

# 6. check_rejects_bad_brief
mkstage 00-overview 01-x 02-group-canvas
rm -rf "$O"
run >/dev/null 2>&1
sh "$G" --check "$O" >"$R" 2>&1
rc=$?
if [ "$rc" = 0 ] && [ "$(cat "$R")" = "$O: ok" ]; then ok check_passes_good_briefs
else bad check_passes_good_briefs "rc=$rc $(cat "$R")"; fi
cp "$O/01-x.md" "$T/good.md"
grep -v '^## States' "$O/01-x.md" >"$T/bad.md" && mv "$T/bad.md" "$O/01-x.md"
sh "$G" --check "$O" >"$R" 2>&1
rc=$?
if [ "$rc" = 1 ] && has "$R" "01-x" && has "$R" "States"; then ok check_rejects_missing_section
else bad check_rejects_missing_section "rc=$rc $(cat "$R")"; fi
{ cat "$T/good.md"; printf 'Generated 2026-10-24 10:15 UTC\n'; } >"$O/01-x.md"
sh "$G" --check "$O" >"$R" 2>&1
rc=$?
if [ "$rc" = 1 ] && has "$R" "01-x" && has "$R" "timestamp"; then ok check_rejects_timestamp
else bad check_rejects_timestamp "rc=$rc $(cat "$R")"; fi
cp "$T/good.md" "$O/01-x.md"
rm -f "$O/02-group-canvas.md"
sh "$G" --check "$O" >"$R" 2>&1
rc=$?
if [ "$rc" = 1 ] && has "$R" "02-group-canvas"; then ok check_rejects_listed_file_missing
else bad check_rejects_listed_file_missing "rc=$rc $(cat "$R")"; fi

# 7. the template
f="$TPL"
if [ -f "$f" ] && grep -q '^<!-- template: design-briefs' "$f" && ! grep -qiE 'lorem' "$f" \
    && has "$f" 'design/html/' && has "$f" 'scripts/design-sync.sh' \
    && has "$f" 'CHECK=sh scripts/design-sync.sh --check design' \
    && has "$f" '## Purpose and audience' && has "$f" 'For #96.' \
    && has "$f" 'crates/team/web/dispatcher.html' && has "$f" 'crates/team/src/board.rs' \
    && has "$f" '07-phone' && has "$f" 'WRITE:'; then
    ok template_names_the_rules_and_screens
else bad template_names_the_rules_and_screens "template missing or incomplete: $f"; fi

echo "design_briefs: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
