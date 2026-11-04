# e2e for the team binary

Built the way fantastic's `e2e/` is: one folder per test, a plug picks what answers the model, a `costs:` tier on every test.

```
python3 e2e/run.py --only-free          every free test; no network, no key; builds the binary with cargo
python3 e2e/run.py --only task/one_issue_to_draft_pr --bin PATH   (PATH absolute)
python3 e2e/run.py --only-turns         the turns tests, a real model, by hand; SKIP without a key
python3 e2e/run.py --only-turns --dry-run   discover the turns tests and collect their probes; starts nothing
python3 e2e/run.py --build github --bin PATH   the tests for the GitHub build (default: general)
python3 e2e/run.py --selftest           the harness proves itself, no binary
python3 e2e/run.py --only app/attach_remote --bin /ABSOLUTE/path/fantastic_team   a test that needs the app: build it next to the binary first
python3 e2e/run.py --list
```

Needs `python3`, `git` (with `git http-backend`) and `openssl` (a throwaway app key is made per run, never committed; without openssl the launcher says so and runs nothing). Exit code is non-zero when any test fails; a test whose plug has no key is printed as `SKIP` and does not fail the run. A workflow (#46) calls `--only-free`; there is no workflow here, CI is manual, and CI never runs the turns tier.

## The turns tier, by hand

A turns test sends a real model's calls through the `anthropic` plug (`backends/anthropic.py`). It needs a key in the environment, and the launcher never writes it to disk:

```
ANTHROPIC_API_KEY=... python3 e2e/run.py --only-turns
```

- `ANTHROPIC_API_KEY` or `E2E_ANTHROPIC_KEY`: the key (either name; without one the turns tests are skipped).
- `E2E_MODEL`: the model's name (default: `claude-haiku-5-5`, set in `backends/anthropic.py`).
- `E2E_MAX_REQUESTS`: a hard cap on model requests per run (default 60); past it the relay answers 429, so a loop cannot run up the bill.
- `E2E_PRICE_IN_PER_MTOK` and `E2E_PRICE_OUT_PER_MTOK`: USD per million input and output tokens. Set both to get a USD estimate in `cost.json`; without them the estimate is `null`.
- `E2E_UPSTREAM`: the upstream base URL (default `https://api.anthropic.com`).

Each run writes `cost.json` in its run folder (tokens, requests, model, the USD estimate if priced). It holds no key. The relay forwards the team's own `x-api-key` and streams the reply back unchanged; it never reads or stores the key value.

## The cli group (#219)

`tests/cli/*` are the tests for the command line being fantastic's and the config being fantastic's records. They hold no `team.yaml`: each folder has `test.yaml`, `README.md`, `script.json`, `probe.py` and a `blueprint/` folder (a fantastic blueprint: `blueprint.yaml` with `ports` and `agents`). All six are free (scripted model, no key, no network) and need no forge. Run them with `python3 e2e/run.py --only cli/<id> --bin ABS_PATH`.

The probe starts the binary the way fantastic is started, in the run folder (fantastic has no `--dir`; it works in the directory it is started in): `--headless`, or `canvas --port N`, or the recipe `--headless ai anthropic --model M --endpoint URL`. It makes the credential store (`backend: env`, the model's key in the team's environment only) and an exec agent in the fixture repo with `send` (`create_agent`), then imports the blueprint under `user` (`fantastic_team import blueprint --parent user --bind ... --approve`) and hands the lead its task with `send`. `helpers/cli.py` has the steps; `Team.import_blueprint` and `Team.agents` are in `helpers/team.py`. The role cards are not in a blueprint (an import counts a card as code): they sit in the test's `roles/` folder, which the probe copies next to the kernel, where `role_dirs: [roles]` reads them. `FANTASTIC_AGENTIC_RESOURCES=1` turns the registry on for the tests that spawn.

The shared fixture is a local git repo with one failing test (`test_calc.py` checks `adder.add` and `muler.mul`, both wrong). Solved means the test fails on the start commit, passes on HEAD, and the work tree is clean: the fix is a commit. Two bugs in two files, so two workers can each fix one.

| test | start | does | checks |
| --- | --- | --- | --- |
| `single_lead` | a lead, once from a blueprint, once from the recipe + `send` (no file) | solves | solved; the records are the ones it started with |
| `lead_spawns_two` | a lead with the registry and the `worker` card | spawns 2 workers, they solve, the lead deletes them | solved; 2 spawns; the records are the ones it started with |
| `precreated_1_2` | a lead and 2 workers in the blueprint | solve | solved; the same 3 under the same parents, nothing extra |
| `precreated_tree` | a lead, 2 sub-leads, a worker under each, in the blueprint | solve | solved; the tree unchanged |
| `leads_spawn_leads` | a lead with the `sublead` and `worker` cards | spawns 2 sub-leads, they spawn workers, solve, each parent deletes what it spawned | solved; 4 spawns; no spawned agent left, no orphans |
| `headless_and_canvas` | `--headless`, then `canvas --port N` | an agent makes a canvas with `create_agent` | headless opens no port and needs no terminal; `canvas` serves; the canvas made with no `canvas` word |

Facts found while writing them (about fantastic at this commit, not decisions of the suite):

1. Fantastic renames every imported id with `_1` (`lead_1`, `roles_1`, `w1_1`), so a script names those. Spawned ids are minted (`anthropic_backend_7a308a`).
2. A spawned agent is not named on the credential store's `callable_by`, so a tier with `credentials` is refused its first turn and the agent never asks the model. The spawned tiers here carry no `credentials` (the record's own rule: no `credentials` is a keyless server, which the scripted plug is).
3. The registry's `spawn` sends the task detached, so the lead's turn goes on at once; the plug's lanes (below) keep the order.
4. `delete_agent` of a parent takes its children with it; a spawned agent carries `spawned_by`, which is how the tests see that none is left.

