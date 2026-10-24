#!/bin/sh
# sh media/render-preview.sh [INPUT.mp4 [OUTPUT.gif]] — makes the animated
# README preview of the how-it-works video: 8 frames a second, 960 px wide,
# looping, with a palette taken from the video (GitHub does not play repo mp4s
# inline; it does show a GIF). Defaults: media/how_it_works.mp4 to
# media/how_it_works-preview.gif. Needs ffmpeg on PATH, or /opt/homebrew/bin/ffmpeg.
# Exit 127: no ffmpeg. Exit 1: failed. Exit 2: the result is over the 8 MB cap.
set -eu
DIR=$(cd "$(dirname "$0")" && pwd)
IN=${1:-$DIR/how_it_works.mp4}
OUT=${2:-$DIR/how_it_works-preview.gif}
CAP=8000000
FFMPEG=$(command -v ffmpeg 2>/dev/null || true)
if [ -z "$FFMPEG" ] && [ -x /opt/homebrew/bin/ffmpeg ]; then FFMPEG=/opt/homebrew/bin/ffmpeg; fi
if [ -z "$FFMPEG" ]; then
    echo "render-preview: ffmpeg not found" >&2
    exit 127
fi
if [ ! -f "$IN" ]; then
    echo "render-preview: no input $IN" >&2
    exit 1
fi
"$FFMPEG" -hide_banner -loglevel error -y -i "$IN" \
    -filter_complex "fps=8,scale=960:-2:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" \
    -map_metadata -1 -loop 0 "$OUT"
SIZE=$(wc -c <"$OUT" | tr -d ' ')
if [ "$SIZE" -ge "$CAP" ]; then
    echo "render-preview: $OUT is $SIZE bytes, over the $CAP byte cap" >&2
    rm -f "$OUT"
    exit 2
fi
echo "render-preview: wrote $OUT ($SIZE bytes)"
