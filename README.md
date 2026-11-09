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

The script ends with two lines: `sha256: <hash>  <name>` and `built: <path>`. The `built:` path is a versioned copy, `target/release/fantastic_team-<version>-<target>`, with the same bytes as the plain binary. `scripts/build-github.sh` prints the same two lines for `fantastic_team-github-<version>-<target>`.

There are two builds. The **general** build (Team, the default) gives workers stock exec. The **GitHub** build (Team GitHub) applies [patches/github-exec.patch](patches/github-exec.patch) to a copy of the tree, so workers cannot reach forge CLIs, `git push` or credentials: `sh scripts/build-github.sh` makes `target/github/release/fantastic_team-github`. The github connector is in both builds.

The general binary is `target/release/fantastic_team`. There are no releases yet, so build from source. On macOS the build needs `MACOSX_DEPLOYMENT_TARGET=14.0`; `.cargo/config.toml.example` sets it under `[env]`.

For the step-by-step version, with the GitHub App registration and troubleshooting, see docs/install.md. The builds are reproducible in intent: `rust-toolchain.toml` pins the toolchain, `scripts/checksums.sh` writes `SHA256SUMS`, and `scripts/verify.sh` checks a binary against it (docs/install.md, Reproducible builds).

## Configure

There is no config file. A team is agent records, and config is fantastic's: a record's own fields, a recipe's arguments and `send`. The `fantastic_team` binary is fantastic's command line with this repo's bundles added (see Run), so the records are made the way fantastic makes any agent: with `create_agent` under an id you choose. The first run in a folder seeds the team's fixed agents from `crates/team/blueprint.yaml` (ids as written: the roles grant their tools by id, and fantastic's `import` would rename `github` to `github_1`) and its role bundles under `.fantastic/bundles/`; every later run rehydrates them from disk.

- **A recipe, then `send`**: `fantastic_team --headless ai anthropic --model M` makes the root AI; further agents are `create_agent` payloads: `fantastic_team send '{"target": "kernel_state", "payload": {"type": "create_agent", ...}}'`. [examples/create_agent](examples/create_agent/README.md) shows the payloads.
- **A directory that already holds `.fantastic/agents/`** (records you wrote or copied).

Where the old `team.yaml` keys went:

| was | now |
|---|---|
| `templates` | the `tiers` field of the `agentic_resources` record, and each AI record's own fields (`model`, `endpoint`, `credentials`, `can_call`, `callable_by`) |
| `first_agent`, `directive_file` | an ordinary AI record; its directive is its `system_prompt_file` (read through its `file_bridge_id`), or a message sent with `send` |
| `roles` | the role bundles (`.fantastic/bundles/<name>/bundle.yaml`) and the `agentic_resources` record's `limits` |
| `connectors`, `queue`, `store`, `credentials` | agent records with their own fields (`team_git.tools`, `team_github.tools`, `ram_store.tools`, `sqlite_store.tools`, `credential_store.tools`) |
| `groups` | the `groups` field of the dispatcher, the `team_groups` field of the board and the `group_settings` field of the cadencer |
| `board` | the board's record (`repos`, `github`, `store`) |
| `cadencer` | the cadencer's record (`enabled`) |
| `canvases` | no switch: the canvas is fantastic's |
| `.github/team.yaml` of a repo | the `repo_config` field of the puller's record, keyed by `owner/name` |
| `--instructions`, `--workers`, `--leads`, `--per-lead` (no longer flags) | the lead's `system_prompt_file`, or a `send` to it |

The example is a script of `send` payloads: [examples/create_agent/worker_only.sh](examples/create_agent/worker_only.sh) makes one AI worker, and its `create_agent` payload is:

```json
{"type": "create_agent", "id": "w1", "parent_id": "user", "handler_module": "anthropic_backend.tools", "model": "example-model", "endpoint": "https://api.example.com/v1/messages", "credentials": {"store": "credentials", "ref": "EXAMPLE_KEY"}, "memory_store_id": "memory", "can_call": {"$self": "*", "memory": "*", "credentials": "*"}, "callable_by": {"$self": "*", "$parent": "*"}}
```

The first run of `fantastic_team` in a folder, from the CLI or the desktop app alike (`crates/team/src/hydrate.rs`), writes the blueprint's agents: `lead` (the `lead` role bundle) and, under it, `queue`, `github`, `board`, `roles` (the registry, with its limits), `tasks`, `worker_exec` and `worker_files`, with the other end of each edge on the home's `memory`, `files`, `credentials` and the root; and the role bundles beside them (a bundle you have stays as it is). The ids are fixed, so the roles' grants (`github`, `board`, `queue`, `tasks`) are true as written. Edit the records afterwards with `update_agent`, as with any agent (replace `owner/name` in `github` and `board`). The git connector is not made by the app, because it takes the absolute paths of your clone and your worktrees: make it with `send` (`create_agent`, `handler_module: team_git.tools`, `repo_dir`, `workspace_root`, `remote`, `default_branch`).

Every role's directive starts with a short section, "How the system works": which agents exist and their ids, what each tool agent is for, that an agent is started with `spawn` and deleted by the agent that spawned it (its own decision, as in fantastic), and that API keys reach every agent on their own (the team runs on fantastic's god mode, `reach`), so there is nothing to grant.

