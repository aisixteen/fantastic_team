#!/bin/sh
# design-sync.sh — keeps an app's design/ briefs in step with a staging dir (templates/design-briefs.md).
#   design-sync.sh --app APP --commit SHA --staging DIR --out DIR [--removed NN-name ...]
#   design-sync.sh --check DIR
# STAGING holds one NN-name.md per screen, written from scratch by the worker. A run copies only the
# briefs that differ, keeps every screen's number, and rewrites manifest.md only when something other
# than the "last run" line changed. README.md is written once. Nothing under OUT/html/ is ever
# created, written or deleted.
# stdout: one line per change (new|changed|removed NN-name), or "no change". Exit 0 for a run;
# --check exits 1 when a brief is bad; 2 for a usage error or a refused run.
set -eu
LC_ALL=C
export LC_ALL
PROG=design-sync
USAGE='usage: design-sync.sh --app APP --commit SHA --staging DIR --out DIR [--removed NN-name ...] | --check DIR'
STALE='brief changed, html older than brief'
SECTIONS='## Purpose and audience
## Data shown
## States
## Actions
## Shared rules'

die() {
    printf '%s: %s\n' "$PROG" "$1" >&2
    exit "${2:-2}"
}

# hash_of FILE — the first 12 hex digits of its SHA-256.
hash_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -c1-12
    else
        sha256sum "$1" | cut -c1-12
    fi
}

# check_dir DIR — every brief has the required sections and no timestamp or lorem ipsum; the
# manifest lists only files that exist.
check_dir() {
    d=$1
    [ -d "$d" ] || die "no such directory: $d"
    bad=0
    if [ ! -f "$d/manifest.md" ]; then
        printf '%s: %s/manifest.md is missing\n' "$PROG" "$d" >&2
        bad=1
    fi
    for f in "$d"/[0-9][0-9]-*.md; do
        [ -f "$f" ] || continue
        n=${f##*/}
        while IFS= read -r h; do
            grep -qxF -- "$h" "$f" || { printf '%s: %s: missing section `%s`\n' "$PROG" "$n" "$h" >&2; bad=1; }
        done <<EOM
$SECTIONS
EOM
        if grep -qE '[0-9]{4}-[0-9]{2}-[0-9]{2}[T ][0-9]{2}:[0-9]{2}' "$f"; then
            printf '%s: %s: timestamp-like line\n' "$PROG" "$n" >&2
            bad=1
        fi
        if grep -qi 'lorem' "$f"; then
            printf '%s: %s: lorem ipsum\n' "$PROG" "$n" >&2
            bad=1
        fi
    done
    if [ -f "$d/manifest.md" ]; then
        listed=$(awk -F'|' '/^\| [0-9][0-9]-/ { n = $2; gsub(/ /, "", n); print n }' "$d/manifest.md")
        while IFS= read -r n; do
            [ -n "$n" ] || continue
            [ -f "$d/$n.md" ] || { printf '%s: manifest lists %s, which has no file\n' "$PROG" "$n" >&2; bad=1; }
        done <<EOM
$listed
EOM
    fi
    [ "$bad" = 0 ] || exit 1
    printf '%s: ok\n' "$d"
    exit 0
}

[ $# -gt 0 ] || die "$USAGE"
case $1 in
    -h | --help) echo "$USAGE"; exit 0 ;;
    --check) [ $# = 2 ] || die "$USAGE"; check_dir "$2" ;;
esac

APP= COMMIT= STAGING= OUT= REMOVED=
while [ $# -gt 0 ]; do
    case $1 in
        --app) [ $# -ge 2 ] || die "--app needs a value"; APP=$2; shift 2 ;;
        --commit) [ $# -ge 2 ] || die "--commit needs a value"; COMMIT=$2; shift 2 ;;
        --staging) [ $# -ge 2 ] || die "--staging needs a directory"; STAGING=$2; shift 2 ;;
        --out) [ $# -ge 2 ] || die "--out needs a directory"; OUT=$2; shift 2 ;;
        --removed)
            shift
            while [ $# -gt 0 ]; do
                case $1 in --*) break ;; esac
                REMOVED="$REMOVED $1"
                shift
            done ;;
        *) die "unexpected argument \`$1\` — $USAGE" ;;
    esac
