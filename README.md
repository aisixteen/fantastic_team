# AISixteen Fantastic Team

Program: `fantastic_team`. The long install guide is docs/install.md.

**Unattended team work on [fantastic](https://github.com/aisixteen/fantastic).** You queue tasks; a team of AI workers takes them one by one, does the work on its own, and hands back finished results for you to review. You come back to done work, not to a chat.

It is not only for code. A task is any piece of work that can be described exactly and checked: code, documentation, research, triage, data, review, translation or ports.

## Install

You need `git`, a Rust toolchain with `cargo`, and Linux or macOS. `gh` is optional. CI unit-tests on Linux, macOS and Windows; the run was verified on Linux only (docs/install.md).

```
git clone https://github.com/aisixteen/fantastic_team
cd fantastic_team
sh scripts/build.sh
./target/release/fantastic_team --help
```

The script ends with two lines: `sha256: <hash>  <name>` and `built: <path>`. The `built:` path is a versioned copy, `target/release/fantastic_team-<version>-<target>`, with the same bytes as the plain binary.

The binary is `target/release/fantastic_team`. There are no releases yet, so build from source. On macOS the build needs `MACOSX_DEPLOYMENT_TARGET=14.0`; `.cargo/config.toml.example` sets it under `[env]`.

For the step-by-step version, with the GitHub App registration and troubleshooting, see docs/install.md. The build is reproducible in intent: `rust-toolchain.toml` pins the toolchain, `scripts/checksums.sh` writes `SHA256SUMS`, and `scripts/verify.sh` checks a binary against it (docs/install.md, Reproducible builds).

## Configure

There is no config file. A team is agent records, and config is fantastic's: a record's own fields, a recipe's arguments and `send`. The `fantastic_team` binary is fantastic's command line with this repo's bundles added (see Run), so the records are made the way fantastic makes any agent: with `create_agent` under an id you choose. The first run in a folder seeds the team's fixed agents from `crates/team/blueprint.yaml` (ids as written: the bundles grant their tools by id, and fantastic's `import` would rename `github` to `github_1`) and its bundles under `.fantastic/bundles/`; every later run rehydrates them from disk.

- **A recipe, then `send`**: `fantastic_team --headless ai anthropic --model M` makes the root AI; further agents are `create_agent` payloads: `fantastic_team send '{"target": "kernel_state", "payload": {"type": "create_agent", ...}}'`.
- **A directory that already holds `.fantastic/agents/`** (records you wrote or copied).

The agents are record files under `.fantastic/agents/` that the kernel loads at start; change one with `update_agent`.

The first run of `fantastic_team` in a folder, from the CLI or the desktop app alike (`crates/team/src/hydrate.rs`), writes the blueprint's agents: `lead` (the `lead` bundle) and, under it, `github`, `bundles` (the registry), `tasks` (the task registry, its tracker `github`), `worker_exec` and `worker_files`, with the other end of each edge on the home's `memory`, `files`, `credentials` and the root; and the bundles beside them (a bundle you have stays as it is). The ids are fixed, so the bundles' grants (`github`, `bundles`, `tasks`, `worker_exec`, `worker_files`) are true as written. Edit the records afterwards with `update_agent`, as with any agent (replace `owner/name` in `github`). The `dispatcher` and the `cadencer` are not seeded: the lead makes one with `create_agent`, under that id, when it judges it useful (#292). The git connector is not made by the app, because it takes the absolute paths of your clone and your worktrees: make it with `send` (`create_agent`, `handler_module: team_git.tools`, `repo_dir`, `workspace_root`, `remote`, `default_branch`).

Every bundle's directive starts with a short section, "How the system works": which agents exist and their ids, what each tool agent is for, that an agent is started with `spawn` and deleted by the agent that spawned it (its own decision, as in fantastic), and that API keys reach every agent on their own (the team runs on fantastic's god mode, `reach`), so there is nothing to grant.

The `fantastic_team` binary creates no coding agent of its own: no lead or worker is a default, and nothing is added behind the records. `scripts/new-task.sh` renders a task file for whatever agents you made.

### Bundles

A bundle is a META BUNDLE, fantastic's `bundles/<name>/bundle.yaml` with a `base` and no program: its directive (`record.post_system_prompt`) and its grants (`record.can_call`), over an AI backend, and no model. The coder bundles are compiled into the binary (`crates/team/bundles/`: `worker`, `lead`, `issue_keeper`, `issue_writer`, `spec_reviewer`, `project_lead`) and written under `.fantastic/bundles/` at the first run; the kernel loads them before the tree, `reflect` (`bundles: all`) lists them, and a bundle you add is a folder you drop there. The main `lead` is an instance of the `lead` bundle (`handler_module: lead` in the blueprint). The bundles registry is `agentic_resources` (this repo's `crates/agentic_resources`, moved from fantastic in #245); it reads the bundles from the kernel's bundles and writes no field of its own on the agents it spawns. A bundle:

```yaml
# .fantastic/bundles/worker/bundle.yaml
base: anthropic_backend.tools
sentence: Carries out the task its lead hands it and hands it back with one message.
record:
  can_call: {$self: '*', worker_exec: '*', worker_files: '*'}
  post_system_prompt: |
    # The worker directive
    ...
```

The lead starts every worker and sub-lead with `spawn {bundle, model}` on the registry (it creates the agent and sends nothing; the lead then gives it its work with a plain `send`), after it asks `bundles`; it does not use `create_agent` for them. The child runs on its spawner's backend (its handler and provider fields) with the `model` the spawn names (the spawner's own when it names none): the spawner picks the model from its own job prompt. `backend: {handler_module, …}` in `spawn` puts the child on another backend. The bundles name the agents they grant by id (the `project_lead` bundle grants `github`), so make those agents under exactly those ids, as the blueprint does. The bundles registry's own view shows the count (`get_count`).

