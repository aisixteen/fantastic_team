# Notes for fantastic: what the canvas zip needs to be reproducible

Team's builds (#137) are reproducible except for one input: the canvas zip that `fantastic-canvas-page` embeds in the binary. Its packer lives in the fantastic dependency, and Team does not patch dependencies. These are the changes fantastic has to make. Found while reading `src/lib/ts/scripts/pack.sh` and `src/tui/web/canvas/scripts/pack.sh` at fantastic commit `e7ede96` (the commit in `Cargo.lock`).

## 1. Stamp the staged files with SOURCE_DATE_EPOCH before zipping

`pack.sh` line 139 is `( cd "$OUT" && zip -q -j "$ZIP" readme.md bundle.min.js bundle.min.js.map )`. The staged files get their mtimes when they are written, so the zip's entries change on every build. Before the zip step, set the three staged files' mtimes to `$SOURCE_DATE_EPOCH` (default 0 when unset), portably:

- GNU `touch -d "@$SOURCE_DATE_EPOCH"` and BSD/macOS `touch -t` differ. Use `TZ=UTC touch -t "$(TZ=UTC date -u -d "@$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S)"` where GNU date is present, and `TZ=UTC date -u -r "$SOURCE_DATE_EPOCH" +%Y%m%d%H%M.%S` on BSD. Either way the three files get the same time.
- The Git-for-Windows `sh` used on Windows has GNU `date` and `touch`, so the first form covers it.

## 2. Zip with -X and -D, in the same fixed order

`zip -q -j` keeps the explicit entry order (readme.md, bundle.min.js, bundle.min.js.map), which is already fixed. Add `-X` (no extra fields: uid, gid, extended timestamps) and `-D` (no directory entries). Keep the explicit list; do not glob.

Result: the zip's bytes depend only on the three files' contents and the timestamp from step 1.

## 3. Check the source map for absolute paths

Not verified. The esbuild call is `"$ESBUILD" "$ENTRY" --bundle ... --sourcemap ... --outfile="$OUT/bundle.min.js"` run from `$CALLER`. If `bundle.min.js.map` has `"sources"` entries with absolute paths (look at `grep -o '"sources":\[[^]]*\]' bundle.min.js.map`), the zip differs between checkouts even with steps 1 and 2. Fix: pass a relative entry and run esbuild from a fixed directory, or use `--source-root` with relative sources.

## 4. Scope

- `src/tui/web/canvas/scripts/pack.sh` calls the library's pack.sh, so the same change covers both. Its staging dir is under `OUT_DIR` in `build.rs`, which does not change the result.
- `esbuild` itself is already reproducible: it is built from pinned source with the pinned Go toolchain and checked by sha256.

## Team side

Team's `repro` CI job (`.github/workflows/ci.yml`) compares `SHA256SUMS` from two builds. Until steps 1 to 3 land, that comparison differs; the job warns, and fails only with `-f strict=true`. When fantastic lands them, bump the fantastic commit in `Cargo.lock` (`scripts/update-fantastic.sh`) and turn `strict` on for the release run.