done
{ [ -n "$APP" ] && [ -n "$COMMIT" ] && [ -n "$STAGING" ] && [ -n "$OUT" ]; } || die "$USAGE"
case $APP in *[!A-Za-z0-9._-]*) die "--app takes a plain name, got \`$APP\`" ;; esac
[ -d "$STAGING" ] || die "no such staging directory: $STAGING"
DATE=${DESIGN_SYNC_DATE:-$(date -u +%Y-%m-%d)}

W=$(mktemp -d "${TMPDIR:-/tmp}/design-sync.XXXXXX")
trap 'rm -rf "$W"' EXIT
mkdir -p "$OUT"

# The staged screens, sorted. Each name is NN-name with a-z, 0-9 and -.
: >"$W/staged.raw"
for f in "$STAGING"/*.md; do
    [ -f "$f" ] || continue
    n=${f##*/}
    n=${n%.md}
    case $n in
        [0-9][0-9]-*) case $n in *[!a-z0-9-]*) die "brief $n.md: the name takes a-z, 0-9 and -" ;; esac ;;
        *) die "brief $n.md: the name must be NN-name.md" ;;
    esac
    printf '%s\n' "$n" >>"$W/staged.raw"
done
LC_ALL=C sort "$W/staged.raw" >"$W/staged"
[ -s "$W/staged" ] || die "no briefs in $STAGING"
dups=$(cut -c1-2 "$W/staged" | uniq -d)
[ -z "$dups" ] || die "two briefs share the number $dups"

# What is on disk and in the manifest. prev holds "name|status" for each manifest row.
: >"$W/outnn"
for f in "$OUT"/[0-9][0-9]-*.md; do
    [ -f "$f" ] || continue
    n=${f##*/}
    printf '%s\n' "${n%.md}" >>"$W/outnn"
done
: >"$W/prev"
if [ -f "$OUT/manifest.md" ]; then
    awk -F'|' '/^\| [0-9][0-9]-/ { n = $2; s = $4; gsub(/^ +| +$/, "", n); gsub(/^ +| +$/, "", s); print n "|" s }' \
        "$OUT/manifest.md" >"$W/prev"
fi
cut -d'|' -f1 "$W/prev" >"$W/prevnames"
LC_ALL=C sort -u "$W/outnn" "$W/prevnames" >"$W/known"

status_of() { awk -F'|' -v n="$1" '$1 == n { print $2; exit }' "$W/prev"; }

# A new screen may not take a number that any screen has used, removed ones included.
while IFS= read -r s; do
    grep -qxF -- "$s" "$W/known" && continue
    nn=$(printf '%s' "$s" | cut -c1-2)
    clash=$(awk -v nn="$nn" 'substr($0, 1, 2) == nn { print; exit }' "$W/known")
    [ -z "$clash" ] || die "$s reuses number $nn of $clash — a number is never reused"
done <"$W/staged"

for r in $REMOVED; do
    grep -qxF -- "$r" "$W/staged" && die "$r is still staged; drop its brief before naming it in --removed"
    grep -qxF -- "$r" "$W/known" || die "$r is not a known screen"
done

# Pass A: copy the briefs that differ. kinds holds "name|kind".
: >"$W/kinds"
while IFS= read -r s; do
    src="$STAGING/$s.md"
    brief="$OUT/$s.md"
    prev=$(status_of "$s")
    if [ ! -f "$brief" ]; then
        cp "$src" "$brief"
        k=new
    elif ! cmp -s "$src" "$brief"; then
        cp "$src" "$brief"
        k=changed
    elif [ "$prev" = removed ]; then
        k=restored
    else
        k=same
    fi
    printf '%s|%s\n' "$s" "$k" >>"$W/kinds"
done <"$W/staged"

# Pass B: screens not staged. A file that is gone takes no row; one that stays is removed.
while IFS= read -r k; do
    grep -qxF -- "$k" "$W/staged" && continue
    if [ -f "$OUT/$k.md" ]; then
        if [ "$(status_of "$k")" = removed ]; then rk=kept; else rk=removed; fi
    else
        rk=gone
    fi
    printf '%s|%s\n' "$k" "$rk" >>"$W/kinds"
done <"$W/known"

: >"$W/changes"
while IFS='|' read -r s k; do
    case $k in
        new) printf 'new %s\n' "$s" >>"$W/changes" ;;
        changed | restored) printf 'changed %s\n' "$s" >>"$W/changes" ;;
        removed | gone) printf 'removed %s\n' "$s" >>"$W/changes" ;;
    esac