### The scripted plug with several agents: lanes

One plug serves every agent, and their requests race. A script turn may carry `"lane": "w1"`: it goes to the agent whose first message holds `[lane:w1]` (a spawn's `task`, or the `text` of a `send`); turns with no lane are lane `main`. Each lane plays its own turns in order. A turn may carry `"after": ["w1", "w2"]`: it is held until those lanes have been served all their turns, so one agent works after another (two commits never meet on the git lock) and a lead deletes workers only after they are done. `{{id:N}}` in a turn is the id in the Nth `<id>...</id>` of the request body (the reply of a `spawn`, in the order the agent saw it). A script with no lane anywhere is the old single ordered script. The selftest covers all three.

## The stock team of the older tests (`helpers/stock.py`)

The older groups (`task`, `secrets`, `exec`, `dispatcher`, `roles`, `packs`, `surfaces`, `ui`, `app`, `turns`) once held a `team.yaml`; the binary has no config file now, so each test builds its team the way a person does (#219):

- the AI agents are a BLUEPRINT FOLDER in the test (`blueprint/blueprint.yaml`: the lead and its ports), imported under `user` with `fantastic_team import` (`Team.import_blueprint`). Fantastic renames every imported id with `_1`, so the lead is `lead_1` (`ctx.lead`). A port the blueprint declares (`@memory @credentials @exec @files @queue @git @github`) is bound to the agent of that name (`stock.PORTS`);
- what a blueprint cannot hold (an absolute path, a key, an id the product's code and the role cards name) is made with `fantastic_team send`: the credential store, `worker_exec` and `worker_files`, `queue`, the `git` and `github` connectors against the fake forge, the queue folder and the puller; the `pack` and `pack_listing` agents; the roles registry (fantastic's `agentic_resources`, with its tiers and limits) and the `tasks` registry; the board, the dispatcher with its `worker_view`, the cadencer. Every edge is written on both ends (`stock.link`);
- the team page the older UI tests drive is the app's (`app_control::prepare_web`): `stock.serve_team_page(team, port)` makes the `web` host, `team_page` and `team_ws` and sends `boot`. Start such a team `--headless`: with fantastic's `canvas` word there is a second socket leg (`canvas_ws`) on the same `/{host_id}/ws` route, and it wins;
- a probe starts it with `run_flow.boot(ctx, ...)` / `stock.boot(ctx, ...)`: `forge=False` (no connectors), `git=False, puller=False`, `packs=True`, `board=True`, `groups={name: {driver: "$lead", members: [...]}}` (the dispatcher and cadencer), `registry_tiers={...}` (a roles registry the dispatcher makes workers from), `roles_tiers={...}` plus `limits` (roles mode: the cards), `sub="a"` (a second team in a sub folder), `port=N` (`canvas --port N`).

The GitHub build's tests (`build: github`) get the worker exec the patch gives (`stock.github_allowed()` reads `patches/github-exec.patch`); run them with a binary built from the patched tree.

## Tiers

| costs | what answers | status |
| --- | --- | --- |
| `free` | a scripted model (`backends/scripted.py`) and a fake forge (`helpers/forge.py`) | built |
| `turns` | a real model through the `anthropic` plug (aisixteen/fantastic#302), run by hand, needs `ANTHROPIC_API_KEY` or `E2E_ANTHROPIC_KEY` | built |
| `gpu` | a local-model plug on the GPU box (aisixteen/fantastic#303) | not built |

## Layout

- `run.py`: the launcher.
- `backends/`: plugs. A test's `test.yaml` names one (`plug: scripted` or `plug: anthropic`); a test never names a model, an endpoint or a key. A blueprint takes `{{model_name}}` (the plug's model), `{{model_endpoint}}` and `{{model_host}}`.
- `helpers/`: `cli.py` (the cli group: the fixture repo, fantastic's start, blueprint import, the shared script turns), `forge.py` (fake GitHub API plus git over smart HTTP from a bare repo), `team.py` (start with fantastic's words such as `canvas --port N`, stop, `send`, `import_blueprint`, wiring, the page's chat grant), `run_flow.py` (fake keys, boot, hand the lead its message), `verdict.py` (named checks), `packs.py` (the packs tests: a plain team, a source worker, `pack` and `install` through `send`), `packs_evals.py` (the eval and verdict tests: build and pack a worker, drive `fantastic_team packs ...`, the pack agents' grants), `panel_dom.js` (the packs panel on a stub DOM against the live team; needs `node`), `packs_pub.py` (the pack publishing tests, #42: write pack folders through the `files` agent, pack, publish, trust a signer, the content hash), `registry.py` (`FakeRegistry`, an https server with a throwaway self-signed certificate).
- `helpers/web.py`, `helpers/app.py`: HTTP and websocket client; the app shell and handle driver as processes.
- `tests/<group>/<id>/`: `README.md`, `test.yaml` (`costs`, `timeout`, `plug`, `build`: `any` (default), `general` or `github`; `run.py --build` runs the tests for `any` and that build), `blueprint/blueprint.yaml` (the lead; see `helpers/stock.py`), `script.json` (the scripted plug's turns; none for a turns test), `probe.py` (`run(ctx)` returns the exit code).
- `ui/sidebar_opens_agent_full_screen` (free): drives the served team page with `helpers/page_driver.js` (the page's own script on a stub DOM, over the live `/team_ws/ws`); needs node >= 22, else it prints `SKIP` and passes.
- `tests/ui/`: the web UI (the team page on a web host, its websocket via `helpers/web.py`). The browser overflow check is not built.
- `runs/`: one folder per run, gitignored (it holds model output).

## The app and the dispatcher's web UI (#37)

The `app/` and `dispatcher/` tests drive the web UI of a live team: they start the binary with `canvas --port` (`stock.boot(ctx, port=P)`), or serve the team page (`stock.serve_team_page`), talk HTTP and a websocket with `helpers/web.py` (stdlib only), and run the app shell `fantastic_team_app` and the in-process handle driver `crates/team/examples/handle_serve.rs` as processes (`helpers/app.py`). `run.py` builds all three; with `--bin`, build them yourself and keep them in the same folder:

```
cargo build -p fantastic-team --bin fantastic_team --example handle_serve
cargo build -p fantastic-team-app --bin fantastic_team_app
python3 e2e/run.py --only dispatcher/chat_features --bin "$PWD/target/debug/fantastic_team"   # --bin must be absolute
```

A probe that cannot run here raises `helpers.verdict.Skip(reason)`; the launcher prints `SKIP <name> (<reason>)` and the run still exits 0. A test whose binary (`fantastic_team_app`, `examples/handle_serve`) is not next to `--bin` skips the same way. `app/window_per_platform` also skips without a tauri build (`E2E_APP_TAURI_BIN`), a Linux display and `xdotool`.

Facts about the web UI at this commit, found while writing these tests:

1. `fantastic_team canvas --port P` serves the canvas page `/canvas_page/file/index.html` (the web host answers 404 at `/`) and its socket leg `canvas_ws`; the binary no longer serves the team page or `/team_ws/ws` by itself (the app does, in-process: `TeamHandle`, `examples/handle_serve`). The chat tests use `canvas_ws`, to which the harness writes the page's chat grant (`helpers/team.py grant_chat`); the tests that need the team page build it with `stock.serve_team_page`.
2. The launcher (`fantastic_team_app`) starts `canvas --port N` in the team folder and waits for a 200 at `/canvas_page/file/index.html` (`READY_PATH`), the page that command serves. In a folder with no `blueprint/` it writes the app's preset there and imports it once the page answers (it needs the home's `credentials` agent); a folder that has a `blueprint/` is started as it is, so `app/launcher_start_attach_stop` brings its own. `app/attach_remote` keeps a small proxy front for its tunnel leg.
3. The Rust ai-core never emits the `tool_calling` status the page's tool blocks key on; a tool call is a `call` part sent to the watchers of its addressee.
4. The scripted plug takes `"thinking": "..."` (a reasoning block before the text) and `"delay": seconds` (the plug waits before it answers) on a turn.

## The packs group

`tests/packs/*` (issue #42) drive the `pack` and `pack_listing` agents of a team that has no connector: `stock.boot(ctx, forge=False, packs=True)` makes it (the pack agents, the `files` peer they read through, the `user` verb they build with, and the `send` leg's grant on `files`), and a probe writes pack folders through `files`. All are free (scripted model, no key, no network):

- `packs/pack_publish_install_update`: pack, publish (signed), list (untrusted signer skipped), install, an opt-in update with its permission diff, takedown and its warning.
- `packs/install_from_hosted_registry`: `fetch` pins by name and hash and refuses before any connection; a live entry reaches the (distrusted) fake registry and stops there.
- `packs/listing_page`: the static, escaped html page.
- `packs/web_browser`: the panel as the team persists it, and the verbs it sends (list, show, install preview then approve, eval, the browser view-model with and without opt-in). No browser is started: rendering is not exercised.
- `packs/public_publish_review`: a scan finding refuses a publish and holds a submit, nothing is sent.
- `packs/report_and_takedown`: report refusals, a takedown whose registry push fails, the warning on the installed pack.

**The hosted-registry limit.** `registry_url` and every entry url must be https, and the binary trusts only the public web roots (rustls with webpki roots, no way to add one). A `FakeRegistry` therefore cannot be trusted: the tests assert exactly that (a connection, a failed handshake, no HTTP request) and everything up to the network. The success paths (a download that matches its pin and is staged, a `submit` that lists, a `report` or a takedown push that the registry accepts) stay unit items in `crates/team/src/pack_listing.rs` until the binary can trust a test certificate; that would be new API and is a decision for the maintainer.

## Two builds

The general build (stock exec) and the GitHub build (`patches/github-exec.patch`, built by `scripts/build-github.sh`) share the `any` tests. `exec/general_allows_gh` runs on the general binary only; the four `dispatcher/*` tests (`digest_reaches_driver`, `quiet_heartbeat`, `stop_asks_then_stops`, `hand_task_add_worker`) are free and run on either build; `exec/github_refuses_forge_calls`, `secrets/github_no_credential_in_worker` and `task/github_pr_through_connector` on the GitHub binary only.

## How a free test runs

The launcher makes a bare repo and a seeded clone, starts the fake forge (the repo is `acme/widgets`, one issue labelled `team-ready` by the maintainer with a `## SPEC` comment), starts the plug, and starts `fantastic_team --headless` (or `canvas --port N`) in the run folder with fake keys in ITS environment only (the credential store reads them with `backend: env`). The stock records and the test's blueprint are made with `send` and `import` (see above); the probe then drives the team with `fantastic_team send`.

The anthropic backend sends no `tools` array: a model calls a tool by writing a `<call name="send">` element in its text. So the scripted plug streams Anthropic SSE whose text is that element; a script turn lists `calls`, and the plug renders them.

## Things the harness works around (found while building it)

These are facts about the binary at the commit this was built on, not decisions of the suite. Each workaround is in `helpers/team.py` or `helpers/run_flow.py` and goes away when the cause is fixed.

1. `worker_files` and the puller's queue folder are sealed by default (`edge sealed by policy`); `stock.py` makes them with `ingress_rule: allow_all_ingress`, and writes the callers on both ends.
2. The binary never calls `prepare_sandbox`: `stock.make_tools` makes `work/`, `work/wt`, `work/tasks` and `sandbox/home` first. The worker's HOME is the relative `./sandbox/home`.
3. Fantastic renames every imported id (`lead` is `lead_1`), and an import rewrites an id only in `can_call`, `callable_by`, `memory_store_id`, `file_bridge_id`, `backend_id` and `credentials.store`. A string field that names another agent (the board's `github`, the dispatcher's `team`) is therefore not rewritten, so the agents those fields name are made with `send` under the ids the product's code and the role cards use.
4. A spawner grants only what it holds, and the operator is the `mcp` leg: `stock.operator_holds` gives it the grants a card names before a probe spawns a role as the operator.
5. A model cannot pass `title` to `github pr_create`: `title` is a call-level field of the `<call>` grammar. The scripted model therefore stops after the push, and the probe sends `pr_create` itself, as the operator. The PR request is still what the github connector sends.
6. The roles registry and the board are made by `stock.py`, not by a start-up step: nothing in the binary plans a tree now.
7. The team page for the older UI tests is `stock.serve_team_page` (see above).

## The packs tests

`tests/packs/*` drive the pack verbs the way a person does: `fantastic_team packs pack|install|eval ...` (the CLI) for what has a flag, and `Team.send` to the `pack` and `pack_listing` agents for `signing_key`, `from_shifts` and `publish` (which have no flag). All are `free` on the scripted model: each eval task is one `send` to the lead, which the script answers with `ok` then `done` (the lead nudges once when an answer holds no call). `helpers/packs.py` adds the grants the two-step boot leaves out (the base `files` and `user` do not name the pack agents). `packs/shifts_produce_verifiable_record` also points the planned board at `queue_files` (the planned board's store is the `queue`, a ram_store with no files) and has the fake forge list a second issue. `packs/ui_shows_pass_rate_time_cost` runs the real panel page in node (no browser, no display) and prints `SKIP: node not found` and passes when there is no `node`.

## The roles group

`tests/roles/*` (issue #185) drive the roles registry (`agentic_resources`) with the repo's `roles/` cards, which each probe copies into the run dir. Both are free (scripted model, fake forge), `build: any`, and not part of any CI: run them by hand (`python3 e2e/run.py --only roles/total_limit_refuses_fourth_worker --bin /ABS/PATH`). The scripted plug is one ordered script for every agent, so each test keeps one agent's model request open at a time.

- `roles/total_limit_refuses_fourth_worker`: `roles: limits: total: 3`; the lead spawns four workers, one call per turn; the fourth is refused with its reason; `get_count`, `fleet` and the dispatcher's `fleet` agree (3 of 3). The worker tiers are inert, so no worker asks the model.
- `roles/project_lead_plans_and_runs_issues`: the coder `project_lead` reads four fake issues, proposes a tree and waits for "yes"; then two workers are started, a PR is opened for each, and the tree stays. The chat entry, the plan view and the approval gate (LATER, UI) are skipped with a note. The README of the test lists the ordering it relies on and the gaps it works around.

## Later

- `driver/rulebook_reply_and_escalation` waits for the group driver (#95): the driver is a library (`crates/team/src/driver/`) with no agent, verb or `main.rs` wiring yet, so there is nothing for a turns test to drive. It is not written, and it is not a turns test until #95 wires the driver.
- the `gpu` tier (a local-model plug, aisixteen/fantastic#303).