The task registry is the agent `tasks` (`task_registry`, this repo's `crates/task_registry`, moved from fantastic in #245; first aisixteen/fantastic#429), the team's one tasks agent: a record with `task_store_id` (its tables) and `task_tracker_id` (the agent that is the tracker: the `github` connector). It is seeded with the team (#351). `pull` reads the tracker's open issues (every one, on the repos the connector's record lists) and adds each one that is not a task yet as a waiting task (`owner/name#N`, source `gh:owner/name#N`); the lead and the `project_lead` use its verbs (`pull`, `add`, `assign`, `reassign`, `status`, `report`, `list`, `get`) and write a result back to the issue through the connector. Its own html view (the Board page of the design mock) is what the team page mounts for it. The `project_lead` also records, with each task, the `versions` it was done against (`{repo, branch, commit}` per repo), the issue's `source_version` and the mark it was forked from (`fork_from`), and passes a child's `usage` up with `report`. The GitHub connector's `staleness` verb gives the facts for reusing a mark (the commits and files since the recorded commit, whether the issue changed) and the lead decides (#194). The `project_lead` records each issue it hands out (source `gh:owner/repo#N`), its holder (an agent or a subtree root) and every reassignment, and pushes updates to the source itself through the GitHub connector; nothing syncs automatically.

The memory of every AI agent (the lead, the agents it spawns, workers added from the dispatcher) and the rows of `tasks` live in ONE store agent: the home's `memory`, a `sqlite_store` with one file, `.fantastic/memory.db`, so the history and the task rows survive a restart. The records name it (`memory_store_id`, `task_store_id`); the kernel's own bookkeeping (`kernel_memory`) stays a RAM store. A worker added from the dispatcher (`add_worker {template}`, the template being one of the backend records under the registry record's `tiers`, a field only the dispatcher reads) is granted on the store when it is made; removing it leaves its rows.

The `project_lead` waits for its children with the kernel's `await_set`; the lead holds `await_set`, `await_cancel`, `create_agent` and `delete_agent` on the root (its record's `can_call`), and a spawn opens them to the child its bundle grants them to.

In bundles mode the team page has two columns: an Agents list (the lead first, labelled Lead, then its tree indented by depth) beside the pane of the opened agent. The lead's pane shows the plan (the task registry's rows) above its chat and redraws it in place; any other agent shows its chat alone. Approval is the user's chat reply to the lead: its bundle says when it asks (a large plan, GitHub writes) and honours "show me the plan first". Not done yet (#185, #198, later): the e2e. A spawned agent that reads a key must still be named on the credential store (it is not a bundle grant).

The team page's left column is fantastic's sidebar agent's html view (the agent tree, each chat's state, what waits for you, the files). Opening an agent asks its own `reflect`; only when the page's leg may not read it, the dispatcher's `open` grants the leg what drawing it takes (an agent's html view: its `reflect` and the view's calls; a worker's chat: the chat verbs; a file's view agent: `reflect`, `read`, `watch`) and the page reflects again.

