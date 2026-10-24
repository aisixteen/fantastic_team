# fantastic_team: README for AIs

You are reading the text of the `fantastic_team` binary, the team's start-up. Use only what is written here. Every command below exists in this repo.

## 1. Build and check

- Build: `cargo build --workspace`. The binary is `fantastic_team`.
- Check: `fantastic_team ai-readme` prints this file. `fantastic_team --help` prints the usage text.
- Tests: `cargo test --workspace`. Secrets check: `scripts/check-secrets.sh .`
- e2e: `python3 e2e/run.py --only-free` runs the binary on a scripted model and a fake forge (no network, no key). See `e2e/README.md`.

## 2. Start the team

Run it from the directory the team lives in, or pass `--dir PATH` (created if missing).

- No flags: fully headless. No port, no terminal UI. This is the default.
- `--port N`: serve the web UI on 127.0.0.1:N.
- `--tui`: the terminal UI. It needs a terminal; without one it exits with code 2. `--port` and `--tui` cannot be combined.
- `--dir PATH`: the team's directory.
- `--instructions TEXT` or `--instructions-file PATH`: the first agent's instructions. Its flags `--workers N`, `--leads N`, `--per-lead N` say how many agents run and are put into those instructions.
- `--resend-instructions`: send the first agent its instructions again, even if it already exists.
- `fantastic_team mcp`, `fantastic_team send`, `fantastic_team readme` run fantastic's own commands in this directory (`readme` is fantastic's readme, not this file).

When the team starts it reads `team.yaml` from its directory (if there is one) and creates the agent tree from it, then sends the instructions to the first agent.

The web UI's home screen is the dispatcher (agent `dispatcher`): a sidebar of
the team's groups. In each group, every worker is followed by a `subagents (N)`
expander, and its subagents (kernel agents whose parent is that worker) show
under it only when expanded. A click on a worker or a subagent opens that agent
as a full-screen chat, and Back returns to the sidebar with its scroll and
expansion kept. The dispatcher is placed on `canvas_page` at the first start
only. It reads no secrets and sends no work. The `sidebar` verb answers the
list; `open` answers the chat for one agent (`view: chat`, `target`, `fullscreen`).
`control {node, action, confirm?}` sends start, pause or stop to a worker (any other action is refused); stop needs `confirm: true`, and the answer is `{sent, error?}`.

## 3. team.yaml

`examples/team.yaml` is the reference. Copy it into the team's directory and edit it. Its sections:

- `templates`: one per agent kind. Each has `handler_module`, `model`, `endpoint`, `credentials` (a store and a ref), prices, `can_call` (what the agent may reach) and `extra`.
- `first_agent`: `id`, `template`, `directive_file`.
- `connectors`: for example `git` (`kind: team_git.tools`, with `repo_dir`, `workspace_root`, `remote`, `default_branch`) and a `directive` that says what its agents may do.
- `queue`: the queue agent's id.
- `groups` (optional): the team's groups by name. Each has `members` (worker ids) and an optional `driver` (the agent that gets the group's digest; none means the group has no AI). The dispatcher shows these groups; the board reads them too.
- `board` (optional): `repos` (`owner/name`, at least one) and `github` (the id of a connector of this file). The board caches the open issues and PRs of those repos and is created under the root, so the GitHub connector and the queue name it in their `callable_by`. Without a `board` section there is no board agent.
- `credentials`: `backend: env`, and per ref the host and the NAMES of the environment variables that hold the key (for example `ANTHROPIC_API_KEY`).

Keys are never written in `team.yaml`. Set the environment variables named there in your shell, and tell the user which names to set.

## 4. Run it on a remote box

Remote scripts run from the laptop, in `scripts/`:

- `remote-add.sh <name> <host>:<dir> [--bin PATH] [--port N] [--keys "NAME …"] [--dry-run]`: once, to save the remote.
- `remote-start.sh <name> [--port N] [--dry-run]`: start it detached. The values of the `--keys` variables are read on the laptop and sent over SSH stdin. They never appear in an argument list or a file.
- `remote-ls.sh [--dry-run]`: every remote and its state.
- `remote-attach.sh <name> [--port N] [--dry-run]`: the web UI through a tunnel, at http://127.0.0.1:N. `--tui` is not supported yet.
- `remote-logs.sh <name> [-f] [-n LINES]`: the team's log.
- `remote-stop.sh <name> [--now] [--yes] [--dry-run]`: drains the team (SIGTERM). `--now` kills it. It asks first unless `--yes`.

## 5. Give it work

Work comes from GitHub issues. A maintainer adds the label `team-ready` to an issue, and a `## SPEC` comment is written by the maintainer or the app. The puller (`crates/team/src/puller.rs`) takes only issues whose latest `team-ready` event is by the maintainer. It writes `queue/<repo>-<n>.md` and a pending `.state` file. It never overwrites a state file. Read `queue/` to see what was taken.

## 6. Rules

- Never put a key, token or password in a file, an argument, a commit or a log. Keys come from the environment variables named in `team.yaml`. Run `scripts/check-secrets.sh .` before you commit.
- Never push to the default branch (`main`), and never force-push. Workers push work branches through the git connector only.
- Merging is the maintainer's decision.
- Ask before stopping a running team. `remote-stop.sh` asks unless `--yes` is given.

## 7. Troubleshooting

- `--tui needs a terminal`: run without `--tui`, or use `--port N`.
- `--port and --tui cannot be combined`: drop one of them.
- `cannot use --dir …`: the directory cannot be created or entered. Fix its path or permissions.
- `… is not set on this laptop`: export the variable named in the remote's keys, then run `remote-start.sh` again.
- `no remote \`name\``: add it first with `remote-add.sh`.
- `already running · pid …`: the team is up. Use `remote-stop.sh` before starting again.

## 8. Pack evals

A worker pack may carry sample tasks in `evals/*.yaml`. Each file is a YAML list of `{id, task, check}`. A check is one of `contains: text`, `regex: pattern`, `file_exists: path` or `exit_code: n`.

- `eval` is a verb of the `team_pack.tools` bundle, not a CLI subcommand: `{"type": "eval", "pack": "<pack dir>", "worker": "<worker agent id>", "timeout_s": 60}`. `timeout_s` is optional and bounds each eval.
- It sends each task to the worker as a `send` and checks the reply. It returns `pass_rate`, `passed`, `total`, `total_time_s`, `total_cost` (null when no eval reported a cost), `per_eval` and a one-line `text` such as `evals: 1/2 passed, pass rate 50%, time 0.3s, cost n/a`.
- It refuses a missing pack (`pack not found`), a pack with no evals (`pack has no evals`) and an empty worker. It creates no agent and writes nothing.
- `install` takes optional `worker` and `timeout_s` keys: `{"type": "install", "pack": "<pack dir>", "into": "<templates dir>", "worker": "<worker agent id>", "timeout_s": 60}`. With a worker, it runs the copied pack's evals on that worker after the copy and replies with the same report under `evals`. A pack with no evals replies `evals: {"skipped": "pack has no evals"}`. If the evals cannot be loaded it replies `evals: {"error": ...}`; the install still succeeds. A refused install runs no evals. Without `worker` the reply has no `evals` key. It reports only: enable is not gated on the evals.
- Not built yet: gating enable on a minimum pass rate; the measured record (`pack_record.rs`) built from these results, signed by the measuring shift, and published only through `publishable`; listing display of success rate and cost per task; evals run by real shifts rather than eval runs.
