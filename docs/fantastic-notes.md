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

`Cargo.lock` now pins fantastic `b175660`. It brings `fantastic_tui::seed_dir(dir) -> Result<bool, String>` (fantastic#444: the shell's first-run seeding, a no-op once `.fantastic` exists) and spawn applying the card's tool grants on both ends (fantastic#436: a card may only grant what its spawner holds, so the lead holds `board` and the root's await verbs). Team calls `seed_dir` before it boots a fresh directory, so the first `--port` start serves the page.

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

## Pin at `8eaa08a`

`Cargo.lock` now pins fantastic `8eaa08a` (#453: declared TUI views, `view.tui` on a record). Team declares `view.tui` for its agent kinds in `declared_views.rs` (#207); fantastic's panel parser is private, so team validates the shape itself.

## Pin at `f27b3ba`

`Cargo.lock` now pins fantastic `f27b3ba` (#451/#452: agentic_resources and task_registry refuse in the kernel shape: `error` = the sentence, `reason` = the code such as `limit` or `not_allowed`, plus `hint`). Team change: the lead and project_lead cards, the README and the tests read the code from `reason`.

## Pin at `22f88cb`

`Cargo.lock` now pins fantastic `22f88cb`. It brings the route-clash refusal (fantastic#446: web boot fails and `mount` errors when two children declare the same literal route), `?canvas=<id>` in the canvas page, and ai_core `embed`/`unembed` (history rows `{kind:"embed"|"unembed", head:<agent id>}`, no role). Team's page, ws and canvas routes are distinct, so none clashes.

## Pin at `5e74eb5`

`Cargo.lock` now pins fantastic `5e74eb5` (#467/#468: reach writes a new agent's `*` grants before `create_agent` returns; #469/#470: the CLI host registers `ws_bridge` and `relay_connector`). The roles registry `agentic_resources` and the task registry `task_registry` moved from fantastic into this repo (#245): `crates/agentic_resources` (package `team-agentic-resources`) and `crates/task_registry` (package `team-task-registry`), moved from fantastic `5e74eb5` with the same handler names, verbs, fields and refusals. Two changes: the registry ships no built-in `project_lead` card (team's `roles/project_lead.md` is the only one), and `spawn` (plain and `from`) MERGES the parent's `pause`/`resume`/`stop` into what the parent already holds on the child instead of overwriting it, so under reach the spawner keeps `*` (`send`, `history`) on what it spawned. No fantastic bundle crate for either registry is left in `Cargo.lock`; fantastic deletes its copies after this (fantastic#471).

## Pin at `a9b7a31`

`Cargo.lock` now pins fantastic `a9b7a31` (main after #476/#479, the two-way kernel bridge over WebSocket). `ws_bridge` says `hello {guid, leg}` at every boot (`reconnect` included); `web_ws` registers that connection as the dialling kernel's leg, so the far kernel reaches the dialler's agents as `<guid>|<agent>`. Those calls are served AS THE LEG (its `ingress_rule`, sealed by default, its `can_call`, the target's `callable_by`); a far `watch`/`unwatch` of `<guid>|<agent>` streams the agent's events back as `event` frames. On a drop, calls in flight settle at once with `bridge_down`, the far side keeps its watches per guid and re-sends them on the next `hello`, and the newest `hello` for a guid replaces the older connection. It also brings fantastic#474 (fantastic's own `fantastic-cabinet` deleted: it leaves `Cargo.lock`) and #471 (fantastic's registry copies deleted). The build and tests pass with no Team code change.

What the ssh twins measured at `a9b7a31` (#252: the backends and the key are on the master, a remote agent sends and watches them over the one bridge; `e2e/helpers/ssh_twins.py`):

- **A remote agent that wants a model does not need one.** The box's exec agent (`can_call` `<master guid>|*`: send, watch, status, stop) runs a program with fantastic's connector: `kernel.watch("<guid>|<backend>")`, then `kernel.send(...)` with its own `client_id`. The master serves both AS THE LEG (`ws_bridge` `ingress_rule` open, its `can_call` and the backend's `callable_by` name exactly `send`, `watch`, `status`, `stop`), the backend's turn runs on the master, and its events come back as `event` frames in the order the backend emitted them: `status` (`thinking`, `streaming`, `done`), `token`, `part_*`.
- **Which parts a watcher gets.** `PartPublisher::send` (`fantastic-ai-core/src/events.rs:132`): a text part goes to the backend's own inbox for any client but `cli`; a call part goes to the agents it calls. A turn of calls alone shows its watcher `status` and `token` events and no `part_*`: a backend that is watched opens its turn with words. A text-only turn is no end of a run (the backend asks the model again until a `done` call), so a scripted turn ends with `done`.
- **A watch mirrors the inbox.** The watcher also gets every payload sent to the backend, the prompt text of each `send` among them, and other callers' calls (`reflect`). `watch` is a read of the whole inbox: grant it as such.
- **A refused watch is silent.** The master answers `{type:"error", id, src, ...}` (`fantastic-bridge/src/lib.rs:1512` `refuse_watch`); the remote's `ws_peer` `answer` (`fantastic-web/src/ws_peer.rs:491`) finds no call waiting on that id and drops it. With the watch grant sealed the watcher is deaf and is told nothing; the twin's check sees it as no event.
- **No redial by itself.** A ws `ws_bridge` after a drop stays down (`readme_ws.md`, `lib.rs:141`: `reconnect` is "explicit"); the operator (or a lead) sends `reconnect` (teardown, then `boot`, which says `hello` again). Calls in flight settle `bridge_down` at once on the side that was waiting; the remote kernel keeps its watches per guid and asks for them again on the next `hello` (`ws_peer.rs:444-446`), so a watcher made before the break streams the next turn with no new `watch`.
- **An in-flight call dies with the drop, and its backend stays `thinking`.** On a drop the read loop aborts every inbound dispatch (`fantastic-bridge/src/lib.rs:1759` `h.abort()`): the backend's `send` future is dropped in mid-turn, nothing closes the turn out (`on_cancel` is the kernel's cancel, not an abort), `status` keeps `generating: true`, `phase: "thinking"` for ever, and a later `send` to it is held behind that turn. The reconnect twin keeps the call that is in flight to a backend of its own (`w3`) for this reason.

## Pin at `a6975ea`

`Cargo.lock` now pins fantastic `a6975ea` (main after fantastic#480, `fantastic_tui::seed_dir_with(dir, &[blueprint])`: a host writes its own agents after the stock tree at the first run, ids as written, by the same writer: no `parent_id`, `@root` resolved, disk position is the parent; an entry that names an earlier blueprint's id is a parent entry that carries only `id` and `agents`). Team uses it for `crates/team/blueprint.yaml` (#261), and does the same as fantastic does for its own stock agents, nothing more:

- **First run.** `hydrate::seed` (called by the launcher, `main.rs`, when `starts_kernel` is true) writes the fixed agents with their grants and config from the yaml. `@work` and the worker's exec record are filled in first, because they are absolute paths of the folder; `allowed` is the yaml entry's own value.
- **Every run after.** The fs loader (`kernel_state`) is the root loader and rehydrates `.fantastic/agents/**`; fantastic's own boot (`Host::boot`, `kern/host.rs`) runs `reach::grant_all` (`kern/reach.rs`), which writes both ends of the edges of the human and of every AI agent (a record that names a `memory_store_id`: the lead and each spawned worker) on every agent, and again for each agent added later. The blueprint is never read again (`src/tui/src/boot/seed.rs`, module doc, and `seed_dir_with`: a folder that has its `.fantastic` is left alone). A fixed agent deleted from disk stays deleted and a field changed on disk stays changed, exactly as for `exec`, `files` or `memory`; the team keeps no `ensure`, no boot-time `create_agent`/`update_agent` and no boot-time grant.
- **The run's flags.** `--port`/`--host` are the run's, not the blueprint's: fantastic's boot sets them (`--headless --port`), as fantastic's `Host::boot` does for its own `canvas` (`kern/host.rs`, `set_web_field`, `set_web_bind`).
- **The gap** (closed at `4954bb0`, below): a parent entry could not carry fields, so the other end of the team's edges on the stock agents could not be written from yaml.

## Pin at `4954bb0`

`Cargo.lock` now pins fantastic `4954bb0` (main after fantastic#500/#504: a parent entry in `seed_dir_with`'s blueprint may carry `callable_by` / `can_call`; #491/#492/#493 bridge fixes). The grants are UNIONED per key into the stock record (a stock grant is never removed or narrowed, `@root` resolved); an entry for the root (`@root`) and for the credential store, which have no record at seed time, is kept in `.fantastic/cli/grants.yaml` and applied when the root runs (`Host::boot`, `boot/declared.rs`, `apply_to_root`) and when fantastic creates the store (`kern/keys.rs`, `ensure_store`, once a key mode is chosen). The update path never writes `callable_by`, so they survive later boots. Team's `crates/team/blueprint.yaml` ends with the five callee-side grants, written as parent entries with exactly the verbs the caller's `can_call` declares:

- `memory.callable_by`: `roles` `*`, `tasks` `*`;
- `credentials.callable_by`: `github` `*` (declared; fantastic applies it when it creates the store);
- the root's `callable_by` (`@root`): `board` `[reflect]`;
- `user.callable_by`: `team_ws` `[inquiries, answer_inquiry]` (on the existing `user` entry, which also carries `agents:`).

The agents the person makes later (`dispatcher`, `cadencer`) still carry their own `callable_by: {team_ws: [...]}` on their records. Team chooses no key mode and makes no helper agent (`team_app::choose_key_mode` and `new_team::choose_key_mode` are gone): the key mode is the person's, through fantastic's set-up page. Without a mode there is no credential store, and the declared `github` grant waits in `.fantastic/cli/grants.yaml` until fantastic makes it.

## Pin at `8eca583`

`Cargo.lock` now pins fantastic `8eca583`. Since `a9b7a31` it brings:
- **Meta bundles** (#513, #514). A `bundles/<name>/bundle.yaml` with a `base` and no program is the yaml over a compiled bundle, in RAM. An instance inherits its `record` (the nearest bundle wins) and overrides any field. The same inheritance runs at `create_agent`, at load and on `reload_bundle`, from one place (`fantastic-kernel/src/inherit.rs`). `bundle: X` beside another base keeps that handler. Team's roles are these bundles (`crates/team/bundles/`), and the registry reads them from the kernel.
- **Redial:** `ws_bridge` redials a dropped link by itself, with a backoff, and keeps its watches (#493). `web_ws` emits `socket_open` / `socket_closed` on its leg.
- **Connector error:** the exec bridge connector keeps its setup error, so a job that can't reach its kernel says why.
- **Turns:** a drop mid-turn ends the called agent's turn through cancel (#491). A refused remote watch reaches the watcher as a named refusal (#492). The default `max_tokens` is bounded (#495, #496).
- **Also:** `seed_dir_with` grants on stock agents, `reflect` carries every agent's view (#483), theming is removed (#482), and the kiosk and system agents are out of the base (#475, #486).

## Pin at `e1dc8c2`

`Cargo.lock` now pins fantastic `e1dc8c2`.

- **The AI chat apart from its provider** (#516, #521). `ai_chat.tools` is the AI agent: it holds the conversation and completes each turn through the provider agent its record names (`provider_id`), on its own `model` (absent: the provider's default). Every `*_backend` is a provider and nothing else: `complete`, `models` (`{models, default}`), `set_default_model`; a chat verb on it is refused.
- **Embedded meta bundles** (#520). `BundleRegistry::register_meta_yaml(name, yaml)`: the team registers its stock roles from code, in RAM, on `base: ai_chat.tools`; a team folder's `bundle.yaml` of the same name overrides one.
- **The model of a pass** (#522): the chat's `model`, else the provider's `default_model`, else the server's own (written as the default). A provider keeps its state (its default model, as a `setting` row) in its own memory store (#523). The team's compiled blueprint names no provider: the user adds one after install and sets the lead's `provider_id`.

## Pin at `5449402`

`Cargo.lock` now pins fantastic `5449402`.

- **The routing form** (#527, #529). A `send` may carry `<routing>` (how the call travels: `target_id`, `to`, `reply_to`, `title`, `timeout`, …) and `<payload>` (the target's message, delivered whole, nothing inside it read or reserved): `<call name="send"><routing><target_id>github</target_id></routing><payload><type>pr_create</type><title>…</title></payload></call>`. Inline calls still parse. The system prompt teaches the routing form; the stock roles' examples and the spawn note use it, and so does the scripted e2e model.

## Pin at `eb37c8d`

`Cargo.lock` now pins fantastic `eb37c8d`.

- **Every model-facing call example writes the routing form** (#530): the system prompt's `<bytes>` and `docs` examples, the recipients note, the refusal hints and the call checker's "write it as" forms.

## Pin at `fd9351a`

`Cargo.lock` now pins fantastic `fd9351a`.

- **The call checker reports any tag outside a call** (#525), an empty one included: `[unsupported tag — not read]` for `<invoke name="x"></invoke>`, `<tool_call></tool_call>`, `<b>`; code spans and fences are code, not tags.

## Pin at `4a517d3`

`Cargo.lock` now pins fantastic `4a517d3`.

- **A turn's default output budget is 260000 tokens** (#531), never above the model's own maximum: a record with no `max_tokens` asks for the model's limit, not 16384.

## Pin at `5d91e43`

`Cargo.lock` now pins fantastic `5d91e43`.

- **The per-send ceiling is 600 s** (#538, was 180): a model turn and every dispatched call get 10 minutes before they time out. Also in this pin: #534 (TUI composer Backspace).

## Pin at `4bb0969`

`Cargo.lock` now pins fantastic `4bb0969`. Since `5d91e43` it brings:
- A failed turn is retried as the chain's own next turn (aisixteen/fantastic#540, for #533).
- The anthropic provider counts usage per model, with a `usage_reset` test (aisixteen/fantastic#537, for #535).
- Python `ai_core` and the docs teach the routing form (aisixteen/fantastic#539, for #528); the Rust change is `transports.md` text only.

## Pin at `dc1907e`

`Cargo.lock` now pins fantastic `dc1907e`.

- **Nested quoted calls are text** (#543): a call quoted inside another call is not dispatched.
- **Await and docs-note wording** (#544, #545).
- **e2e for the failed-turn retry** (#533).
- **Prices are one `prices` map on the provider** (#535): `{<model>: {input, output, cache_read?, cache_write_5m?}}` in $ per million tokens replaces the four flat `price_*_per_mtok` fields, which are removed. A model with no entry is `unpriced`, never $0; `usage` `by_model` carries `cost_usd` per model and in total.
- **`usage` has `by_agent`** (#546).

## Pin at `255bef1`

`Cargo.lock` now pins fantastic `255bef1`.

- Base has one boot and one stop: `boot_kernel`, `stop_kernel` and `starts_kernel`. `main_with` takes a third argument, `readme_sections`.
- `--headless --port N --host ADDR` is accepted. A recorded `web` boots in every mode and records a free port when it has none. A taken port is only a warning.
- The stop is bounded: 15 s grace, then exit 1.
- The launcher seeds the folder before `main_with`; the kernel never seeds.
- `BootstrapOptions::one_shot(state_dir, work_dir)` has two folders.
- The `status` reply of an AI agent has `inquiry_from`: the cadencer holds `send` and `status` on its digest leads and reads an open inquiry from it.
- Base's boot writes the way back, so `team_ws` and every agent may `send` to `user` and an AI agent holds `*`. This is base's behaviour and the team keeps it; the blueprint is unchanged.
- The team calls base's `error_text`, `Kernel::exists`, `grants::union` and `USAGE_KEYS` in place of its own copies.

## Pin at `c8c6ad7`

`Cargo.lock` now pins fantastic `c8c6ad7`.

- `reflect` takes `handler_modules`, a list of handler names: `tree` keeps only the agents whose stored handler, or the base their role resolves to, is listed (#605). The team does not send it yet.
- Base no longer has `fantastic_ai_core::is_ai`, `is_ai_handler` or `ai_check`; `Host::ai_agents` is async; `port_kind` has a new signature. The team called none of them.
- An agent of a role counts by its role's base for base's reach and rows (#607), so the team's lead and workers, which are agents of roles, hold base's reach as before.

## Pin at `cd28b38`

`Cargo.lock` now pins fantastic `cd28b38` (#642). Every team bundle moves to the base's one entry.

- A bundle implements `Handler`: `verbs!` declares each verb as a struct, `record!` the fields of its record, `handle` answers a typed `Call`. The kernel runs the generated entry first, so a call with a missing, unknown or wrongly typed argument is refused as the agent's own reply (`missing_arg`, `bad_arg`, `unhandled`) and the handler never sees a payload. There is no coercion, no readme, no `verb_docs` and no `record_schema`: `docs` and `reflect` render the declarations, and a bundle has one short man text.
- A record is checked by its bundle at `create_agent` and `update_agent`: a field the record does not declare is refused. The bundles registry writes nothing onto the agents it spawns (team #329): no role, no limits; the bundle is `handler_module` and the place in the tree is `parent_id`.
- `boot` and `shutdown` are verbs only of the bundles that handle them; the kernel sends them only to an agent that declares them.
- `create_agent` takes `handler_module` only (a bare name or a role bundle); the `bundle` alias is gone, and `lifecycle::create_agent` takes the typed `CreateAgent`.
- The team page declares `read_stream` on its record (`verbs`) as `read_stream_declared()`; the blueprint names it with the placeholder `@read_stream`.
- `boot_kernel` and the seeding take the state folder (`<folder>/.fantastic`) and the work folder apart; there is no `readme` word (a bare run with no terminal prints the readme).
- Dev-dependencies that use base's fixtures turn on `test-support` (`fantastic-kernel`, `fantastic-ram-store`, `fantastic-credential-store`); a stand-in bundle declares its verbs with `stand_in_verbs!`.

## Pin at `59c7017`

`Cargo.lock` now pins fantastic `59c7017` (#655, #660, #658).

- A `reflect` reply always stays whole in the chat's view; only `docs` replies fold after the next message.
- The routing field that names an execution is `id` (was `exec_id`).
- The schema engine checks the fields inside array items and objects and names the path in the refusal (`versions[0].branch`); a required argument may be `null` only where its type says so (fantastic #656, #657).

## Pin at `5df7ce5`

`Cargo.lock` now pins fantastic `5df7ce5` (#663): an AI chat's wake watchdog ends as soon as its held sends are answered, instead of sleeping the full send ceiling.

## Pin at `beb8a3e`

`Cargo.lock` now pins fantastic `beb8a3e` (#664): `user.forward` reads its inner payload by the target verb's schema, so a call written as text on a peer (a `wait_s` of `30` inside a bridge `forward`) reaches the remote agent as the number it declares.

## Pin at `797e431`

`Cargo.lock` now pins fantastic `797e431` (#666): a chat with no `caching` runs `provider_cache` where its backend places cache marks, so the team's chats send cache marks by default.

## Pin at `1bf7671`

`Cargo.lock` now pins fantastic `1bf7671` (#668): an argument's label names its maximum (`limit:int >= 1 <= 100`), so an over-cap value is refused with the cap shown.

## Pin at `d372d5b`

`Cargo.lock` now pins fantastic `d372d5b` (#669): `caching` has two modes, `none` and `provider_cache` (the default everywhere); `cleanup` and its view trimmers are gone.
