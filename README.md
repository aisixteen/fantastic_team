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

A team is a folder with a `team.yaml`. The file names the worker templates the team may start, the connectors it may use, the queue, and the credential store.

```yaml
templates:
  writer:
    handler_module: anthropic_backend
    model: example-model
    endpoint: https://api.example.com/v1
    credentials:
      store: credentials
      ref: EXAMPLE_KEY
    price_input_per_mtok: 1.0
    price_output_per_mtok: 5.0
first_agent:
  id: lead
  template: writer
  directive_file: lead.md
connectors:
  git:
    kind: team_git.tools
    directive_file: git.md
queue:
  id: queue
credentials:
  id: credentials
  backend: env
  refs:
    EXAMPLE_KEY: EXAMPLE_KEY
```

- `templates`: each is an agent template (model, endpoint, credential ref, prices per million tokens).
- `first_agent` (optional): the lead, made from a template, instructed by its `directive_file`. Without it no agent is created.
- `connectors`: GitHub, a plain git remote, or others, each with its own directive.
- `queue`: where tasks wait.
- `credentials`: the store the team creates when it has none. The values are never in this file; only the ref names are.
- `roles` (optional): turns on fantastic's roles registry (`agentic_resources`) for the team, see below.

### The caller defines the agents

The `fantastic_team` binary creates no coding agent of its own: nothing is added behind `team.yaml`, and no lead or worker is a default. A `team.yaml` with no `templates` and no `first_agent` is valid and starts only the system agents (the dispatcher, the tools, the queue). `queue` defaults to the agent `queue`. Which agents exist, with or without a lead, with or without `roles:`, is the file's choice. `scripts/new-task.sh` renders a task file for whatever agents your `team.yaml` defines.

One worker, no lead, no `roles:` (the worker is the `first_agent`; give `--instructions` to send it work):

```yaml
# worker-only
templates:
  worker:
    handler_module: anthropic_backend
    model: example-model
    endpoint: https://api.example.com/v1
    can_call: [worker_exec, worker_files, queue]
first_agent:
  id: w1
  template: worker
```

With `roles:`, the lead spawns the agents it needs through the registry, so `roles:` needs a `first_agent` and templates (the tiers the role cards name):

```yaml
# roles mode
templates:
  strong:
    handler_module: anthropic_backend
    model: example-model
    endpoint: https://api.example.com/v1
    can_call: [kernel, kernel_state, worker_exec, worker_files, queue]
  small:
    handler_module: anthropic_backend
    model: example-model
    endpoint: https://api.example.com/v1
    can_call: [worker_exec, worker_files, queue]
  issue_keeper:
    handler_module: anthropic_backend
    model: example-model
    endpoint: https://api.example.com/v1
    can_call: [queue]
first_agent:
  id: lead
  template: strong
  directive_file: roles/lead.md
connectors:
  git:
    kind: team_git.tools
    record: {repo_dir: /path/to/clone, workspace_root: /path/to/worktrees, remote: origin, default_branch: main}
  github:
    kind: team_github.tools
    record: {repos: [owner/name], git: git}
board:
  repos: [owner/name]
  github: github
roles:
  limits:
    total: 3
```

The role cards (`roles/` of this repo) go in a `roles/` folder next to `team.yaml`.

The desktop app ships this second shape as a preset, `crates/team/preset/app_team.yaml`. When the app starts a team in a folder with no `team.yaml`, it writes the preset and the role cards there once. It is an ordinary `team.yaml`: edit it, and the app never overwrites it.

### Roles and limits

With a `roles:` section the team gets a registry agent (`roles` by default). The coder role cards live in the `roles/` folder of this repo, one file each: `worker`, `lead`, `issue_keeper`, `issue_writer`, `spec_reviewer` and `project_lead`. A card is a `---` header (`name`, `purpose`, `tier`, `grants`) and the directive text. The `tier` is a key: the name of a template in `team.yaml` (`small`, `strong`, `issue_keeper` in the example), so a card never names a model. The registry holds one tier per template. The lead starts every agent with `spawn` on the registry, after it asks `roles` and `get_count`; it does not use `create_agent`.

```yaml
roles:
  id: roles        # the registry agent's id (default roles)
  dirs: [roles]    # card folders, relative to the team's directory (default [roles])
  limits:          # set with set_limit at every start; absent keys clear the old limit
    total: 3
    per_role:
      worker: 2
```