done <"$W/kinds"
if [ -s "$W/changes" ]; then anychange=yes; else anychange=no; fi

# Statuses. A status is kept from the last run that set it, except that a "new" row becomes
# "ok" in a run that changes something else, and a stale row becomes "ok" once its html is
# newer than its brief.
: >"$W/all"
while IFS='|' read -r s k; do
    brief="$OUT/$s.md"
    prev=$(status_of "$s")
    case $k in
        new) st=new ;;
        changed) st=$STALE ;;
        restored) st=ok ;;
        removed | kept) st=removed ;;
        gone) continue ;;
        same)
            st=${prev:-ok}
            if [ "$st" = new ] && [ "$anychange" = yes ]; then st=ok; fi
            if [ "$st" = "$STALE" ] && [ -f "$OUT/html/$s.html" ] \
                && [ -n "$(find "$OUT/html/$s.html" -newer "$brief")" ]; then
                st=ok
            fi ;;
    esac
    printf '%s|%s\n' "$s" "$st" >>"$W/all"
done <"$W/kinds"

LC_ALL=C sort "$W/all" >"$W/all.sorted"
: >"$W/table"
while IFS='|' read -r s st; do
    h=$(hash_of "$OUT/$s.md")
    if [ -f "$OUT/html/$s.html" ]; then hp=yes; else hp=no; fi
    printf '| %s | %s | %s | %s |\n' "$s" "$h" "$st" "$hp" >>"$W/table"
done <"$W/all.sorted"

# manifest.md: the body (everything but the "last run" line) decides whether it is rewritten.
emit() {
    printf '# Design manifest\n\n'
    printf 'app: %s\n' "$APP"
    printf 'source commit: %s\n' "$COMMIT"
    [ -z "$1" ] || printf 'last run: %s\n' "$1"
    printf '\n| screen | brief hash | status | html present |\n| --- | --- | --- | --- |\n'
    cat "$W/table"
}
emit "" >"$W/body.new"
body_same=no
if [ -f "$OUT/manifest.md" ]; then
    sed '/^last run: /d' "$OUT/manifest.md" >"$W/body.old"
    cmp -s "$W/body.old" "$W/body.new" && body_same=yes
fi
if [ "$body_same" = no ]; then
    emit "$DATE" >"$OUT/manifest.md"
fi

if [ ! -f "$OUT/README.md" ]; then
    cat >"$OUT/README.md" <<'EOM'
# Design briefs

This folder carves one app's screens into briefs for Claude Design. `scripts/design-sync.sh` writes it from a run of the `design-briefs` task. The loop:

1. **Brief.** `NN-screen.md` says what the screen is for, what data it shows, its states and its actions.
2. **Claude Design.** Give the brief to Claude Design and get the screen as HTML.
3. **html/.** Save the page as `html/NN-screen.html`. Only the maintainer writes here. The task never creates, changes or deletes anything in `html/`.
4. **UI.** Map the pasted page into the app's UI.

`manifest.md` lists each screen with its brief hash, its status and whether its html exists. When a brief changes, its status reads "brief changed, html older than brief" until a newer html file is pasted over the old one.
EOM
fi

if [ -s "$W/changes" ]; then
    LC_ALL=C sort -k2,2 "$W/changes"
else
    echo "no change"
fi
exit 0
