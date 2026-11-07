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

## Pin at `8eaa08a`

`Cargo.lock` now pins fantastic `8eaa08a` (#453: declared TUI views, `view.tui` on a record). Team declares `view.tui` for its agent kinds in `declared_views.rs` (#207); fantastic's panel parser is private, so team validates the shape itself.

## Pin at `f27b3ba`

`Cargo.lock` now pins fantastic `f27b3ba` (#451/#452: agentic_resources and task_registry refuse in the kernel shape: `error` = the sentence, `reason` = the code such as `limit` or `not_allowed`, plus `hint`). Team change: the lead and project_lead cards, the README and the tests read the code from `reason`.

## Pin at `22f88cb`

`Cargo.lock` now pins fantastic `22f88cb`. It brings the route-clash refusal (fantastic#446: web boot fails and `mount` errors when two children declare the same literal route), `?canvas=<id>` in the canvas page, and ai_core `embed`/`unembed` (history rows `{kind:"embed"|"unembed", head:<agent id>}`, no role). Team's page, ws and canvas routes are distinct, so none clashes.

## Pin at `5e74eb5`

`Cargo.lock` now pins fantastic `5e74eb5` (#467/#468: reach writes a new agent's `*` grants before `create_agent` returns; #469/#470: the CLI host registers `ws_bridge` and `relay_connector`). The roles registry `agentic_resources` and the task registry `task_registry` moved from fantastic into this repo (#245): `crates/agentic_resources` (package `team-agentic-resources`) and `crates/task_registry` (package `team-task-registry`), copied from fantastic `5e74eb5` with the same handler names, verbs, fields and refusals. Two changes: the registry ships no built-in `project_lead` card (team's `roles/project_lead.md` is the only one), and `spawn` (plain and `from`) MERGES the parent's `pause`/`resume`/`stop` into what the parent already holds on the child instead of overwriting it, so under reach the spawner keeps `*` (`send`, `history`) on what it spawned. No fantastic bundle crate for either registry is left in `Cargo.lock`; fantastic deletes its copies after this (fantastic#471).

## Cabinet moved (#248), pin still `5e74eb5`

The cabinets bundle `cabinet.tools` moved from fantastic into this repo (#248): `crates/cabinet` (package `team-cabinet`), copied from fantastic `8ee7e3f` (`src/lib/rust/crates/bundles/fantastic-cabinet`: `lib.rs`, `tests.rs`, `readme.md`, `view.html`; the crate is the same at `5e74eb5`, so `Cargo.lock` keeps that pin). Same handler name, verbs, fields, refusals, grants, wiring and status-cell html; its units moved with it. `register_team` registers it after fantastic's stock set, so team's copy is the one a team kernel and team's canvas dispatch to. It still asks fantastic's canvas (`canvas.tools`) for `new_canvas`, which stays in fantastic. Until fantastic deletes its copy (fantastic#474), `fantastic-cabinet` stays in `Cargo.lock` as a dependency of `fantastic-tui`. The Python twin is not ported.

## Pin at `a9b7a31`

`Cargo.lock` now pins fantastic `a9b7a31` (main after #476/#479, the two-way kernel bridge over WebSocket). `ws_bridge` says `hello {guid, leg}` at every boot (`reconnect` included); `web_ws` registers that connection as the dialling kernel's leg, so the far kernel reaches the dialler's agents as `<guid>|<agent>`. Those calls are served AS THE LEG (its `ingress_rule`, sealed by default, its `can_call`, the target's `callable_by`); a far `watch`/`unwatch` of `<guid>|<agent>` streams the agent's events back as `event` frames. On a drop, calls in flight settle at once with `bridge_down`, the far side keeps its watches per guid and re-sends them on the next `hello`, and the newest `hello` for a guid replaces the older connection. It also brings fantastic#474 (fantastic's own `fantastic-cabinet` deleted: it leaves `Cargo.lock`, team's `crates/cabinet` is the only copy) and #471 (fantastic's registry copies deleted). The build and tests pass with no Team code change.

What the ssh twins measured at `a9b7a31` (#252: the backends and the key are on the master, a remote agent sends and watches them over the one bridge; `e2e/helpers/ssh_twins.py`):

- **A remote agent that wants a model does not need one.** The box's exec agent (`can_call` `<master guid>|*`: send, watch, status, stop) runs a program with fantastic's connector: `kernel.watch("<guid>|<backend>")`, then `kernel.send(...)` with its own `client_id`. The master serves both AS THE LEG (`ws_bridge` `ingress_rule` open, its `can_call` and the backend's `callable_by` name exactly `send`, `watch`, `status`, `stop`), the backend's turn runs on the master, and its events come back as `event` frames in the order the backend emitted them: `status` (`thinking`, `streaming`, `done`), `token`, `part_*`.
- **Which parts a watcher gets.** `PartPublisher::send` (`fantastic-ai-core/src/events.rs:132`): a text part goes to the backend's own inbox for any client but `cli`; a call part goes to the agents it calls. A turn of calls alone shows its watcher `status` and `token` events and no `part_*`: a backend that is watched opens its turn with words. A text-only turn is no end of a run (the backend asks the model again until a `done` call), so a scripted turn ends with `done`.
- **A watch mirrors the inbox.** The watcher also gets every payload sent to the backend, the prompt text of each `send` among them, and other callers' calls (`reflect`). `watch` is a read of the whole inbox: grant it as such.
- **A refused watch is silent.** The master answers `{type:"error", id, src, ...}` (`fantastic-bridge/src/lib.rs:1512` `refuse_watch`); the remote's `ws_peer` `answer` (`fantastic-web/src/ws_peer.rs:491`) finds no call waiting on that id and drops it. With the watch grant sealed the watcher is deaf and is told nothing; the twin's check sees it as no event.
- **No redial by itself.** A ws `ws_bridge` after a drop stays down (`readme_ws.md`, `lib.rs:141`: `reconnect` is "explicit"); the operator (or a lead) sends `reconnect` (teardown, then `boot`, which says `hello` again). Calls in flight settle `bridge_down` at once on the side that was waiting; the remote kernel keeps its watches per guid and asks for them again on the next `hello` (`ws_peer.rs:444-446`), so a watcher made before the break streams the next turn with no new `watch`.
- **An in-flight call dies with the drop, and its backend stays `thinking`.** On a drop the read loop aborts every inbound dispatch (`fantastic-bridge/src/lib.rs:1759` `h.abort()`): the backend's `send` future is dropped in mid-turn, nothing closes the turn out (`on_cancel` is the kernel's cancel, not an abort), `status` keeps `generating: true`, `phase: "thinking"` for ever, and a later `send` to it is held behind that turn. The reconnect twin keeps the call that is in flight to a backend of its own (`w3`) for this reason.

## Pin at `a6975ea`

`Cargo.lock` now pins fantastic `a6975ea` (main after fantastic#480, `fantastic_tui::seed_dir_with(dir, &[blueprint])`: a host writes its own agents after the stock tree at the first run, ids as written, by the same writer: no `parent_id`, `@root` resolved, disk position is the parent; an entry that names an earlier blueprint's id is a parent entry that carries only `id` and `agents`). Team uses it for `crates/team/blueprint.yaml` (#261), and does the same as fantastic does for its own stock agents, nothing more:

- **First run.** `hydrate::seed` (called by `persist_web`, the `webui` word) writes the fixed agents with their grants and config from the yaml. `@work` and the worker's exec record are filled in first, because they are absolute paths of the folder and the GitHub build narrows `allowed` by patch.
- **Every run after.** The fs loader (`kernel_state`) is the root loader and rehydrates `.fantastic/agents/**`; fantastic's own boot (`Host::boot`, `kern/host.rs`) runs `reach::grant_all` (`kern/reach.rs`), which writes both ends of the edges of the human and of every AI agent (a record that names a `memory_store_id`: the lead and each spawned worker) on every agent, and again for each agent added later. The blueprint is never read again (`src/tui/src/boot/seed.rs`, module doc, and `seed_dir_with`: a folder that has its `.fantastic` is left alone). A fixed agent deleted from disk stays deleted and a field changed on disk stays changed, exactly as for `exec`, `files` or `memory`; the team keeps no `ensure`, no boot-time `create_agent`/`update_agent` and no boot-time grant.
- **The run's flags.** `--port`/`--host` are the run's, not the blueprint's: `webui` puts them on the seeded `web` host (`app_control::set_run_flags`), as fantastic's `Host::boot` does for its own `canvas` (`kern/host.rs`, `set_web_field`, `set_web_bind`).
- **The gap.** A parent entry cannot carry fields (`seed.rs`, `write_blueprint`), and `reach::grant_all` names only the human, the MCP leg and the AI agents. So the other end of four edges the team's non-AI agents need on fantastic's stock agents cannot be written from yaml: `memory.callable_by` for `roles` and `tasks`, `credentials.callable_by` for `github`, and the root's `callable_by` for `board` (`reflect`). The team's end of each (`can_call`) is in the blueprint. The same holds for the page leg's end on `user` (`inquiries`, `answer_inquiry`) and for the agents the person makes later (`dispatcher`, `cadencer`, `pack`, `pack_listing`: their own records carry `callable_by: {team_ws: [...]}`).
