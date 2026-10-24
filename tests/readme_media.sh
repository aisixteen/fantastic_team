#!/bin/sh
# sh tests/readme_media.sh — the README shows the animated preview of
# media/how_it_works.mp4 and links the mp4; both files exist; the preview is
# animated, 960 px wide and under the 8 MB cap; media/render-preview.sh
# regenerates it (that check runs only when ffmpeg is installed).
set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
README="$ROOT/README.md"
PREVIEW="$ROOT/media/how_it_works-preview.gif"
VIDEO="$ROOT/media/how_it_works.mp4"
RENDER="$ROOT/media/render-preview.sh"
CAP=8000000
T=$(mktemp -d "${TMPDIR:-/tmp}/readme_media.XXXXXX")
trap 'rm -rf "$T"' EXIT
PASS=0
FAIL=0
ok() { PASS=$((PASS + 1)); echo "ok   $1"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL $1${2:+ — $2}"; }
skip() { echo "skip $1 — $2"; }

# size FILE: bytes, or 0 when missing
size() { if [ -f "$1" ]; then wc -c <"$1" | tr -d ' '; else echo 0; fi; }

# frames FILE: number of frames (ffprobe), or empty when ffprobe is missing
frames() {
    command -v ffprobe >/dev/null 2>&1 || return 0
    ffprobe -v error -count_frames -select_streams v:0 \
        -show_entries stream=nb_read_frames -of csv=p=0 "$1" 2>/dev/null
}

# animated FILE: more than one frame (ffprobe), else the GIF loop extension
# that ffmpeg writes for an animated GIF
animated() {
    [ -f "$1" ] || return 1
    n=$(frames "$1")
    if [ -n "$n" ]; then [ "$n" -gt 1 ]; else grep -aq 'NETSCAPE2.0' "$1"; fi
}

# gif_width FILE: logical screen width, bytes 6..7 little endian
gif_width() {
    set -- $(od -An -tu1 -j6 -N2 "$1")
    echo $(($1 + $2 * 256))
}

# find ffmpeg the same way media/render-preview.sh does
FFMPEG=$(command -v ffmpeg 2>/dev/null || true)
[ -n "$FFMPEG" ] || { [ -x /opt/homebrew/bin/ffmpeg ] && FFMPEG=/opt/homebrew/bin/ffmpeg; }

# 1. readme_links_preview_and_mp4
if grep -qF 'media/how_it_works-preview.gif' "$README" &&
    grep -qF 'media/how_it_works.mp4' "$README" &&
    ! grep -qF 'fantastic_shift' "$README"; then
    ok readme_links_preview_and_mp4
else
    bad readme_links_preview_and_mp4 "README must link the preview and the mp4, with no fantastic_shift URL"
fi

# 2. media_files_exist
if [ -s "$PREVIEW" ] && [ -s "$VIDEO" ]; then
    ok media_files_exist
else
    bad media_files_exist "missing or empty: $PREVIEW or $VIDEO"
fi

# 3. preview_is_animated_and_small
n=$(size "$PREVIEW")
if [ "$n" -gt 0 ] && [ "$n" -lt "$CAP" ] && animated "$PREVIEW"; then
    ok preview_is_animated_and_small
else
    bad preview_is_animated_and_small "size $n bytes (cap $CAP)"
fi

# 4. preview_is_960_wide
if [ -s "$PREVIEW" ] && [ "$(gif_width "$PREVIEW")" = 960 ]; then
    ok preview_is_960_wide
else
    bad preview_is_960_wide "width $(gif_width "$PREVIEW" 2>/dev/null || echo none)"
fi

# 5. readme_top_section_precedes_intro_end
pl=$(grep -nF 'media/how_it_works-preview.gif' "$README" | head -1 | cut -d: -f1)
il=$(grep -nF 'It is not only for code.' "$README" | head -1 | cut -d: -f1)
if [ -n "$pl" ] && [ -n "$il" ] && [ "$pl" -lt "$il" ]; then
    ok readme_top_section_precedes_intro_end
else
    bad readme_top_section_precedes_intro_end "preview line ${pl:-none}, intro end line ${il:-none}"
fi

# 6. render_script_reproduces (needs ffmpeg; skipped without it)
if [ -z "$FFMPEG" ]; then
    skip render_script_reproduces "ffmpeg not installed"
elif [ ! -f "$RENDER" ]; then
    bad render_script_reproduces "missing $RENDER"
else
    out="$T/regen.gif"
    if sh "$RENDER" "$VIDEO" "$out" >"$T/log" 2>&1 && [ "$(size "$out")" -gt 0 ] &&
        [ "$(size "$out")" -lt "$CAP" ] && animated "$out" && [ "$(gif_width "$out")" = 960 ]; then
        ok render_script_reproduces
    else
        bad render_script_reproduces "$(tail -5 "$T/log")"
    fi
fi

echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