The `fantastic_team` binary creates no coding agent of its own: no lead or worker is a default, and nothing is added behind the records. `scripts/new-task.sh` renders a task file for whatever agents you made.

### Roles and limits

A role is a META BUNDLE, fantastic's `bundles/<name>/bundle.yaml` with a `base` and no program: its directive (`record.post_system_prompt`) and its grants (`record.can_call`), over an AI backend, and no model. The coder roles are compiled into the binary (`crates/team/bundles/`: `worker`, `lead`, `issue_keeper`, `issue_writer`, `spec_reviewer`, `project_lead`) and written under `.fantastic/bundles/` at the first run; the kernel loads them before the tree, `reflect` (`bundles: all`) lists them, and a role you add is a folder you drop there. The main `lead` is an instance of the `lead` bundle (`handler_module: lead` in the blueprint). The roles registry is `agentic_resources` (this repo's `crates/agentic_resources`, moved from fantastic in #245), switched on by `FANTASTIC_AGENTIC_RESOURCES=1`; it reads the roles from the kernel's bundles and keeps the limits:

```yaml
# .fantastic/bundles/worker/bundle.yaml
base: anthropic_backend.tools
sentence: Carries out tasks from the queue, one at a time, and hands each back.
record:
  can_call: {$self: '*', worker_exec: '*', worker_files: '*', queue: '*'}
  post_system_prompt: |
    # The worker directive
    ...
```

```yaml
limits:                # the registry's record; kept by set_limit; edit with `send` (`set_limit {max}` / `{role, max}`)
  total: 3
  per_role:
    worker: 2
```

The lead starts every agent with `spawn {role, model, task}` on the registry, after it asks `roles` and `get_count`; it does not use `create_agent`. The child runs on its spawner's backend (its handler and provider fields) with the `model` the spawn names (the spawner's own when it names none): the spawner picks the model from its own job prompt. `backend: {handler_module, …}` in `spawn` puts the child on another backend. The roles name the agents they grant by id (the `project_lead` role grants `github`, `board`, `queue` and `tasks`), so make those agents under exactly those ids, as the blueprint does. A spawn past a limit is refused with `{error: <sentence>, reason: "limit", ...}` (`reason` is the code, `error` the sentence); the dispatcher's sidebar shows the count (`fleet` `{running, max}`, and "agents R/M" in its footer line).