The team refuses to start when a card does not parse, names a tier that is not a template, grants an agent `team.yaml` does not define (the `project_lead` card grants the `github` connector and the `board`, so a team with `roles:` defines both, or ships its own `project_lead.md`), or when one of the six cards is missing. A spawn past a limit is refused with `{error: "limit", reason}`; the dispatcher's sidebar shows the count (`fleet` `{running, max}`, and "agents R/M" in its footer line). The team's one store agent (below) keeps every spawned agent's conversation under its own id.

With `roles:` the team also gets the agent `tasks` (fantastic's `task_registry`, aisixteen/fantastic#429), a child of the lead that keeps its rows in the team's one store agent; the id `tasks` is taken. The `project_lead` card grants its write verbs (`add`, `assign`, `reassign`, `status`, `report`, `list`, `get`) and the lead admits it on `tasks` like on every other tool. The `project_lead` records each issue it hands out (source `gh:owner/repo#N`), its holder (an agent or a subtree root) and every reassignment, and pushes updates to the source itself through the GitHub connector; nothing syncs automatically.

The memory of every AI agent (the lead, the tiers of the roles registry, workers added from the dispatcher) and the rows of `tasks` live in ONE store agent: a `sqlite_store` with one file per team directory, `.fantastic/memory.db`, so the history and the task rows survive a restart. It is config, not enforced in code: the optional `store:` section of `team.yaml` renames it or moves its file (`store: {id: memory, path: .fantastic/memory.db}`; the path is relative to the team directory, no `..`, and the id must not be another agent's). The kernel's own bookkeeping (`kernel_memory`) stays a RAM store. A worker added from the dispatcher is granted on the store when it is made; removing it leaves its rows.

With `roles:` the `board` verb also returns a `roles` section: the tree below the `project_lead` (each agent's role and active/paused/done state, from the `fleet` and the tasks it holds), the registry's `get_count` as `fleet`, and the `tasks` rows (source, holder, status). A team without `roles:` keeps the old rows only.

The `project_lead` waits for its children with the kernel's `await_set`; each start names the agents below the root for `await_set` and `await_cancel` in the root's `callable_by`.

In roles mode the dispatcher page has two columns: an Agents list (the lead first, labelled Lead, then its tree indented by depth) beside the pane of the opened agent. The lead's pane shows the plan (the task registry's rows) above its chat and redraws it in place; any other agent shows its chat alone. Approval is the user's chat reply to the lead: its card says when it asks (a large plan, past its limits, GitHub writes) and honours "show me the plan first". Not done yet (#185, #198, later): the e2e. `spawn` does not open a tool's `callable_by` to the new agent: the lead names each spawned agent on the tools its card lists (the lead card says how), and the credential store must name it too.

The sidebar of every team (roles mode or not) also has a Team section with the team's own services: the queue, the board, the github connector and the dispatcher itself (the sidebar reply's `services`). Opening one answers `{view: panel, kind, target, model}` and the right pane draws the model as tables of text (never markup) and refreshes it every five seconds: the queue by queued, taken and done tasks with title, holder and age (failed ones only in the counts); the board by the lead's agent tree and the registry's tasks (without roles mode, its agents and the task counts by state); the github connector by its repos, its last calls (verb, target, ok / refused / unreachable / error status) and its rate limit and permissions; the dispatcher by its workers and lanes, with agents running over the cap. The github view is built from a whitelist of the connector's `reflect` reply (which now lists `recent_calls` and `rate`) and never shows a token or a key. The TUI shows these agents in its generic panel (the `reflect` reply and the verb entry); a panel of their own there is not part of this.

Each repo that the team serves can opt in with `.github/team.yaml`:

```yaml
task_labels: [team-ready]
paths: [src/**, docs/*.md]
build: cargo build
test: cargo test
review:
  approvals: 1
  approvers: [alice]
```

Every key is optional. An empty file gives the defaults: the label `team-ready`, any path, and one approval from any reviewer.

Two more keys in the team's `team.yaml` are optional. `groups` names the worker groups the dispatcher shows: each has an AI `driver` (an agent id; absent means the group has no AI) and its `members`. `board` names the repos the board caches, through a GitHub connector of the same file:

```yaml
groups:
  build:
    driver: lead
    members: [worker_1, worker_2]
board:
  repos: [aisixteen/fantastic_team]
  github: git
```

`examples/team.yaml` is a complete team config with both keys in place (commented out), and `templates/` holds example directive files to start from.

**Register the GitHub App.** The GitHub connector needs an app you own. `fantastic_team setup github --name NAME [--org ORG]` runs the manifest flow: a local page posts the app's manifest to GitHub, GitHub redirects back, and the code is exchanged once for the app id and private key. The key goes to the credential store and is never written to a file or printed; only the app id is written to the config. `--org` creates the app under an organization. docs/install.md, step 3, lists the permissions and the by-hand steps as a fallback.

## Lanes

A lane is one worker template a team may start. Lanes differ in model, price, and the tools granted to them. Run several lanes at once, mix models, and compare them on the same tasks.

Tasks are exact: each says what to read, what to produce, and how to check it. The lead gives each worker its task. The cadencer's digest to a group's AI driver marks a worker that holds a task and stops making progress as `STALLED`, with the idle time.

## Run

```
./target/release/fantastic_team --dir ./my-team            # headless: no port, no terminal UI
./target/release/fantastic_team --dir ./my-team --port 8123  # also serves the web UI on 127.0.0.1:8123
./target/release/fantastic_team --dir ./my-team --tui        # the terminal UI (needs a terminal)
```

`--port` and `--tui` are the only surfaces and are not combined. Ctrl-C to stop.

The first agent's instructions come from `--instructions "<text>"` or `--instructions-file <path>` (not both), and from counts: `--workers <n>` starts n workers; `--leads <m> --per-lead <k>` starts m leads, each with k workers (`--workers` and `--leads` are not combined). Given text comes first. With no instruction flag the team starts with no first message. `--resend-instructions` sends the first agent its instructions again when it already exists.

`fantastic_team ai-readme` prints [docs/ai-readme.md](docs/ai-readme.md), the same guide for AI agents, and exits. `mcp`, `send` and `readme` as the first word run fantastic's own commands in this directory.

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
- **The web UI** with `--port`, or the terminal UI with `--tui`.
- **Logs** through `scripts/remote-logs.sh` when the team runs remotely.

## The app

`crates/team_app` builds `fantastic_team_app`, a launcher that starts the headless binary on port 8765 and waits until its web UI answers. `app/tauri.conf.json` is the packaging for a desktop window (macOS, Windows, Linux); the window loads the web UI on `127.0.0.1` only. The window is not built yet. The dispatcher is the home screen: each worker row has Start, Pause and Stop buttons, and Stop asks first. The GitHub build is the same app with the GitHub patch applied. Build steps, controls and the signing notes are in [app/README.md](app/README.md).

## Security model

- Secrets live only in fantastic's credential store (your OS keychain, or an env / file backend in a container). An agent reads a secret when it uses it and never returns, logs or stores it. Tests check that no secret appears in any reply, error, event, store row, record or log file (`crates/secrets_guard`). `scripts/check-secrets.sh` fails when a tracked or new file holds a private key or a GitHub or Anthropic token; run it before you commit.
- Workers run commands through an exec agent the team configures (`crates/team/src/worker_exec.rs`): HOME is a sandbox folder in the team (no forge login, no git credential helper, no SSH keys), the environment is a fixed list of names (no token, no key, no SSH agent), and forge CLIs (`gh`, `glab`, `hub`, `git push`) are refused before they start. That refusal is an early, clear error, not a wall. It is not an OS sandbox: a worker runs as your user, so run the team as a separate user or in a container to keep it away from files your user can read (`~/.ssh`, the login keychain).
- Workers hold no credentials. Delivering (pushing a branch, opening a pull request, posting a comment) goes through a connector that owns the credentials and the rules.
- Delivery agents enforce the rules themselves: no pushes to the default branch, no force-pushes, pull requests as drafts, an allowlist of repositories.
- You review and merge. The team does not merge: its GitHub directive says never merge (`templates/code-github.md`), and docs/install.md step 6 says merging stays human.

## The lead

A team is run by **the lead**: an AI agent configured in `team.yaml` that works from a directive. It spawns worker agents when there is work, gives each a task, checks on them, and deletes workers when the queue is empty. Its model, directive and templates come from config; no provider or model is named in the code.

## Load and progress

**Quiet mode and the heartbeat.** In quiet mode a lead gets only what changed and
every stall. So that a driver with idle members and queued work does not sleep, a
quiet lead that got no digest for three beats in a row, while a direct member is
idle (no task in hand) and tasks are queued, gets one line:
`heartbeat: N idle, M queued`. A regular digest restarts the count. Each group in
`team.yaml` may set `cadence`, `stall_after`, `quiet` and `enabled` (whole seconds,
at least 1); an unset key follows the global setting, which `set` still changes
live. A group `cadence` longer than the timer's beat skips beats until it is due; a
shorter one is held to the beat.

The plan always creates the `cadencer` agent with these settings, with or without a
`board` section (a CLI team has none). It is on by default. `cadencer: {enabled: false}`
in `team.yaml` creates it switched off, and so does `enabled: false` on every group. The
dispatcher page has a "Digests (cadencer)" switch in its header that calls `set
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

**Surfaces.** The pack verbs are reachable locally, with no network and no hosted service. Three agents sit under the lead in every team: `pack` (`pack`, `install`, `eval`), `pack_listing` (`list`, `show`, `updates`, `publish`) and `files`, the file bridge rooted at the team directory that only those two may call. From the shell, with the team running in the directory: `fantastic_team packs list|show|updates|publish|pack|install|eval --key value ...` (for example `fantastic_team packs install --pack mypack --into templates --worker w1`). A pack that carries a `blueprint/` folder (fantastic's blueprint format, see below) can also be built into a worker: `fantastic_team packs install --pack P --into templates --parent-id lead --approve` (without `--approve` it only previews). Over MCP, `fantastic_team mcp` reaches the agents `pack` and `pack_listing`. `pack_listing` reads the local index at `packs/index.json` and installed packs under `templates/`; `trusted_keys` is unset, so no signed index entry is trusted. `pack` writes its files without a final newline, because the file bridge does not return one and `install` must read back exactly what was hashed. `eval` and `install --worker` send each eval task to the worker by id. The `pack` agent may call the lead and the workers listed under `groups:` in `team.yaml`, and the lead accepts it; any other worker must list `pack` in its own `callable_by`, or the eval fails with `pack may not call 'send'` or the worker's refusal. The CLI has no `--bindings` or `--min-pass-rate` flag (use `send` or MCP for those), but `packs install` run from a terminal with no bindings in its payload asks, before it sends, for the agent id of each port in `<pack>/pack.yaml` (an empty answer skips a port) and sends the answers as `bindings`; it asks first because a second install of the same version is refused. Without a terminal nothing is asked. The CLI prints the reply and exits non-zero when it carries an `error` (or when `send` fails), so a script can detect a failed `install` or `eval`.

**Blueprint packs.** `pack` also packs `<worker>/blueprint/` (`blueprint.yaml` plus `<agent id>/directive.md`, the folder `user.export_subtree` writes) next to `worker.json`: the files are listed in `pack.yaml`, covered by the content hash and scanned (a directive in it is scanned like any other). A blueprint folder without `blueprint.yaml` is refused. `install` with a `parent_id` then builds the worker through fantastic's `user.import_subtree` (#269) from the installed `<dest>/blueprint`, after every normal check and the template copy. Without `approve` the reply carries the permission preview under `approval` (agents, new ids, what they reach, ports, code) and nothing is created; with `approve: true` it carries `imported`, the new `ids`, and `installed_as: worker`. The pack's `bindings` become the blueprint's ports (`model` is `@model`, `connector:<name>` is `@connector:<name>`, `exec:sandboxed` is `@exec`), and fantastic refuses an unbound port, a key value or a code file; the refusal is the reply's `error` (`blueprint import refused: ...`) and the template copy stays. `allow_code` is passed through and defaults to false (`pack` already refuses code files). A pack with no blueprint and a `parent_id` is refused before any write. The `pack` agent may call only `import_subtree` on the team's `user` agent. The CLI flags are `--parent-id`, `--approve` and `--allow-code`. The `<dest>/blueprint` path is read by fantastic relative to the team directory, which is also the root of the `files` agent. Without `parent_id` nothing changes: `install` creates no agent.

**Enabling.** Where a worker is built from an installed template (`install` with `parent_id` and `approve: true`), the team enforces the bindings and the verdict before `user.import_subtree` is asked. It refuses, in this order, an unbound port, a bound agent id that is not a live agent (read from the root's `list_agents`, which the `pack` agent may call and nothing else on the root), and, when a `worker` was given so the evals ran, a verdict that is not `allowed`. A refusal creates no agent, keeps the template copy, and replies with `error: enable refused: ...` and `enable_error`; the error names the port keys, never the ids. The preview (no `approve`) never refuses on these. An installed version can be enabled again from its copy: `install` with a `parent_id` on a version whose installed `pack.yaml` has the same content hash as the source pack (and whose files are unchanged) skips the copy, takes `<dest>/bindings.json` as the default bindings (a `bindings` key in the payload overrides it per port) and goes through the same gate and import. A changed pack at an installed version, or a second install with no `parent_id`, is still refused with `version already installed`. The gate checks that a bound agent exists; it does not check that the agent meets the port's window or tools, because the kernel exposes no field to check them against. Without a `worker` there is no verdict, so only the first two checks apply.

**Packs panel.** The first start also places a pack browser on the canvas (the `packs_view` html view, next to the dispatcher). It lists the packs (`pack_listing list`), shows a pack's permission preview (`show`: ports, grants, price, evals, the measured record, whether it is installed or revoked, and the install decision), asks for one agent id per unbound port, previews an install (`install` without `approve`), installs after a confirm (`install` with `approve: true`) and runs the evals (`eval`). It sends no other verb and draws every value as text.

Not yet: signatures, signing and publishing the measured record (it is built locally from eval results, in the `eval` and `install` replies, but is never signed or sent; `publishable` is the gate), checking that a bound agent meets the port's window and tools, fetching pack files from a registry over the network, and `trusted_keys` in `team.yaml`. `publish {pack, url, signing_key, grants?}` signs a packed pack with the 64-hex ed25519 seed in the file at `signing_key` and appends it to the local index (checks first: manifest, content hash, secrets scan; https `url`; no duplicate version or hash). It needs no network, and the CLI has no `--grants` flag (use `send` or MCP). The entry is listed only when its signer is in `trusted_keys`.

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
- `sh tests/e2e.sh BIN`: the model-free e2e on one built binary.

CI is manual: `.github/workflows/ci.yml` runs only on `workflow_dispatch`. `gh workflow run ci.yml` runs the unit tests on Linux, macOS and Windows; `gh workflow run ci.yml -f units_os=linux` runs them on Linux only (a full run always uses all three). `gh workflow run ci.yml -f e2e=true` is the full run (release builds, the reproducible-build check and the model-free e2e), for a finished milestone or a release. `-f cleanup=true` also saves the build caches at the end of each job (off by default; run it on main to refresh the caches).

## What is in this repo

| Part | What it is |
|---|---|
| `fantastic_team` | The program: fantastic's terminal-shell library plus the agents below. Headless by default. |
| the lead | An AI agent configured in `team.yaml`. |
| `queue` agent | Tasks as data: add, list, take, mark done. |
| `team_puller.tools` | Reads `team-ready` issues from the GitHub connector and writes `queue/<repo>-<n>.md`. |
| connectors | `git` (`team_git.tools`) and `github` (optional, `team_github.tools`). Bound in `team.yaml`, each with its directive. |
| `team_board.tools` | Board: the shared picture of the work. Verbs `pull`, `board`, `task`, `report`, `agent`, `ask`, `audit`. It never assigns. |
| `team_worker_view.tools` | Worker view: one vertical canvas per worker. |
| `team_webhook.tools` | Optional webhook receiver (off unless enabled). |
| `team_dispatcher.tools` | The dispatcher: the home screen. It reads and asks; Start, Pause and Stop are its only controls. |
| cadencer | Sends each group's AI driver a digest of its workers, and reports the load counts in `status`. |
| `team_pack.tools` | Worker packs: verbs `pack`, `install` and `eval`. See Worker packs. |
| `team_app` | The launcher `fantastic_team_app` for the desktop app. See The app. |
| `team.yaml` | The lead, the worker templates, the connectors and their directives, the queue, the roles and their limits. |

## Updating fantastic

fantastic is a pinned git dependency. `scripts/update-fantastic.sh` moves all fantastic crates to the newest commit of `main` and runs the build and tests. Then commit the lock file. Use `.cargo/config.toml.example` (copied to a git-ignored `.cargo/config.toml`) to test an unreleased local fantastic checkout; never commit that copy.

## Status

Being built by fantastic's own workers, from exact tasks. Progress: the [project board](https://github.com/orgs/aisixteen/projects/2) and the [issues](https://github.com/aisixteen/fantastic_team/issues).

License: AGPL-3.0-or-later, like fantastic.
