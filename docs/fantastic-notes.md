# Notes for fantastic: what the canvas zip needs to be reproducible

Team's builds (#137) are reproducible except for one input: the canvas zip that `fantastic-canvas-page` embeds in the binary. Its packer lives in the fantastic dependency, and Team does not patch dependencies. These are the changes fantastic has to make. Found while reading `src/lib/ts/scripts/pack.sh` and `src/tui/web/canvas/scripts/pack.sh` at fantastic commit `717765e` (the commit `Cargo.lock` pinned at the time of that reading).

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

Team's `repro` CI job (`.github/workflows/ci.yml`) compares `SHA256SUMS` from two builds. The steps landed in fantastic #409, the pin in `Cargo.lock` was bumped (`scripts/update-fantastic.sh`), and `strict` is on by default: a difference fails the job (`-f strict=false` only warns).

## Pin at `b175660`

`Cargo.lock` now pins fantastic `b175660`. It brings `fantastic_tui::seed_dir(dir) -> Result<bool, String>` (fantastic#444: the shell's first-run seeding, a no-op once `.fantastic` exists) and spawn applying the card's tool grants on both ends (fantastic#436: a card may only grant what its spawner holds, so the lead holds `board` and the root's await verbs). Team calls `seed_dir` before it boots a fresh directory (`run_at_start`, `persist_web`), so the first `--port` start serves the page; `seed_base_tree` keeps the stores the seed wrote.

## Pin at `b980705`

`Cargo.lock` now pins fantastic `b980705`. It brings the durable store `sqlite_store` (fantastic#432, crate `fantastic-sqlite-store`, handler `sqlite_store.tools`: ram_store's verbs over one SQLite file per store agent, record field `path` required and relative to the project directory, `close_all` after the kernel's shutdown) and the #424 active-count fix (`get_count` counts the agents that are running). The build and tests pass with no Team code change.

## Pin at `69971e5`

`Cargo.lock` now pins fantastic `69971e5`. It brings the task registry `task_registry` (fantastic#429) and drops the #424 task lines: spawn `task` is a string only, `fleet` entries are `{id}` without a state, and `get_count` counts every spawned agent that still exists.

## Pin at `7bf494e`

`Cargo.lock` now pins fantastic `7bf494e`. It brings the generic roles registry `agentic_resources` (fantastic#424), fork points (#425), `await_set` (#426) and spawn from a fork point (#431). Team configures the registry for the coder role set (#185).

## Pin at `3535af5`

`Cargo.lock` now pins fantastic `3535af5`. It brings #436 (`spawn` opens the tool's `callable_by` for every grant on the card, on both ends, all or nothing; a card may only grant what the spawner itself holds, else `not_allowed`; `delete_agent` takes the child out again) and #437 (`task_registry` rows carry `versions`, `source_version`, `fork_from` and `usage`; `report` takes `usage`; `list` sums it per batch). Team code change: the lead's `can_call` now holds `board [board, rows]` (when the team has a board) and `kernel_state [await_set, await_cancel]` (unless its template grants `kernel_state`), the verbs the `project_lead` card hands on. The manual admit is gone from the cards. The credential store is still not opened by a spawn: a tier's `credentials` ref is no card grant.

## What Team gets from the bump

`Cargo.lock` now pins fantastic `bab97de` (aisixteen/fantastic main, 11 commits past `df64468`, which was 40 commits past `ef6f9a7`, which was 11 commits past `717765e`, which was 24 commits past `e7ede96`). Only the fantastic crates moved (the 11 newest commits are e2e and CI fixes, no API change), after the earlier bump added the transitive crates that `fantastic-system-agents` needs (`zbus` and its family, from the fantastic lock entries). The build and tests pass with no Team code change. Features the bump brings to Team:

- Blueprints (fantastic#357, for #268 and #269 which stay open): `user.export_subtree {agent_id, path}` writes an agent and its subtree as a blueprint folder, with ports for references that point outside it (`@memory`, `@credentials`, `@exec`, `@files`, `@model`, `@connector:<id>`, `@path:<field>`). Key values and absolute paths are stripped, and a secrets scan runs before the rename. `user.import_subtree` installs it under any parent with fresh ids (one `_<k>` suffix), ports bound and granted back, and a preview until `approve: true`. It refuses unbound ports, key values, absolute paths, and code without `allow_code`. This is what worker packs (#40) need; Team does not use it yet.
- Atomic writes survive a transient rename refusal: fantastic retries a refused atomic-write rename a bounded 8 times and removes the sibling tmp file on final failure (fantastic#352). This helps claim files.
- Inline agent views in a chat: `embed {id}` and `unembed {id}` plus chat cards, so a lead's chat can hold worker views as live cards (fantastic#339, #340).
- File and folder view agents over the file bridge, for the canvas and the CLI (fantastic#336).
- The file tree as the canvas's own fixed, collapsible sidebar, not zoomed (fantastic#338).
- Atomic grant edits: `callable_by` and `can_call` change under one lock (fantastic#324).
- Secret marks: a store on a connected kernel is recorded as secret by the calling kernel, for credential-store calls (fantastic#326).
- Exec-bridge refuses mail to a sealed leg, so refused mail never reaches the leg's program (fantastic#335, #350).

Not relevant to Team: the headless container image (fantastic#337), the Swift kernel retirement that keeps the Foundation Models bridge (fantastic#354, #359), the OS system agents and the audio agent (fantastic#355, #360), the Python ports (the `#170` series, fantastic#353 and #362), the `units_os` CI input (fantastic#356), the file agent follow-ups (fantastic#358) and the e2e model per role (fantastic#361).

## Pack registry and listing

The listing (`team_packs.tools`, record fields `index_path`, `templates_dir`, optional `store` and `trusted_keys`) reads a LOCAL index json through the file bridge. Nothing is fetched from the network.

The index is the signed registry format: `{"entries": [...]}`. Each entry has `name`, `version` (dotted numbers, e.g. `1.2.0`), `hash` (the pack's content hash, which its `pack.yaml` pins), `signer_key` (hex ed25519 public key), `url` (https), `ports`, `grants`, `revoked` and `signature` (hex ed25519 over the canonical payload of every other field, `revoked` included). An entry whose signer is not in `trusted_keys`, or whose signature does not verify, is skipped and reported in `skipped`. So a `revoked` flag changed after signing is never honoured. A malformed entry is skipped the same way; one bad entry does not fail the index.

Verbs:

- `list {query?}` replies `{packs: [...]}`: one card per pack name, the newest non-revoked version (or the newest revoked one when all are revoked). Each card marks `installed` when any version of that name is in the templates directory.
- `show {name, version?}` replies the pack's ports, grants and `install_decision` (`{allow: true}` or `{allow: false, reason: "revoked"}`). The UI disables Install on a refusal.
- `updates {opt_in?}` replies `{updates: [...], warnings: [...]}`. Updates are opt-in: with `opt_in` false (the default) `updates` is empty. An update is the newest non-revoked version with a different hash, with its `permission_diff` (added and removed ports and grants). A revoked installed pack gets a warning. `updates` installs nothing and fetches nothing.

An unsigned legacy index (`{"packs": [...]}`) is still read for display. Its entries are unverified: nothing trusts them, and a revoked one refuses install.

- `publish {pack, url, signing_key, grants?}` replies `{published: {name, version, hash, signer_key}, trusted, note?}`. It is local: no network. It checks the pack folder like an install (manifest, content hash, secrets scan; nothing is signed or written when a check fails), reads the 64-hex ed25519 seed from the file bridge file `signing_key`, signs the entry (`pack_registry::sign_entry`) and appends it to the local index (`pack_registry::add_entry`), keeping existing entries as they are. `url` must be https. `grants` comes only from the payload, since `pack.yaml` has none. It refuses an existing name+version or hash. The key never appears in a reply or error. The signer need not be trusted: `trusted` is false, with a note, when its key is not in the listing's `trusted_keys`, because `list` skips such an entry.

Installing stays `pack_bundle install` from a local pack directory.

Not built yet: setting `trusted_keys` for the listing (`tree.rs` leaves it unset, so a real team skips every signed entry), a key-generation command and key rotation, a richer automated review step (publish runs the pack scan only), report and takedown (a signed re-publish with `revoked: true`, then a `revoke` verb), fetching index and pack files by url, the aisixteen.com listing site and the in-app browser, evals, measured record and price inside the signed payload (the payload does not cover them, so they are not trusted), install from the registry by hash (resolve, fetch, then `pack_bundle::install`).

## Pin at `22f88cb`

`Cargo.lock` now pins fantastic `22f88cb`. It brings the route-clash refusal (fantastic#446: web boot fails and `mount` errors when two children declare the same literal route), `?canvas=<id>` in the canvas page, and ai_core `embed`/`unembed` (history rows `{kind:"embed"|"unembed", head:<agent id>}`, no role). Team's page, ws and canvas routes are distinct, so none clashes.