Two sets of fields are optional. `groups` (a field of the dispatcher's record) names the worker groups the dispatcher shows, each with its members, and the cadencer's `group_settings` its timing:

```yaml
groups:
  build: [worker_1, worker_2]
```

`templates/` holds example directive files to start from.

**GitHub auth.** With `GH_TOKEN` set, the stock tree's `github_credentials` store (`env: {token: GH_TOKEN}`) gives the connector a personal token, with no setup. For your own GitHub App, set the connector's credential record to `env: {app_id: VAR, private_key: VAR}` (PEM text or a .pem path). Without a credential the team works local only (worktrees, branches, commits) and every GitHub verb says so. Nothing pastes or stores a token.

## Lanes

A lane is one worker template a team may start. Lanes differ in model, price, and the tools granted to them. Run several lanes at once, mix models, and compare them on the same tasks.

Tasks are exact: each says what to read, what to produce, and how to check it. The lead gives each worker its task. The cadencer's digest to a group's AI driver marks a worker that holds a task and stops making progress as `STALLED`, with the idle time.

## Run

The command line is fantastic's, unchanged. `fantastic_team` is `fantastic` with this repo's bundles added to its registry. It takes fantastic's words and one of its own (`webui`), and it runs in the directory it is started in (`cd` there first; fantastic has no `--dir`):

```
fantastic_team                                  # the terminal UI, on a terminal
fantastic_team --headless                       # no screen; the page's web host listens on its recorded or a free 127.0.0.1 port
fantastic_team webui --port 8123                # serves the team page on 127.0.0.1:8123
fantastic_team ai anthropic --model M           # the root AI on a backend, then the terminal
fantastic_team run htop                         # a terminal cell running that command
fantastic_team send '<item json>'               # one call into the team running here
fantastic_team readme                           # fantastic's own readme
```

`ai <backend> …`, `run …`, `system`, `--headless`, `mcp`, `send`, `import`, `migrate-home` and `readme` are fantastic's words; `fantastic readme` documents them. The team page, the task registry and the dispatcher are agents like any other: the bundles are registered, and an agent comes from a record. `webui [--port N] [--host ADDR]` is the team's own recipe word: it serves the team page (two columns and the agent list) and its socket from the binary, headless like `canvas` (the browser is the View), with the same `--port` and `--host`. Without `--port` it keeps the port recorded here, else picks a free one, and it prints `the team page: <url>` when `--port` is given. A recorded `web` boots in every mode, so a plain `--headless` listens on a free 127.0.0.1 port. `webui` and `canvas` together are refused: both mount a socket leg on the web host's one `/{host_id}/ws` route and the second would win, so run one. The team no longer has its own `--dir`, `--port`, `--tui`, `--instructions` or `--resend-instructions`; they are refused as unknown. Ctrl-C to stop.

A directive for the lead is its record's `system_prompt_file`, or a message sent to it: `fantastic_team send '{"target": "lead", "payload": {"type": "send", "text": "..."}}'`.

`fantastic_team readme` prints fantastic's readme followed by [docs/ai-readme.md](docs/ai-readme.md), the same guide for AI agents.

The build packs fantastic's canvas page with a pinned esbuild. With no vendored binary it compiles esbuild itself, which needs Go on PATH. CI installs Go 1.23.4 (`setup-go` in `.github/workflows/ci.yml`).

## Observe

- **Cost and time per task.** Each task records its worker, model, time, tokens and cost, computed from the template's prices. Compare lanes on what a task really costs.
- **Usage per model and per agent, on demand.** The provider counts it: `fantastic_team send '{"target": "provider", "payload": {"type": "usage"}}'` (the provider's id; no new verb). `by_model` has each model's requests and tokens (input, cache write, cache read, output) with `cost_usd` from the record's `prices` map or `unpriced: true`; `by_agent` the same per calling chat id (kept after the chat is deleted); the top level is the team total. The e2e `cli/usage_report` checks it against a scripted run.
- **The worker view** (`team_worker_view.tools`): one canvas per worker, with its task, files, output and chat.
- **The dispatcher** (`team_dispatcher.tools`): what the page's leg may call to open a node (`open`), and the control of a worker or group (`control`: start, pause, stop, hand a task, add or remove a worker).
- **The web UI** with `webui --port`, or fantastic's terminal UI.

## The app

`crates/team_app` builds `fantastic_team_app`, a launcher that starts the binary (`webui --port 8765`) and waits until its web UI answers. `app/tauri.conf.json` is the packaging for a desktop window (macOS, Windows, Linux); the window loads the web UI on `127.0.0.1` only. The window is not built yet. The dispatcher is the home screen: each worker row has Start, Pause and Stop buttons, and Stop asks first. Build steps, controls and the signing notes are in [app/README.md](app/README.md).

## Security model

- Secrets live only in fantastic's credential store (your OS keychain, or an env / file backend in a container). An agent reads a secret when it uses it and never returns, logs or stores it. The `git` and `github` agents have tests that check no secret appears in any reply, error, event, store row, record or log file (`tests_secrets.rs` in each crate).
- Workers run commands through an exec agent the team configures (`crates/team/src/worker_exec.rs`): HOME is a sandbox folder in the team (no forge login, no git credential helper, no SSH keys), the environment is a fixed list of names (no token, no key, no SSH agent). It is not an OS sandbox: a worker runs as your user, so run the team as a separate user or in a container to keep it away from files your user can read (`~/.ssh`, the login keychain).
- Workers hold no credentials. Delivering (pushing a branch, opening a pull request, posting a comment) goes through a connector that owns the credentials and the rules.
- Delivery agents enforce the rules themselves: no pushes to the default branch, no force-pushes, pull requests as drafts, an allowlist of repositories.
- You review and merge. The team does not merge: its GitHub directive says never merge (`templates/code-github.md`), and docs/install.md step 6 says merging stays human.

## The lead

A team is run by **the lead**: an AI agent, an ordinary record, that works from a directive. It spawns worker agents when there is work, gives each a task, checks on them, and deletes workers when no task waits. Its model, directive and tiers come from records; no provider or model is named in the code.

## Load and progress

**Quiet mode and the heartbeat.** In quiet mode a lead gets only what changed and
every stall. So that a driver with idle members and queued work does not sleep, a
quiet lead that got no digest for three beats in a row, while a direct member is
idle (no task in hand) and tasks are queued, gets one line:
`heartbeat: N idle, M queued`. A regular digest restarts the count. Each group in
the cadencer's `group_settings` field may set `cadence`, `stall_after`, `quiet` and `enabled` (whole seconds,
at least 1); an unset key follows the global setting, which `set` still changes
live. A group `cadence` longer than the timer's beat skips beats until it is due; a
shorter one is held to the beat.

The cadencer works with or without a bundles registry (a CLI team has none). It is on by default. A
cadencer record with `enabled: false` is switched off, and so is one whose groups all set
`enabled: false`. The
team page has a "Digests (cadencer)" switch in its header that calls `set
{enabled}`; the state is kept in the cadencer's record and shown again after a restart.
With a `bundles:` section the digest tree is the spawned tree (the project_lead, its
sub-leads and their workers, read from the bundles registry, the root's `reflect` and the
task registry): each lead gets a digest of its own children, with stalls and the
heartbeat. Without `bundles:` it beats on the tree a `tick` gives it. For the digests the
cadencer is granted `send` and `status` on the leads the bundles registry lists, at the
start and before each beat; the `inquiry_from` of a lead's `status` tells it an inquiry is open on that lead.

The cadencer's `status` carries the team's load as plain counts: workers busy,
idle and stalled (per group and in all), tasks held, in review, done, blocked
and queued, and the number done in the last hour. The code gives only these numbers. It has no mode, no total and no bar.
The group's driver AI reads them in its digest, one plain `load:` line, and
decides what to show: progress against a fixed list, or load alone when work
keeps arriving. With no AI in a group, the raw numbers are all that is shown.
Tasks waiting on CI are not counted (that needs a GitHub poll). Lanes in use are
not reported: the pinned fantastic does not expose them.

## Any model, any mix

Workers are fantastic AI agents: Anthropic models over the API with your own key, or local models (vLLM, llama.cpp) on your own GPU.

## Delivery: optional, pluggable

Results can stay as files in the team's folder. When work belongs somewhere else, a connector takes it there:

- **git**: a branch pushed to any remote (SSH or HTTPS).
- **GitHub** (optional): as a GitHub App you register yourself. Draft pull requests, issue comments, labels, and the tracker the task registry pulls its tasks from. Register with your own app; the private key stays in your credential store.

## Export and import

A worker or the whole team is exported and imported with fantastic's own blueprint verbs, `user.export_subtree` and `user.import_subtree` (a preview, then `approve`). The team adds no format or verb of its own.

## Tests and CI

Unit tests: `cargo test --workspace`. Also `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and the shell tests `sh tests/new_task.sh`, `sh tests/repro_scripts.sh`. None of them needs a network or a key.

End to end (e2e/README.md) has three tiers:

| costs | what answers | status |
| --- | --- | --- |
| `free` | a scripted model (`backends/scripted.py`) and a fake forge (`helpers/forge.py`) | built |
| `turns` | a real model through the `anthropic` plug, run by hand, needs `ANTHROPIC_API_KEY` or `E2E_ANTHROPIC_KEY` | built |
| `gpu` | a local-model plug on the GPU box | not built |

- `python3 e2e/run.py --only-free`: every free test. No network, no key; it builds the binary with cargo.
- `python3 e2e/run.py --only-turns`: the turns tests, with a real model, by hand. Without a key they are skipped. Never in CI.
- `python3 e2e/run.py --only-gpu`: the five cli scenarios on the shared vLLM server (the `vllm` plug), by hand, one test at a time. Without `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW` they are skipped. Never in CI. See `e2e/README.md`.

CI is manual: `.github/workflows/ci.yml` runs only on `workflow_dispatch`. `gh workflow run ci.yml` runs the unit tests on Linux, macOS and Windows; `gh workflow run ci.yml -f os=linux` runs them on Linux only (`os` picks the OS set of every job, full runs included). `gh workflow run ci.yml -f e2e=true` is the full run (release builds, the reproducible-build check and the model-free e2e), for a finished milestone or a release. `-f cleanup=true` also saves the build caches at the end of each job (off by default; run it on main to refresh the caches).

## What is in this repo

| Part | What it is |
|---|---|
| `fantastic_team` | The program: fantastic's command line (its terminal-shell library) plus the agents below. Its flags are fantastic's. |
| the lead | An AI agent, an ordinary record. |
| `tasks` (`task_registry.tools`) | The team's one tasks agent: every task, its holder and its source, pulled from the tracker (`pull`) and moved by the leads (`add`, `assign`, `reassign`, `status`, `report`, `list`, `get`). Its own html view is the Board page. |
| connectors | `git` (`team_git.tools`) and `github` (optional, `team_github.tools`: the tracker and the source control). Each is a record, with its directive. |
| `team_worker_view.tools` | Worker view: one vertical canvas per worker. |
| `team_dispatcher.tools` | The dispatcher: `open` grants the page's leg what opening a node takes; `control` starts, pauses, stops, hands a task to, adds or removes a worker. |
| cadencer | Sends each group's AI driver a digest of its workers, and reports the load counts in `status`. |
| html views | The task registry, the cadencer, the github connector and the bundles registry carry their own html views in their `reflect`; `declared_views.rs` lists them and grants the page's leg each one's `reflect` and calls. No view shows a secret. |
| `team_app` | The launcher `fantastic_team_app` for the desktop app. See The app. |
| `crates/team/src/new_team.rs` | The team's fixed ids (the lead, the bundles registry, the GitHub connector, the task registry, the worker's tools; the blueprint `crates/team/blueprint.yaml` declares them) and the writer of the bundles (`crates/team/bundles/`, embedded in `crates/team/src/bundles.rs`). |

## Updating fantastic

fantastic is a pinned git dependency. `scripts/update-fantastic.sh` moves all fantastic crates to the newest commit of `main` and runs the build and tests. Then commit the lock file. Use `.cargo/config.toml.example` (copied to a git-ignored `.cargo/config.toml`) to test an unreleased local fantastic checkout; never commit that copy.

## Status

Being built by fantastic's own workers, from exact tasks. Progress: the [project board](https://github.com/orgs/aisixteen/projects/2) and the [issues](https://github.com/aisixteen/fantastic_team/issues).

License: AGPL-3.0-or-later, like fantastic.
