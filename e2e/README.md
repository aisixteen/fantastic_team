# e2e for the team binary

Built the way fantastic's `e2e/` is: one folder per test, a plug picks what answers the model, a `costs:` tier on every test.

```
python3 e2e/run.py --only-free          every free test; no network, no key; builds the binary with cargo
python3 e2e/run.py --only task/one_issue_to_draft_pr --bin PATH   (PATH absolute)
python3 e2e/run.py --only-turns         the turns tests, a real model, by hand; SKIP without a key
python3 e2e/run.py --only-turns --dry-run   discover the turns tests and collect their probes; starts nothing
python3 e2e/run.py --only-gpu           the gpu tests, a vLLM server (the shared RTX 6000), by hand; SKIP without its variables
python3 e2e/run.py --only-gpu --dry-run discover the gpu tests and collect their probes; starts nothing
BACKEND=anthropic python3 e2e/run.py --only-gpu   the gpu tests on Sonnet/Haiku through the relay instead; real money, by hand
python3 e2e/run.py --selftest           the harness proves itself, no binary
python3 e2e/run.py --only app/attach_remote --bin /ABSOLUTE/path/fantastic_team   a test that needs the app: build it next to the binary first
python3 e2e/run.py --list
```

Needs `python3`, `git` (with `git http-backend`) and `openssl` (a throwaway app key is made per run, never committed; without openssl the launcher says so and runs nothing). Exit code is non-zero when any test fails; a test whose plug has no key is printed as `SKIP` and does not fail the run. A workflow (#46) calls `--only-free`; there is no workflow here, CI is manual, and CI never runs the turns or the gpu tier (`--only-free` selects neither).

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

## The gpu tier, by hand (#237)

The five cli scenarios on a real model, the shared RTX 6000 behind vLLM, with no script: `tests/gpu_cli/{single_lead,lead_spawns_two,precreated_1_2,precreated_tree,leads_spawn_leads}`, and their ssh twins `tests/gpu_cli_ssh/` (#242, below). The model finds the bug, decides to spawn, and cleans up after itself. `costs: gpu` is a tier `run.py` knows: `--only-gpu` runs it, `--only-free` and CI never do. No Anthropic call and no GPU call is made by the selftests or the CI run.

```
VLLM_ENDPOINT=https://.../v1 FANTASTIC_AI_MODEL=... VLLM_KV_SLOTS=2 VLLM_KV_OVERFLOW=evict \
    python3 e2e/run.py --only gpu_cli/single_lead --bin /ABSOLUTE/path/fantastic_team     # one test at a time
python3 e2e/run.py --only-gpu --dry-run      # collects the ten probes, starts nothing, needs no variable
```

The same variables as fantastic's `vllm` plug (aisixteen/fantastic `e2e/backends/__init__.py`, `_vllm`). If a required one is missing the test prints `SKIP <name> (<why>)` and the run still exits 0, as a turns test without its key:

- `VLLM_ENDPOINT`: the server's `/v1` URL. No host is guessed. The AI records point straight at it (no relay).
- `FANTASTIC_AI_MODEL`: the served model's name.
- `VLLM_KV_SLOTS`: how many conversations may be live on the endpoint. **1 or 2.** The server's slots are shared with production, so a larger number is refused (`SKIP`, with the reason); the backend then rotates the least-recently-used conversation out (`evict`) or refuses the newcomer (`reject`) when a third agent talks. The trees of 3 to 5 agents therefore take turns on the 2 slots.
- `VLLM_KV_OVERFLOW`: `evict` or `reject`.
- `VLLM_API_KEY` (optional): the door's key. The credential store reads it from the team's environment (`backend: env`, the ref bound to the endpoint's host); it is never written to disk or printed, and a test checks that no file of the run holds it. Without it the records carry no `credentials` and the server is reached keyless.
- `MAX_TOKENS`: one completion's cap (default 65536, an output reserve, not the window).
- `E2E_MAX_REQUESTS`: a hard cap on model requests per run (default 200). The plug stops the team when it is passed, so the run ends and the test fails instead of looping.
- `E2E_PRICE_IN_PER_MTOK` and `E2E_PRICE_OUT_PER_MTOK`: USD per million tokens, for the estimate in `cost.json`.

**`BACKEND`: the other model server (#251).** The gpu tests are declared on the vllm plug; `BACKEND` picks their plug, as fantastic's own e2e picks its backend: `BACKEND=vllm` (the shared RTX, the default) or `BACKEND=anthropic` (Sonnet or Haiku through the recording relay of the turns tier, `backends/anthropic.py`). A free or turns test never reads it; any other value is refused. On the anthropic plug the AI records are `anthropic_backend` ones on the relay (the recipe is `--headless ai anthropic --model M --endpoint URL`), and they take the turns tier's variables (above) plus:

- `ANTHROPIC_CACHING`: the records' `caching` (`none`, `cleanup` or `provider_cache`; default `provider_cache`, prompt caching on, as fantastic's plug).

```
BACKEND=anthropic E2E_MODEL=claude-sonnet-... ANTHROPIC_API_KEY=... E2E_MAX_REQUESTS=80 \
    python3 e2e/run.py --only gpu_cli/single_lead --bin /ABSOLUTE/path/fantastic_team
BACKEND=anthropic python3 e2e/run.py --only-gpu --dry-run    # the ten on the anthropic plug, starts nothing
```

**This costs real money**: every turn is a paid model call. Run it by hand, one test at a time, and cap it with `E2E_MAX_REQUESTS` (past it the relay answers 429). Its `cost.json` has the relay's own stream counts and an `agents` block: like the vllm plug, the relay asks every `anthropic_backend` agent for its `usage` every two seconds and once before the team stops, and sums requests, input, cache write, cache read, output, prompt and saved tokens, and `cost_usd` (`null`, "unpriced" in the report, unless every record carries both prices); the stream's own count leaves cached input out.

The sampling knobs (`no_repeat_ngram_size`, `repetition_penalty`, the measured fix for the chain-of-thought repetition loop) are `backends/sampling.default.json`'s; a `sampling.json` beside it overrides them. `thinking_token_budget` is left to the backend's own default.

**Cost.** There is no relay to count with, so the plug asks the team: every vllm agent keeps its counts (the `usage` verb: requests, prompt and output tokens from the replies' `usage` frame), the plug sums them every two seconds and once more before the team stops (a deleted worker keeps the counts last seen). `cost.json` in the run folder has the model, requests, tokens and the USD estimate if priced. It holds no key.

Each test, in `tests/gpu_cli/<id>/`, is the same scenario as `tests/cli/<id>/`: `helpers/cli_scenarios.py` builds the team (the same `send` + `create_agent` calls, the same fixed ids, the same role cards, `cli/<id>/roles/`) and runs the same outcome checks; only the model differs. The probe hands the lead the task in plain words (`helpers/gpu_cli.py`, `TASKS`: what is broken, `python3 test_calc.py`, that the fix is a commit, how the work is split, and that whoever spawns an agent deletes it), waits for the lead to answer `done` (its last `history` row holds the `done` call) or for the test's `timeout`, then checks: solved (the test passes on HEAD, fails on the start commit, the tree is clean), the record set, the spawns and deletes, no orphans, the registry counts none running. The checks about the scripted model (every turn served, a lane per agent) have no twin. Which tool the model called first and how many turns it took are not asserted; the log has `report:` lines for them.

Open bug: the spawn scenarios (`lead_spawns_two`, `leads_spawn_leads`) spawn workers whose tier carries the model's `credentials`; until aisixteen/fantastic#467 (the reach race) is in the binary, they are expected to fail their first worker turn. `single_lead`, `precreated_1_2` and `precreated_tree` spawn nothing.

**Trying the gpu tests without a GPU.** `helpers/fake_vllm.py` is a fake vLLM `/v1` server (the routes `vllm_backend` calls: `GET /v1/models`, `POST /v1/chat/completions` as SSE with the usage frame) that can play a cli test's `script.json`:

```
python3 e2e/helpers/fake_vllm.py --script e2e/tests/cli/single_lead/script.json    # prints the url
VLLM_ENDPOINT=<url> FANTASTIC_AI_MODEL=fake-model VLLM_KV_SLOTS=2 VLLM_KV_OVERFLOW=evict \
    python3 e2e/run.py --only gpu_cli/single_lead --bin /ABSOLUTE/path/fantastic_team
```

The selftest (`run.py --selftest`) covers the fake, the plug (what it needs, its records, the usage sum, the cap, `cost.json`, the key), the gpu tier's selection and collection, `BACKEND` (the gpu tests only; `--only-gpu --dry-run` collects the ten with either value), the relay's records, recipe and caching, its usage poll summed into `cost.json` (a fake team, no model), and that its child runs carry no model variable: the selftest sets every one (fakes) and the child's gpu tests must still be on the vllm plug and skip. It passes with or without `BACKEND` or a key in the caller's environment.

## The cli group (#219)

`tests/cli/*` are the tests for the command line being fantastic's and the config being fantastic's records. They hold no `team.yaml`: each folder has `test.yaml`, `README.md`, `script.json` and `probe.py`; no test holds a blueprint, and every agent is made with `send` + `create_agent`. All five are free (scripted model, no key, no network) and need no forge. `cli/checkins_busy_stuck` (#284: a lead's check-ins leave a generating sub-lead alone and stop an idle one) is a sixth, free too, with an ssh twin and no gpu twin. A worker or sub-lead ends its run with a plain `send` to its parent (#247: there is no `report` verb); the send waits until the parent is idle, so the scripted parent awaits and each report is a turn of its own, in which it deletes that agent (#264). Run them with `python3 e2e/run.py --only cli/<id> --bin ABS_PATH`.

The probe starts the binary the way fantastic is started, in the run folder (fantastic has no `--dir`; it works in the directory it is started in): `--headless`, or the recipe `--headless ai anthropic --model M --endpoint URL`. It makes the credential store (`backend: env`, the model's key in the team's environment only) and an exec agent in the fixture repo with `send` (`create_agent`), then makes its agents with `send` (`create_agent` under fixed ids: `lead`, `roles`, `w1`, `s1`...; the caller's `can_call` in the payload, the callee's `callable_by` merged into what it has) and hands the lead its task with `send`. `helpers/cli.py` (`make_agent`, `make_lead`, `make_registry`) and `helpers/cli_scenarios.py` (the five scenarios' steps and outcome checks, shared with the gpu twins) and `helpers/records.py` (`create`, `update`, `merge`, `link`) have the steps; `Team.agents` is in `helpers/team.py`. The roles are meta bundles in the test's `bundles/` folder: the probe writes them under the team's `.fantastic/bundles/` and loads them (`reload_bundle`); a spawned agent runs on its spawner's backend. `FANTASTIC_AGENTIC_RESOURCES=1` turns the registry on for the tests that spawn.

The shared fixture is a local git repo with one failing test (`test_calc.py` checks `adder.add` and `muler.mul`, both wrong). Solved means the test fails on the start commit, passes on HEAD, and the work tree is clean: the fix is a commit. Two bugs in two files, so two workers can each fix one.

| test | start | does | checks |
| --- | --- | --- | --- |
| `single_lead` | a lead, once made with `create_agent`, once from the recipe + `send` | solves | solved; the records are the ones it started with |
| `lead_spawns_two` | a lead with the registry and the `worker` card | spawns 2 workers, they solve, the lead deletes them | solved; 2 spawns; the records are the ones it started with |
| `precreated_1_2` | a lead and 2 workers made with `create_agent` | solve | solved; the same 3 under the same parents, nothing extra |
| `precreated_tree` | a lead, 2 sub-leads, a worker under each, made with `create_agent` | solve | solved; the tree unchanged |
| `leads_spawn_leads` | a lead with the `sublead` and `worker` cards | spawns 2 sub-leads, they spawn workers, solve, each parent deletes what it spawned | solved; 4 spawns; no spawned agent left, no orphans |

Facts found while writing them (about fantastic at this commit, not decisions of the suite):

1. The test chooses the ids (`lead`, `roles`, `w1`...), so a script names those. Spawned ids are minted (`anthropic_backend_7a308a`).
2. A spawned agent is not named on the credential store's `callable_by`, so a tier with `credentials` is refused its first turn and the agent never asks the model. The spawned tiers here carry no `credentials` (the record's own rule: no `credentials` is a keyless server, which the scripted plug is).
3. The registry's `spawn` sends the task detached, so the lead's turn goes on at once; the plug's lanes (below) keep the order.
4. `delete_agent` of a parent takes its children with it; a spawned agent carries `spawned_by`, which is how the tests see that none is left.

### The scripted plug with several agents: lanes

One plug serves every agent, and their requests race. A script turn may carry `"lane": "w1"`: it goes to the agent whose first message holds `[lane:w1]` (a spawn's `task`, or the `text` of a `send`); turns with no lane are lane `main`. Each lane plays its own turns in order. A turn may carry `"after": ["w1", "w2"]`: it is held until those lanes have been served all their turns, so one agent works after another (two commits never meet on the git lock) and a lead deletes workers only after they are done. `{{id:N}}` in a turn is the id in the Nth `<id>...</id>` of the request body (the reply of a `spawn`, in the order the agent saw it). A script with no lane anywhere is the old single ordered script. The selftest covers all three.

## The ssh twins (#242, #252): a remote kernel in Podman, the weak wiring

Each cli scenario runs once more split across two kernels: the local one and one in a Linux container reached over ssh (`tests/cli_ssh/`, scripted, and `tests/gpu_cli_ssh/`, the gpu tier). The remote is driven the **weak** way, with existing pieces only: raw commands through an `exec_bridge` agent, and `send`, `watch` and their events carried both ways over one connection by fantastic's `ws_bridge` (fantastic#476). **The backends and the model's key are on the master only** (#252): the box holds tools (an exec agent, git, the clone) and no AI agent.

**The remote: `helpers/podman_remote.py`.**

- Build, once per source tree: a builder image (`localhost/fantastic-team-ssh-builder:<toolchain>-<recipe hash>`: the channel of `rust-toolchain.toml`, `libdbus-1-dev` for fantastic's system agents, Go, zip and unzip for fantastic's canvas pack) compiles the Linux `fantastic_team` with the source mounted read-only and no debug info (`CARGO_PROFILE_DEV_DEBUG=0`: with it, the link is killed for memory in a 2 GiB podman machine, measured); the cargo target and registry are a bind mount, `E2E_PODMAN_BUILD_DIR`, else `<CARGO_TARGET_DIR or target>/podman-linux` (keep it off a small system disk). The box image (`localhost/fantastic-team-ssh:<source tag>`: Debian, `sshd`, `git`, `python3`, the binary) is tagged by a hash of the source outside `e2e/` (tracked files, uncommitted edits, untracked files), so a second run of the same tree builds nothing.
- Start (`Remote`, a context manager): a throwaway ed25519 key per run in the run folder (`remote-key/`, gitignored, never committed); the public half reaches the container in its environment, not an argv; sshd takes only that key for the login `team` (no root, no password, local forwarding only) on a mapped `127.0.0.1` port. `Remote.argv(command, forward=(local, remote))` is the raw `ssh -p PORT -i KEY [-L local:127.0.0.1:remote] team@127.0.0.1 COMMAND` an exec agent runs; `Remote.tunnel` keeps one open for the harness.
- Gateway: a service of the test on this host's 127.0.0.1 (the fake forge of `helpers/forge.py`) is dialled from the container as `host.containers.internal` (`gateway_url`). Measured on macOS (Podman 6, a podman machine); on Linux the host's loopback through Podman's gateway depends on the network mode and is not measured.
- Teardown: the tunnels are closed and the container removed, pass or fail (`with Remote(...)`).
- No Podman, `ssh`/`ssh-keygen` missing, or no running machine: `podman_remote.require()` raises `Skip` with the reason, as the display-only tests do.
- The tunnel's two ends are one port number (`-L N:127.0.0.1:N`): `web`'s host guard admits a `Host` naming its own port only (a 403 otherwise, measured).
- The binary has no `stop` word: a remote kernel started with `echo $! > team.pid` is stopped with SIGTERM and a bounded wait (`podman_remote.STOP`), the way `helpers/team.py` stops a local one.

`python3 e2e/helpers/podman_remote.py --smoke` builds (or reuses) the image and walks the remote half of the wiring once: `--headless` over ssh, `web` (`web.tools`, a port, under `user`) and a `web_ws` leg under it made with `send` over ssh, the canvas's edges (`web` mounts the leg; the leg reaches `user` alone), the leg's `/<leg>/ws` carrying a call through an `ssh -L` tunnel (`user.forward` of `list_agents` lists the remote agents), `git clone` of the host's git server through the gateway, the box listening on sshd and the web port only, the kernel stopped, the container removed. The selftest (`run.py --selftest`) covers the helper without Podman (a fake `podman`): the SKIP reasons, the ssh command lines, the gateway URL, the source tag, the image files, the cache by tag, and teardown after a failed start.

**The weak wiring** (fantastic's canvas is the model: `src/tui/src/kern/web.rs`, `user` -> `web` -> the socket leg, the leg reaching `user`, whose `forward` makes the call). `helpers/ssh_twins.py` builds every raw command; the scripted twins play them, the gpu twins are told them in words:

1. Start: the lead's exec agent `worker_exec` runs `start` of `ssh ... -L N:127.0.0.1:N team@127.0.0.1 'mkdir -p w && cd w && echo $$ > team.pid && exec fantastic_team --headless >> team.log 2>&1'` (one job: the tunnel and the remote kernel). One raw `ssh ... 'fantastic_team send ...'` then makes the remote `web` host (`web.tools`, port N, under `user`), its leg `remote_ws` (`web_ws.tools`, the canvas's edges), merges `remote_ws: [forward, send]` into `user`'s `callable_by` (read with `list_agents`, written back with `update_agent`: a pipe through `python3`), makes the remote exec agent `worker_exec` in the box's clone (its `can_call` lets it call the master's agents, `<master guid>|*`: send, watch, status, stop), writes the remote work program `w/work.py` (the exec agent's `connector` source, fantastic's wire client, then a small program), and boots `web`. Then `git clone` of the test's git server in the box, through the gateway.
2. Bridge: the lead's `create_agent` of `remote` (`ws_bridge.tools`, `host` 127.0.0.1, `port` N, `peer_id` `remote_ws`): it dials `ws://127.0.0.1:N/remote_ws/ws` as the canvas page dials `canvas_ws`, and says `hello` with the master's guid, so the box reaches the master's agents as `<master guid>|<agent>` (a call there is served AS THE LEG: the bridge's `ingress_rule` is `allow_all_ingress`, its `can_call` and the backend's `callable_by` say what it may do). The master opens the leg to its backends with exactly `send`, `watch`, `status` and `stop` (`bridge_record`, `Box.open_backends`; a backend minted by a spawn is opened by the operator when the script reaches the turn that needs it: a turn's `grant_leg`, see below).
3. Remote tools: a call on the remote kernel is `forward` on `remote` with `target: user` and `payload: {type: forward, target: <remote id>, payload: ...}`; the reply is the remote's answer (the exec agent's `run` in the clone: git, the fix, the test).
4. Remote work: no agent on the box makes a model call. What a remote agent does with a model it asks the master's backend for: the work program (a job of the box's exec agent, started with `run` through the bridge, the way a program and an LLM call are substitutes in fantastic's exec_bridge) `watch`es `<master guid>|<backend>`, `send`s it the work as its own `client_id`, and writes every event that comes back, in arrival order, to `<label>.events.jsonl`. The backend's turn runs on the master; its `status` and `part_*` events (a text part is streamed to a backend's inbox for any client but `cli`; a call part goes to the agent it calls, so a backend that is watched opens its first turn with words) cross the bridge as `event` frames; its tools are the box's exec agent, through the bridge's `forward`.
5. Stop: the bridge deleted, SIGTERM to the remote kernel over ssh with a bounded wait; the tunnel's ssh session ends with the kernel (`job_done` on the lead).

**The twins.** `tests/cli_ssh/{single_lead,lead_spawns_two,precreated_1_2,precreated_tree,leads_spawn_leads}` (free, scripted; `SKIP` without Podman) and `tests/cli_ssh/git_credentials` (#253, below; no gpu twin) and `tests/gpu_cli_ssh/` (the same five on the gpu tier; a keyed server is not a `SKIP` any more: the key stays on the master; the lead is told in plain words where the box is, what the wiring is and the bridge's grants, writes every command itself, and the plug counts every agent's usage; `SKIP` without the vllm variables or without Podman). The local side fixes adder.py and pushes; the box's workspace is fixed by a backend on the master that the remote asked (`w2`, or `s2` with its worker `w2`, or `rlead`), whose commands run through the bridge; the lone lead drives the box itself. The checks each twin shares (`Box.end`): solved on the test's git server, a commit from each side (found in that clone's reflog), linear history; the bridge gone, the tunnel's port refusing, no exec job running; the remote kernel stopped, and, started once more by the harness over ssh, holding its stock, the surface and the exec agent only (no backend, no spawned agent, no orphan); the work program's arrival log shows `status` and `part_*` before `done` (`streamed`; the selftest feeds it the mutations it must refuse: unwatched, a refused grant, parts after `done`, none, no status, another client's, no `done`); the container removed. The local checks are the `cli/` test's of the same name.

`cli_ssh/precreated_1_2` is also the **reconnect twin**: the remote kernel on its own (`nohup`) and the tunnel a job of its own, two messages to the lead and the operator's phase between them (`ssh_twins.reconnect_phase`): the watch grant sealed on the master (the box's watch is refused, the work program exits 1: the live mutation); a watcher the box started before; the tunnel's job stopped while a call to `w3` is in flight (it fails `bridge_down` on the box at once; `w3` is a backend kept for it, because an in-flight call aborted by the drop leaves its backend `thinking`); the tunnel made again and the master redials the known port with the bridge's `reconnect` (a ws bridge does not redial by itself, fantastic `readme_ws.md`: "explicit"); the next call through the bridge answers; a turn of the master's on `w2` streams to the watcher the box made before the break, with no new `watch` (the remote kernel keeps its watches per guid and asks for them again on the next `hello`).

`cli_ssh/git_credentials` is the **git credentials twin** (#253): the box gets the git server's token through the bridge, from the master's credential store, per operation, and stores none. The master's store `git_credentials` holds it as one ref bound to the server's host (the `env` backend, no value at rest); only the bridge's leg may `get` from it (`bridge_record(stores=...)`, the store's `callable_by`). The box is given no token (`Box(ctx, git_token=False)`: no `~/.gitconfig`, none in the master's sandbox). The git connector (`team_git.tools`) runs on the remote kernel with `credentials: {store: "<master guid>|git_credentials", ref}`, and the lead drives it through the bridge: `fetch`, a worktree of `origin/main`, the fix, `commit`, `fetch`, `push` of a work branch (the box's repository is `git init` plus the server as `origin`: the connector has no clone verb). Three messages: setup; the work; the grant revoked (a push is refused), given back (it lands), the bridge deleted (a push by the box's own `send` is refused). The box dials the git server on its own `127.0.0.1` through a small relay (a job of the box): the connector sends a credential only over https or http to loopback, and Podman's gateway is not loopback.

```
python3 e2e/run.py --only cli_ssh/precreated_1_2 --bin /ABSOLUTE/path/fantastic_team     # needs podman and a running machine
python3 -m helpers.ssh_twins --write      # in e2e/: rewrite tests/cli_ssh/*/script.json from the builders (the selftest checks they match)
```

The scripted plug's two helpers for them: `ScriptedModel.bind(**values)` fills the placeholders known only once the box runs (`{{ssh_port}}`, `{{ssh_key}}`, `{{tunnel_port}}`, `{{master_guid}}`, `{{remote_git_url}}`; a name left open at load stays as written, `bind` refuses one still open), and `{{minted:N}}` is the Nth distinct minted id in the request's `<id>` tags (a forwarded `create_agent` names its id too). A turn may carry `"grant_leg": ["w2", "{{minted:2}}"]`: before the plug answers it, the test's operator opens those backends to the bridge leg (`ScriptedModel.leg_grant`, set by `Box.bind`: both ends of each grant, exactly send/watch/status/stop, merged into what each holds). A backend minted by a spawn has no name before the lead spawns it.

Facts found while building them (fantastic at 5e74eb5):

1. `job_done` of a `start`ed job wakes its starter as a turn of its own. The remote kernel's SIGTERM therefore brings two messages in either order (the stop's reply, the tunnel's `job_done`); the script answers the first with `await`. An open forwarded channel (the bridge's socket) keeps the tunnel's ssh session up after the remote kernel ends, so the bridge is deleted (or its kernel stopped) before the stop.
2. The `web` host is not booted when a kernel loads its record (`--headless`): after a restart of the remote kernel the lead sends `boot` to `web` again.
3. An AI agent whose kernel restarts asks the model once on its next message to rebuild its working context (ai-core's recall).
4. A spawned agent's conversation is read by its spawner only: `agentic_resources` writes the spawner `pause`/`resume`/`stop` on it, and that write replaces the reach's `*` when it lands second (fantastic `src/lib/rust/crates/bundles/fantastic-agentic-resources/src/lib.rs:684`, `m.insert`; seen both ways for `history` through `user`). The spawn twins read the box clone's last commit instead.
5. The gpu twins: the vllm plug polls the local team's agents only, so the remote agents' requests are not in `cost.json` nor under `E2E_MAX_REQUESTS`; a keyed server is a `SKIP` (the box holds no key).

## The stock team of the older tests (`helpers/stock.py`)

The older groups (`task`, `secrets`, `dispatcher`, `roles`, `ui`, `app`, `turns`) once held a `team.yaml`; the binary has no config file and the team uses no blueprint, so each test builds its team the way a person does (#219, #221): every agent is a `create_agent` payload sent with `fantastic_team send`, under an id the test chooses, and every edge is written on both ends (`records.link`):

- the lead is `lead` (`ctx.lead`), made by `stock.make_lead`: the plug's model and credential ref, the shared `memory` store, and the tools it may call (`worker_exec`, `worker_files`, `queue`, `git`, `github`, those that exist; `lead_tools=` narrows them);
- the other agents, under the ids the product's code and the role cards name: the credential store, `worker_exec` and `worker_files`, `queue`, the `git` and `github` connectors against the fake forge, the queue folder and the puller; the roles registry (`agentic_resources`, with its tiers and limits) and the `tasks` registry; the board, the dispatcher with its `worker_view`, the cadencer. Every edge is written on both ends (`stock.link`);
- the team page the UI tests drive comes from the binary: `stock.serve_webui(team, port)` stops the headless team and starts it again with `webui --port N` (#224), which makes the `web` host, `team_page` and `team_ws` and grants the leg what the page sends to, for the agents that exist at that start (make them first). `stock.serve_team_page(team, port)` is the older by-hand port of `app_control::prepare_web`, still used by `ui/worker_view_opens`;
- a probe starts it with `run_flow.boot(ctx, ...)` / `stock.boot(ctx, ...)`: `forge=False` (no connectors), `git=False, puller=False`, `board=True`, `groups={name: {driver: "$lead", members: [...]}}` (the dispatcher and cadencer), `registry_tiers={...}` (a roles registry the dispatcher makes workers from), `roles_tiers={...}` plus `limits` (roles mode: the cards), `sub="a"` (a second team in a sub folder).

## Tiers

| costs | what answers | status |
| --- | --- | --- |
| `free` | a scripted model (`backends/scripted.py`) and a fake forge (`helpers/forge.py`) | built |
| `turns` | a real model through the `anthropic` plug (aisixteen/fantastic#302), run by hand, needs `ANTHROPIC_API_KEY` or `E2E_ANTHROPIC_KEY` | built |
| `gpu` | a vLLM server reached directly, through the `vllm` plug (`backends/vllm.py`, #237), run by hand, needs `VLLM_ENDPOINT` and its kin; or, with `BACKEND=anthropic`, the `anthropic` plug (#251) | built |

## Layout

- `run.py`: the launcher.
- `backends/`: plugs. A test's `test.yaml` names one (`plug: scripted`, `plug: anthropic` or `plug: vllm`); a test never names a model, an endpoint or a key. A test takes the plug's model, endpoint and host from `ctx.vars` (`model_name`, `model_endpoint`, `model_host`).
- `helpers/`: `cli.py` (the cli group: the fixture repo, fantastic's start, `create_agent` helpers, the shared script turns), `records.py` (`create`, `update`, `merge`, `link`), `forge.py` (fake GitHub API plus git over smart HTTP from a bare repo), `team.py` (start with fantastic's words such as `webui --port N`, stop, `send`, wiring, the page's chat grant), `run_flow.py` (fake keys, boot, hand the lead its message), `verdict.py` (named checks), `page.py` and `page_driver.js` (the team page's own script on a stub DOM over the live `/team_ws/ws`; needs `node` >= 22), `podman_remote.py` (the ssh remote in a Podman container, #242), `ssh_twins.py` (the ssh twins: the raw commands and scripts, the box, the shared checks) and `gpu_ssh.py` (their gpu tier: the wiring in plain words).
- `helpers/web.py`, `helpers/app.py`: HTTP and websocket client; the app shell and handle driver as processes.
- `tests/<group>/<id>/`: `README.md`, `test.yaml` (`costs`, `timeout`, `plug`, `build`: `any` (default), `general` or `github`; `run.py --build` runs the tests for `any` and that build), `script.json` (the scripted plug's turns; none for a turns test), `probe.py` (`run(ctx)` returns the exit code).
- `ui/sidebar_opens_agent_full_screen` (free): drives the team page the binary serves with `webui`, with `helpers/page_driver.js` (the page's own script `page.js` on a stub DOM, over the live `/team_ws/ws`); needs node >= 22, else it prints `SKIP` and passes.
- `tests/ui/`: the web UI (the team page on a web host, its websocket via `helpers/web.py`). The browser overflow check is not built.
- `runs/`: one folder per run, gitignored (it holds model output).

## The app and the dispatcher's web UI (#37)

The `app/` and `dispatcher/` tests drive the web UI of a live team: they serve the team page (`stock.serve_team_page`, or `webui --port N` through `stock.boot(ctx, extra_args=...)`), talk HTTP and a websocket with `helpers/web.py` (stdlib only), and run the app shell `fantastic_team_app` and the in-process handle driver `crates/team/examples/handle_serve.rs` as processes (`helpers/app.py`). `run.py` builds all three; with `--bin`, build them yourself and keep them in the same folder:

```
cargo build -p fantastic-team --bin fantastic_team --example handle_serve
cargo build -p fantastic-team-app --bin fantastic_team_app
python3 e2e/run.py --only dispatcher/quiet_heartbeat --bin "$PWD/target/debug/fantastic_team"   # --bin must be absolute
```

A probe that cannot run here raises `helpers.verdict.Skip(reason)`; the launcher prints `SKIP <name> (<reason>)` and the run still exits 0. A test whose binary (`fantastic_team_app`, `examples/handle_serve`) is not next to `--bin` skips the same way. `app/window_per_platform` also skips without a tauri build (`E2E_APP_TAURI_BIN`), a Linux display and `xdotool`.

Facts about the web UI at this commit, found while writing these tests:

1. `fantastic_team webui --port P` serves the team page `/team_page/file/index.html` and its socket `/team_ws/ws` (#224; the app's in-process handle serves the same page: `TeamHandle`, `examples/handle_serve`). The tests that need the team page build it with `stock.serve_team_page`.
2. The launcher (`fantastic_team_app`) starts `webui --port N` in the team folder and waits for a 200 at `/team_page/file/index.html` (`READY_PATH`), the page that command serves. It writes the role cards under `roles/` and, once the page answers, makes the team with `create_agent` under fixed ids when the kernel has no `lead` (it needs the home's `credentials` agent); a folder whose kernel has its `lead` is started as it is, so `app/launcher_start_attach_stop` makes one first (and has a second leg for the folder with no team). `app/attach_remote` keeps a small proxy front for its tunnel leg.
3. The Rust ai-core never emits the `tool_calling` status the page's tool blocks key on; a tool call is a `call` part sent to the watchers of its addressee.
4. The scripted plug takes `"thinking": "..."` (a reasoning block before the text) and `"delay": seconds` (the plug waits before it answers) on a turn.

## The GitHub connector tests

`task/github_merge_through_connector` (#273, free): the probe makes three branches and draft PRs with the `git`/`github` verbs; a scripted lead (an AI chat granted only `merge` on `github`) sends every `merge` against the fake forge's PRs, check runs (`forge.checks`), ready mutation and `PUT .../merge`: green draft merged, red and pending refused by check name, no token in a reply, model request or log. The forge's fixed pull request 7 (`pr_status`, roles/lead_verifies_on_github) answers only when the run opened none.

## How a free test runs

The launcher makes a bare repo and a seeded clone, starts the fake forge (the repo is `acme/widgets`, one issue labelled `team-ready` by the maintainer with a `## SPEC` comment), starts the plug, and starts `fantastic_team --headless` in the run folder with fake keys in ITS environment only (the credential store reads them with `backend: env`). The stock records and the test's blueprint are made with `send` and `import` (see above); the probe then drives the team with `fantastic_team send`.

The anthropic backend sends no `tools` array: a model calls a tool by writing a `<call name="send">` element in its text. So the scripted plug streams Anthropic SSE whose text is that element; a script turn lists `calls`, and the plug renders them.

## Things the harness works around (found while building it)

These are facts about the binary at the commit this was built on, not decisions of the suite. Each workaround is in `helpers/team.py` or `helpers/run_flow.py` and goes away when the cause is fixed.

1. `worker_files` and the puller's queue folder are sealed by default (`edge sealed by policy`); `stock.py` makes them with `ingress_rule: allow_all_ingress`, and writes the callers on both ends.
2. The binary never calls `prepare_sandbox`: `stock.make_tools` makes `work/`, `work/wt`, `work/tasks` and `sandbox/home` first. The worker's HOME is the relative `./sandbox/home`.
3. Agents are made with `create_agent` under fixed ids, so a string field that names another agent (the board's `github`, the dispatcher's `team`) and a card's grant on `github` or `worker_exec` are true as written (fantastic's import would rename them `_1`).
4. A spawner grants only what it holds, and the operator is the `mcp` leg: `stock.operator_holds` gives it the grants a card names before a probe spawns a role as the operator.
5. A model cannot pass `title` to `github pr_create`: `title` is a call-level field of the `<call>` grammar. The scripted model therefore stops after the push, and the probe sends `pr_create` itself, as the operator. The PR request is still what the github connector sends.
6. The roles registry and the board are made by `stock.py`, not by a start-up step: nothing in the binary plans a tree now.
7. The team page for the older UI tests is `stock.serve_team_page` (see above).

## The roles group

`tests/roles/*` (issue #185) drive the roles registry (`agentic_resources`) with the repo's `roles/` cards, which each probe copies into the run dir. Both are free (scripted model, fake forge), and not part of any CI: run them by hand (`python3 e2e/run.py --only roles/total_limit_refuses_fourth_worker --bin /ABS/PATH`). The scripted plug is one ordered script for every agent, so each test keeps one agent's model request open at a time.

- `roles/total_limit_refuses_fourth_worker`: `roles: limits: total: 3`; the lead spawns four workers, one call per turn; the fourth is refused with its reason; `get_count`, `fleet` and the dispatcher's `fleet` agree (3 of 3). The worker tiers are inert, so no worker asks the model.
- `roles/project_lead_plans_and_runs_issues`: the coder `project_lead` reads four fake issues, proposes a tree and waits for "yes"; then two workers are started, a PR is opened for each, and the tree stays. The chat entry, the plan view and the approval gate (LATER, UI) are skipped with a note. The README of the test lists the ordering it relies on and the gaps it works around.
- `roles/app_team_spawn_trees` (#227): the team the app makes (`fantastic_team_app --dir`, the cards of `roles/`) runs lead -> sub-lead -> worker and lead -> project_lead -> worker on the scripted plug; each parent deletes what it spawned; no spawned agent is left. Needs `fantastic_team_app` next to `--bin`.

## Later

- `driver/rulebook_reply_and_escalation` waits for the group driver (#95): there is no driver agent, verb or `main.rs` wiring (the unwired library went in #292), so there is nothing for a turns test to drive. It is not written, and it is not a turns test until #95 wires the driver.
- the `gpu` tier (a local-model plug, aisixteen/fantastic#303).