The task registry is the agent `tasks` (`task_registry`, this repo's `crates/task_registry`, moved from fantastic in #245; first aisixteen/fantastic#429): a record with `task_store_id`. The `project_lead` card grants its write verbs (`add`, `assign`, `reassign`, `status`, `report`, `list`, `get`) and `spawn` opens `tasks` to it (fantastic#436). The `project_lead` also records, with each task, the `versions` it was done against (`{repo, branch, commit}` per repo), the issue's `source_version` and the mark it was forked from (`fork_from`), and passes a child's `usage` up with `report`. The GitHub connector's `staleness` verb gives the facts for reusing a mark (the commits and files since the recorded commit, whether the issue changed) and the lead decides (#194). The `project_lead` records each issue it hands out (source `gh:owner/repo#N`), its holder (an agent or a subtree root) and every reassignment, and pushes updates to the source itself through the GitHub connector; nothing syncs automatically.

Cabinets are `cabinet.tools` (this repo's `crates/cabinet`, package `team-cabinet`, moved from fantastic's `fantastic-cabinet` in #248): a master AI opens cabinets, each a nested canvas of its own with one worker AI. `register_team` registers it, so a team kernel and team's canvas have it. It still asks fantastic's canvas (`canvas.tools`) for `new_canvas`. The verbs, fields, refusals, grants and the status cell are fantastic's, unchanged; the crate's `src/readme.md` describes them. The Python twin stays in fantastic.

The memory of every AI agent (the lead, the agents it spawns, workers added from the dispatcher) and the rows of `tasks` live in ONE store agent: the home's `memory`, a `sqlite_store` with one file, `.fantastic/memory.db`, so the history and the task rows survive a restart. The records name it (`memory_store_id`, `task_store_id`); the kernel's own bookkeeping (`kernel_memory`) stays a RAM store. A worker added from the dispatcher (`add_worker {template}`, the template being one of the backend records under the registry record's `tiers`, a field only the dispatcher reads) is granted on the store when it is made; removing it leaves its rows.

With a roles registry the `board` verb also returns a `roles` section: the tree below the `project_lead` (each agent's role and active/paused/done state, from the `fleet` and the tasks it holds), the registry's `get_count` as `fleet`, and the `tasks` rows (source, holder, status). A team without one keeps the old rows only.

The `project_lead` waits for its children with the kernel's `await_set`; the lead holds `await_set`, `await_cancel`, `create_agent` and `delete_agent` on the root (its record's `can_call`), and a spawn opens them to the child its role grants them to.

In roles mode the team page has two columns: an Agents list (the lead first, labelled Lead, then its tree indented by depth) beside the pane of the opened agent. The lead's pane shows the plan (the task registry's rows) above its chat and redraws it in place; any other agent shows its chat alone. Approval is the user's chat reply to the lead: its role says when it asks (a large plan, past its limits, GitHub writes) and honours "show me the plan first". Not done yet (#185, #198, later): the e2e. A spawned agent that reads a key must still be named on the credential store (it is not a role grant).

The sidebar of every team (roles mode or not) also has a Team section with the team's own services: the queue, the board, the github connector and the dispatcher itself (the sidebar reply's `services`). Opening one answers `{view: panel, kind, target, model}` and the right pane draws the model as tables of text (never markup) and refreshes it every five seconds: the queue by queued, taken and done tasks with title, holder and age (failed ones only in the counts); the board by the lead's agent tree and the registry's tasks (without roles mode, its agents and the task counts by state); the github connector by its repos, its last calls (verb, target, ok / refused / unreachable / error status) and its rate limit and permissions; the dispatcher by its workers and lanes, with agents running over the cap. The github view is built from a whitelist of the connector's `reflect` reply (which now lists `recent_calls` and `rate`) and never shows a token or a key. The TUI shows these agents in its generic panel (the `reflect` reply and the verb entry); a panel of their own there is not part of this.

A repo the puller serves is one in its record's `repos`. Its settings are the `repo_config` field of that record, keyed by `owner/name`:

```yaml
task_labels: [team-ready]
paths: [src/**, docs/*.md]
build: cargo build
test: cargo test
review:
  approvals: 1
  approvers: [alice]
```

Every key is optional. A repo with no entry takes the defaults: the label `team-ready`, any path, and one approval from any reviewer.

Two more sets of fields are optional. `groups` (a field of the dispatcher's record) names the worker groups the dispatcher shows, each with its members; the board's `team_groups` field adds each group's AI `driver` (an agent id; absent means the group has no AI), and the cadencer's `group_settings` its timing. The board's `repos` names the repos it caches, through the GitHub connector its `github` field names:

```yaml
groups:
  build: [worker_1, worker_2]
team_groups:
  build: {driver: lead, members: [worker_1, worker_2]}
repos: [aisixteen/fantastic_team]
github: git
```

`templates/` holds example directive files to start from.

**Register the GitHub App.** The GitHub connector needs an app you own. `fantastic_team setup github --name NAME [--org ORG]` runs the manifest flow: a local page posts the app's manifest to GitHub, GitHub redirects back, and the code is exchanged once for the app id and private key. The key goes to the credential store and is never written to a file or printed; the app id is stored beside it, in the same credential ref. `--org` creates the app under an organization. docs/install.md, step 3, lists the permissions and the by-hand steps as a fallback.

## Lanes

A lane is one worker template a team may start. Lanes differ in model, price, and the tools granted to them. Run several lanes at once, mix models, and compare them on the same tasks.

Tasks are exact: each says what to read, what to produce, and how to check it. The lead gives each worker its task. The cadencer's digest to a group's AI driver marks a worker that holds a task and stops making progress as `STALLED`, with the idle time.

## Run

The command line is fantastic's, unchanged. `fantastic_team` is `fantastic` with this repo's bundles added to its registry, the way `canvas` sits on top of the kernel. It takes fantastic's words and one of its own (`webui`), and it runs in the directory it is started in (`cd` there first; fantastic has no `--dir`):

```
fantastic_team                                  # the terminal UI, on a terminal
fantastic_team --headless                       # no screen and no port
fantastic_team webui --port 8123                # serves the team page on 127.0.0.1:8123
fantastic_team canvas --port 8123               # serves fantastic's canvas on 127.0.0.1:8123
fantastic_team ai anthropic --model M           # the root AI on a backend, then the terminal
fantastic_team run htop                         # a terminal cell running that command
fantastic_team send '<item json>'               # one call into the team running here
fantastic_team readme                           # fantastic's own readme
```

`canvas [--port N] [--host ADDR]`, `ai <backend> …`, `run …`, `system`, `--headless`, `mcp`, `send`, `import`, `migrate-home` and `readme` are fantastic's words; `fantastic readme` documents them. The team page, the board and the dispatcher are agents like any other: the bundles are registered, and an agent comes from a record. The canvas is registered too but only started by the `canvas` word; an agent can still make a canvas of its own with `create_agent`. The team's own first words are `setup github`, `packs …`, `ai-readme` and `check` (the dry check of a `team.yaml`, docs/team-yaml.md). `webui [--port N] [--host ADDR]` is the team's own recipe word: it serves the team page (two columns and the agent list) and its socket from the binary, headless like `canvas` (the browser is the View), with the same `--port` and `--host`. Without `--port` it keeps the port recorded here, else picks a free one, and it prints `the team page: <url>`. `--headless` opens no port, even in a folder `webui` has run in. `webui` and `canvas` together are refused: both mount a socket leg on the web host's one `/{host_id}/ws` route and the second would win, so run one. The team no longer has its own `--dir`, `--port`, `--tui`, `--instructions` or `--resend-instructions`; they are refused as unknown. Ctrl-C to stop.

A directive for the lead is its record's `system_prompt_file`, or a message sent to it: `fantastic_team send '{"target": "lead", "payload": {"type": "send", "text": "..."}}'`.

`fantastic_team ai-readme` prints [docs/ai-readme.md](docs/ai-readme.md), the same guide for AI agents, and exits.

The build packs fantastic's canvas page with a pinned esbuild. With no vendored binary it compiles esbuild itself, which needs Go on PATH. CI installs Go 1.23.4 (`setup-go` in `.github/workflows/ci.yml`).

## Run it on another machine

The team can run on a box with a GPU while you start and watch it from a laptop over SSH. It runs detached, like a tmux session: closing the laptop, the tunnel or a terminal never stops it. Until these become `fantastic_team remote …` subcommands, they are scripts:


```
scripts/remote-add.sh gpu box.example.com:/srv/team --port 8093 --keys "ANTHROPIC_API_KEY"   # once
scripts/remote-start.sh gpu
scripts/remote-ls.sh
scripts/remote-attach.sh gpu     # Ctrl-C detaches; gpu keeps running
scripts/remote-logs.sh gpu -f
scripts/remote-stop.sh gpu       # asks first; drains (SIGTERM); --now kills
```

- **Keys only at start.** The laptop config names variables, never values. `remote-start.sh` reads their values from the laptop's environment and pipes them over SSH stdin. They are never in an argv, a file or shell history.
- **127.0.0.1 only.** The team binds the box's loopback; `remote-attach.sh` reaches it through an SSH tunnel. No public port.
- **After the box reboots** the team is down, because its keys were only in memory. `remote-ls.sh` says so.
- Every script takes `--dry-run`, which prints the commands and no secret.

## Observe

- **Cost and time per task.** Each task records its worker, model, time, tokens and cost, computed from the template's prices. Compare lanes on what a task really costs.
- **The board** (`team_board.tools`): the shared picture of the work — open issues with their queue state (pending, in work, done, failed) and the agents. Verb `board`.
- **The worker view** (`team_worker_view.tools`): one canvas per worker, with its task, files, output and chat.
- **The dispatcher** (`team_dispatcher.tools`): the home screen of the web UI and the app. Workers by group, each with its state, current task, time and cost, and what is waiting for you.
- **The web UI** with `webui --port`, or fantastic's terminal UI.
- **Logs** through `scripts/remote-logs.sh` when the team runs remotely.

## The app

`crates/team_app` builds `fantastic_team_app`, a launcher that starts the binary (`webui --port 8765`) and waits until its web UI answers. `app/tauri.conf.json` is the packaging for a desktop window (macOS, Windows, Linux); the window loads the web UI on `127.0.0.1` only. The window is not built yet. The dispatcher is the home screen: each worker row has Start, Pause and Stop buttons, and Stop asks first. The GitHub build is the same app with the GitHub patch applied. Build steps, controls and the signing notes are in [app/README.md](app/README.md).

## Security model

- Secrets live only in fantastic's credential store (your OS keychain, or an env / file backend in a container). An agent reads a secret when it uses it and never returns, logs or stores it. Tests check that no secret appears in any reply, error, event, store row, record or log file (`crates/secrets_guard`). `scripts/check-secrets.sh` fails when a tracked or new file holds a private key or a GitHub or Anthropic token; run it before you commit.
- Workers run commands through an exec agent the team configures (`crates/team/src/worker_exec.rs`): HOME is a sandbox folder in the team (no forge login, no git credential helper, no SSH keys), the environment is a fixed list of names (no token, no key, no SSH agent), and forge CLIs (`gh`, `glab`, `hub`, `git push`) are refused before they start. That refusal is an early, clear error, not a wall. It is not an OS sandbox: a worker runs as your user, so run the team as a separate user or in a container to keep it away from files your user can read (`~/.ssh`, the login keychain).
- Workers hold no credentials. Delivering (pushing a branch, opening a pull request, posting a comment) goes through a connector that owns the credentials and the rules.
- Delivery agents enforce the rules themselves: no pushes to the default branch, no force-pushes, pull requests as drafts, an allowlist of repositories.
- You review and merge. The team does not merge: its GitHub directive says never merge (`templates/code-github.md`), and docs/install.md step 6 says merging stays human.

## The lead

A team is run by **the lead**: an AI agent, an ordinary record, that works from a directive. It spawns worker agents when there is work, gives each a task, checks on them, and deletes workers when the queue is empty. Its model, directive and tiers come from records; no provider or model is named in the code.

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

The cadencer works with or without a board (a CLI team has none). It is on by default. A
cadencer record with `enabled: false` is switched off, and so is one whose groups all set
`enabled: false`. The
team page has a "Digests (cadencer)" switch in its header that calls `set
{enabled}`; the state is kept in the cadencer's record and shown again after a restart.
With a `roles:` section the digest tree is the spawned tree (the project_lead, its
sub-leads and their workers, read from the roles registry, the root's `reflect` and the
task registry): each lead gets a digest of its own children, with stalls and the
heartbeat. Without `roles:` it reads the board's rows, as before. For the digests the
cadencer is granted `send` on the leads the roles registry lists, at the start and
before each beat.

The cadencer's `status` carries the team's load as plain counts: workers busy,
idle and stalled (per group and in all), tasks held, in review, done, blocked
and queued, the number done in the last hour, and the tokens the drivers have
spent. The code gives only these numbers. It has no mode, no total and no bar.
The group's driver AI reads them in its digest, one plain `load:` line, and
decides what to show: progress against a fixed list, or load alone when work
keeps arriving. With no AI in a group, the raw numbers are all that is shown.
Tasks waiting on CI are not counted (that needs a GitHub poll). Lanes in use are
not reported: the pinned fantastic does not expose them. Tokens come from the
`reflect` usage of each group driver (workers are not counted); the first beat
after a restart counts the driver's lifetime total once, and a counter that
drops (a restarted driver) counts from zero again.

## Any model, any mix

Workers are fantastic AI agents: Anthropic models over the API with your own key, or local models (vLLM, llama.cpp) on your own GPU.

## Delivery: optional, pluggable

Results can stay as files in the team's folder. When work belongs somewhere else, a connector takes it there:

- **git**: a branch pushed to any remote (SSH or HTTPS).
- **GitHub** (optional): as a GitHub App you register yourself. Draft pull requests, issue comments, labels, reading a task queue from issues. Register with your own app; the private key stays in your credential store.

## Worker packs

A worker pack is a folder: `pack.yaml` (the manifest: name, version, author, license, price `free`, the sha256 content hash of the other files, an empty signature, the ports the pack needs, and the list of files), `manifest.yaml`, `worker.json` (the record with its model, credentials and other bindings stripped), and `directives/`, `templates/`, `evals/`. The `team_pack.tools` verbs are `pack {worker, out}`, `install {pack, into, worker?, bindings?, timeout_s?, min_pass_rate?}` and `eval {pack, worker, timeout_s?}`. `eval` runs the pack's evals on a worker and reports the pass rate; it writes nothing. `install` with a `worker` also runs the evals on it after the copy and reports them under `evals`, and replies with an advisory `enable {allowed, min_pass_rate, reason}` verdict. `min_pass_rate` is a number in `0..=1` (default `1.0`, every eval passing); `allowed` is false below it, and when the evals cannot be loaded. A pack with no evals is allowed. The verdict never fails or rolls back the install.

The `eval` reply, and the `evals` report of an `install` with a worker, also carry `measured {pack_hash, tasks, successes, success_rate, cost_per_task, measured_by, signed}`: the pack's measured record for that run. It is local and unsigned (`signed` is false, `measured_by` is empty for now), is never written to disk or published, and holds no task text or paths.

- `pack` scans every file before it writes anything (`crates/team/src/pack_scan.rs`). It refuses secrets, directives or templates that ask for a key, read `~/.ssh` or send data out, and any code file (`.rs`, `.py`, `.sh`, executables under `bin/`, …). A refused pack writes nothing and the reply lists each finding by path, rule and line, never the matched text.
- `install` refuses a pack with no `pack.yaml`, an invalid manifest, a content hash that differs from the files it lists (a tampered file), a scan refusal, or an installed version. It checks all of this before its first write, then copies the pack as a template under `<into>/<name>-<version>/`. It creates no agent. The reply carries the pack's parsed ports (`{"kind": "model" | "connector" | "exec:sandboxed", …}`) so the caller can show what the pack asks for. `bindings` is the buyer's map from each port key (`model`, `connector:<name>`, or `exec:sandboxed`; two `@model` ports share `model`) to the id of the agent that fills it. `install` refuses a key that is not a port key of the pack, a value that is not a plain agent id (a secret-shaped value or one with path characters is refused, and the value is never echoed), and a non-object, all before its first write. The bound map is written to `<dest>/bindings.json`, outside the hash. The reply carries `bindings: {bound, unbound}`, where `unbound` lists the ports no binding fills. With no bindings, every port is unbound and a plain `install` still succeeds: binding is checked where a worker is built, not at the copy (see **Enabling**).

**Surfaces.** The pack verbs are reachable locally, with no network and no hosted service. Three agents serve them, made as records like any other (`pack` on `team_pack.tools`: `pack`, `install`, `eval`; `pack_listing` on `team_packs.tools`: `list`, `show`, `updates`, `publish`; and `files`, a file bridge rooted at the team directory that only those two may call). The CLI sends to the ids `pack` and `pack_listing`. From the shell, with the team running in the directory: `fantastic_team packs list|show|updates|publish|pack|install|eval --key value ...` (for example `fantastic_team packs install --pack mypack --into templates --worker w1`). A pack that carries a `blueprint/` folder (fantastic's blueprint format, see below) can also be built into a worker: `fantastic_team packs install --pack P --into templates --parent-id lead --approve` (without `--approve` it only previews). Over MCP, `fantastic_team mcp` reaches the agents `pack` and `pack_listing`. `pack_listing` reads the local index at `packs/index.json` and installed packs under `templates/`; `trusted_keys` is unset, so no signed index entry is trusted. `pack` writes its files without a final newline, because the file bridge does not return one and `install` must read back exactly what was hashed. `eval` and `install --worker` send each eval task to the worker by id. The `pack` agent may call the agents its own record's `can_call` names; any worker must list `pack` in its own `callable_by`, or the eval fails with `pack may not call 'send'` or the worker's refusal. The CLI has no `--bindings` or `--min-pass-rate` flag (use `send` or MCP for those), but `packs install` run from a terminal with no bindings in its payload asks, before it sends, for the agent id of each port in `<pack>/pack.yaml` (an empty answer skips a port) and sends the answers as `bindings`; it asks first because a second install of the same version is refused. Without a terminal nothing is asked. The CLI prints the reply and exits non-zero when it carries an `error` (or when `send` fails), so a script can detect a failed `install` or `eval`.

**Blueprint packs.** `pack` also packs `<worker>/blueprint/` (`blueprint.yaml` plus `<agent id>/directive.md`, the folder `user.export_subtree` writes) next to `worker.json`: the files are listed in `pack.yaml`, covered by the content hash and scanned (a directive in it is scanned like any other). A blueprint folder without `blueprint.yaml` is refused. `install` with a `parent_id` then builds the worker through fantastic's `user.import_subtree` (#269) from the installed `<dest>/blueprint`, after every normal check and the template copy. Without `approve` the reply carries the permission preview under `approval` (agents, new ids, what they reach, ports, code) and nothing is created; with `approve: true` it carries `imported`, the new `ids`, and `installed_as: worker`. The pack's `bindings` become the blueprint's ports (`model` is `@model`, `connector:<name>` is `@connector:<name>`, `exec:sandboxed` is `@exec`), and fantastic refuses an unbound port, a key value or a code file; the refusal is the reply's `error` (`blueprint import refused: ...`) and the template copy stays. `allow_code` is passed through and defaults to false (`pack` already refuses code files). A pack with no blueprint and a `parent_id` is refused before any write. The `pack` agent may call only `import_subtree` on the team's `user` agent. The CLI flags are `--parent-id`, `--approve` and `--allow-code`. The `<dest>/blueprint` path is read by fantastic relative to the team directory, which is also the root of the `files` agent. Without `parent_id` nothing changes: `install` creates no agent.

**Enabling.** Where a worker is built from an installed template (`install` with `parent_id` and `approve: true`), the team enforces the bindings and the verdict before `user.import_subtree` is asked. It refuses, in this order, an unbound port, a bound agent id that is not a live agent (read from the root's `list_agents`, which the `pack` agent may call and nothing else on the root), and, when a `worker` was given so the evals ran, a verdict that is not `allowed`. A refusal creates no agent, keeps the template copy, and replies with `error: enable refused: ...` and `enable_error`; the error names the port keys, never the ids. The preview (no `approve`) never refuses on these. An installed version can be enabled again from its copy: `install` with a `parent_id` on a version whose installed `pack.yaml` has the same content hash as the source pack (and whose files are unchanged) skips the copy, takes `<dest>/bindings.json` as the default bindings (a `bindings` key in the payload overrides it per port) and goes through the same gate and import. A changed pack at an installed version, or a second install with no `parent_id`, is still refused with `version already installed`. The gate checks that a bound agent exists; it does not check that the agent meets the port's window or tools, because the kernel exposes no field to check them against. Without a `worker` there is no verdict, so only the first two checks apply.

**Packs panel.** The first start places no agent for it: the team page (`team_page`, served at `/team_page/file/index.html`) serves the team page and its Pack install panel. The panel lists the packs (`pack_listing list`), shows a pack's permission preview (`show`: ports, grants, price, evals, the measured record, whether it is installed or revoked, and the install decision), asks for one agent id per unbound port, previews an install (`install` without `approve`), installs after a confirm (`install` with `approve: true`) and runs the evals (`eval`). It sends no other verb and draws every value as text.

Not yet: signatures, signing and publishing the measured record (it is built locally from eval results, in the `eval` and `install` replies, but is never signed or sent; `publishable` is the gate), checking that a bound agent meets the port's window and tools, fetching pack files from a registry over the network, and `trusted_keys` on the `pack_listing` record. `publish {pack, url, signing_key, grants?}` signs a packed pack with the 64-hex ed25519 seed in the file at `signing_key` and appends it to the local index (checks first: manifest, content hash, secrets scan; https `url`; no duplicate version or hash). It needs no network, and the CLI has no `--grants` flag (use `send` or MCP). The entry is listed only when its signer is in `trusted_keys`.

## Tests and CI

Unit tests: `cargo test --workspace`. Also `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `sh scripts/check-secrets.sh .`, and the shell tests `sh tests/new_task.sh`, `sh tests/remote_scripts.sh`, `sh tests/build_github.sh`, `sh tests/repro_scripts.sh`. CI also runs `sh scripts/build-github.sh --check` (the GitHub patch applies) and `sh scripts/build-github.sh --test` (the GitHub build's tests). None of them needs a network or a key.

End to end (e2e/README.md) has three tiers:

| costs | what answers | status |
| --- | --- | --- |
| `free` | a scripted model (`backends/scripted.py`) and a fake forge (`helpers/forge.py`) | built |
| `turns` | a real model through the `anthropic` plug, run by hand, needs `ANTHROPIC_API_KEY` or `E2E_ANTHROPIC_KEY` | built |
| `gpu` | a local-model plug on the GPU box | not built |

- `python3 e2e/run.py --only-free`: every free test. No network, no key; it builds the binary with cargo.
- `python3 e2e/run.py --build github --bin PATH`: the tests for the GitHub build.
- `python3 e2e/run.py --only-turns`: the turns tests, with a real model, by hand. Without a key they are skipped. Never in CI.
- `python3 e2e/run.py --only-gpu`: the five cli scenarios on the shared vLLM server (the `vllm` plug), by hand, one test at a time. Without `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW` they are skipped. Never in CI. See `e2e/README.md`.
- `sh tests/e2e.sh BIN`: the model-free e2e on one built binary.

CI is manual: `.github/workflows/ci.yml` runs only on `workflow_dispatch`. `gh workflow run ci.yml` runs the unit tests on Linux, macOS and Windows; `gh workflow run ci.yml -f units_os=linux` runs them on Linux only (a full run always uses all three). `gh workflow run ci.yml -f e2e=true` is the full run (release builds, the reproducible-build check and the model-free e2e), for a finished milestone or a release. `-f cleanup=true` also saves the build caches at the end of each job (off by default; run it on main to refresh the caches).

## What is in this repo

| Part | What it is |
|---|---|
| `fantastic_team` | The program: fantastic's command line (its terminal-shell library) plus the agents below. Its flags are fantastic's. |
| the lead | An AI agent, an ordinary record. |
| `queue` agent | Tasks as data: add, list, take, mark done. |
| `team_puller.tools` | Reads `team-ready` issues from the GitHub connector and writes `queue/<repo>-<n>.md`. `status` gives the source, the last pull and the counts. |
| connectors | `git` (`team_git.tools`) and `github` (optional, `team_github.tools`). Each is a record, with its directive. |
| `team_board.tools` | Board: the shared picture of the work. Verbs `pull`, `board`, `task`, `report`, `agent`, `ask`, `audit`. It never assigns. |
| `team_worker_view.tools` | Worker view: one vertical canvas per worker. |
| `team_webhook.tools` | Optional webhook receiver (off unless enabled). `status` lists the last 20 events (event, action, accepted or refused, time) and whether a secret is set, never the secret, a header or a body. |
| `team_dispatcher.tools` | The dispatcher: the home screen. It reads and asks; Start, Pause and Stop are its only controls. |
| cadencer | Sends each group's AI driver a digest of its workers, and reports the load counts in `status`. |
| `team_pack.tools` | Worker packs: verbs `pack`, `install` and `eval`. See Worker packs. |
| declared views | Each kind above (cadencer, puller, webhook, packs, pack, git, github, board, dispatcher, queue) declares its read-only views as data in `declared_views.rs`: the fantastic TUI draws them from `view.tui`, written into the records at every start, and the team page draws the same declaration for cadencer, puller, webhook, packs, pack and git. The queue, board, github and team pages stay page code (`agent_views.rs`, with the reason). No view shows a secret. |
| `team_app` | The launcher `fantastic_team_app` for the desktop app. See The app. |
| `crates/team/src/new_team.rs` | The team's fixed ids (the lead, the roles registry, the tasks registry, the queue, the board, the GitHub connector, the worker's tools; the blueprint `crates/team/blueprint.yaml` declares them) and the writer of the role bundles (`crates/team/bundles/`, embedded in `crates/team/src/roles.rs`). |
| `examples/create_agent/` | A script of `fantastic_team send` `create_agent` payloads. |
| `examples/team.yaml` | A team's setup as one preset yaml, with `fantastic_team check` ([docs/team-yaml.md](docs/team-yaml.md)). |

## Updating fantastic

fantastic is a pinned git dependency. `scripts/update-fantastic.sh` moves all fantastic crates to the newest commit of `main` and runs the build and tests. Then commit the lock file. Use `.cargo/config.toml.example` (copied to a git-ignored `.cargo/config.toml`) to test an unreleased local fantastic checkout; never commit that copy.

## Status

Being built by fantastic's own workers, from exact tasks. Progress: the [project board](https://github.com/orgs/aisixteen/projects/2) and the [issues](https://github.com/aisixteen/fantastic_team/issues).

License: AGPL-3.0-or-later, like fantastic.
