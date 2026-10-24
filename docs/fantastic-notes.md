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

## Pack registry and listing

The listing (`team_packs.tools`, record fields `index_path`, `templates_dir`, optional `store` and `trusted_keys`) reads a LOCAL index json through the file bridge. Nothing is fetched from the network.

The index is the signed registry format: `{"entries": [...]}`. Each entry has `name`, `version` (dotted numbers, e.g. `1.2.0`), `hash` (the pack's content hash, which its `pack.yaml` pins), `signer_key` (hex ed25519 public key), `url` (https), `ports`, `grants`, `revoked` and `signature` (hex ed25519 over the canonical payload of every other field, `revoked` included). An entry whose signer is not in `trusted_keys`, or whose signature does not verify, is skipped and reported in `skipped`. So a `revoked` flag changed after signing is never honoured. A malformed entry is skipped the same way; one bad entry does not fail the index.

Verbs:

- `list {query?}` replies `{packs: [...]}`: one card per pack name, the newest non-revoked version (or the newest revoked one when all are revoked). Each card marks `installed` when any version of that name is in the templates directory.
- `show {name, version?}` replies the pack's ports, grants and `install_decision` (`{allow: true}` or `{allow: false, reason: "revoked"}`). The UI disables Install on a refusal.
- `updates {opt_in?}` replies `{updates: [...], warnings: [...]}`. Updates are opt-in: with `opt_in` false (the default) `updates` is empty. An update is the newest non-revoked version with a different hash, with its `permission_diff` (added and removed ports and grants). A revoked installed pack gets a warning. `updates` installs nothing and fetches nothing.

An unsigned legacy index (`{"packs": [...]}`) is still read for display. Its entries are unverified: nothing trusts them, and a revoked one refuses install.

Installing stays `pack_bundle install` from a local pack directory.

Not built yet: the publish and signing command, the automated review step, report and takedown, fetching index and pack files by url, the aisixteen.com listing site and the in-app browser, evals, measured record and price inside the signed payload (the payload does not cover them, so they are not trusted), install from the registry by hash (resolve, fetch, then `pack_bundle::install`).
