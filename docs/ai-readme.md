# fantastic_team: README for AIs

You are reading the text of the `fantastic_team` binary, the team's start-up. Use only what is written here. Every command below exists in this repo.

## 1. Build and check

- Build: `cargo build --workspace`. The binary is `fantastic_team`.
- Check: `fantastic_team ai-readme` prints this file. `fantastic_team readme` prints fantastic's readme (the command line is fantastic's).
- Tests: `cargo test --workspace`. Secrets check: `scripts/check-secrets.sh .`
- e2e: `python3 e2e/run.py --only-free` runs the binary on a scripted model and a fake forge (no network, no key). See `e2e/README.md`.

## 2. Start the team

Run it from the directory the team lives in (`cd` there first).

- The command line is fantastic's, unchanged (`fantastic readme` documents it): `--headless` (no screen, no port), `webui [--port N] [--host ADDR]` (serves the team page; the team's own word, refused with `canvas`), `canvas [--port N] [--host ADDR]` (serves fantastic's canvas), `ai <backend> …`, `run …`, `system`, `mcp`, `send`, `import`, `migrate-home`, `readme`. It runs in the directory it is started in: there is no `--dir`. `--tui`, a bare `--port`, `--instructions` and `--resend-instructions` are gone; fantastic refuses them.
- `fantastic_team packs list|show|updates|publish|fetch|submit|report|takedown|page|pack|install|eval --key value ...` sends a pack verb to the running team (attach-only, like `send`): `list [--query Q]`, `show --name N [--version V]`, `updates [--opt-in true|false]`, `publish --pack DIR --url HTTPS_URL --signing-key FILE`, `pack --worker W --out DIR`, `install --pack DIR --into DIR [--worker W] [--timeout-s N] [--parent-id AGENT [--approve] [--allow-code]]`, `eval --pack DIR --worker W [--timeout-s N]`. The hosted-registry verbs are `fetch --name N --hash H --stage DIR [--version V]`, `submit --pack DIR --url HTTPS_URL --signing-key FILE`, `report --name N --version V --reason TEXT`, `takedown --name N --version V --signing-key FILE` and `page`.
- `fantastic_team setup github` and `fantastic_team ai-readme` are the team's own first words, with `packs`.

Nothing is read from a config file: the team is the agent records in `.fantastic/agents/`. Make them with a recipe and `send` (`create_agent` payloads, under ids you choose; see section 3). The team uses no blueprints.

The web UI's home screen is the dispatcher (agent `dispatcher`): a sidebar of
the team's groups. In each group, every worker is followed by a `subagents (N)`
expander, and its subagents (kernel agents whose parent is that worker) show
under it only when expanded. A click on a worker or a subagent opens that agent
as a full-screen chat, and Back returns to the sidebar with its scroll and
expansion kept. The dispatcher's view is made at the first start and is not put on
the canvas (a canvas lists sidecars only). It reads no secrets and sends no work. The `sidebar` verb answers the
list; `open` answers the chat for one agent (`view: chat`, `target`, `fullscreen`).
`control {node, action, confirm?}` sends start, pause or stop to a worker (any other action is refused); stop needs `confirm: true`, and the answer is `{sent, error?}`.

The right pane fills the height beside the tree: the chat's transcript scrolls inside it and the chat has no fixed height. An `embed {id}` row in the chat's history is drawn as a card where the row sits (a sandboxed iframe of the host agent's `get_html`; `unembed` removes it, the X hides it in the page only); the page's leg may call that host's panel verbs plus `get_html` and `watch`, granted at `open` and again after a turn, never `*`. When a canvas named after the node exists (`canvas_<id>`, what `new_canvas {name: <id>}` makes), the `open` reply carries `canvas {id, url}` and the pane shows a Canvas button that swaps the chat for `url` (`/canvas_page/file/index.html?canvas=<id>`) in an iframe. The `agents` rows carry `has_children`, so a subtree and the whole tree column fold (remembered in the browser). Up to 600 px wide the list and the pane are two snap panels side by side: opening a node scrolls the pane into view and Back scrolls the list back. 

## 3. Records

There is no `team.yaml`. Every key it had is a field of an agent record, set with `create_agent` / `update_agent` through `fantastic_team send`, with a `create_agent` payload sent to the root (`kernel_state`) under an id you choose. A grant is written on both ends: the new agent's `can_call` and the other agent's `callable_by` (`update_agent` replaces a whole map, so restate the keys it has). The example: `examples/create_agent/worker_only.sh`. The app makes its own team the same way (`crates/team/src/new_team.rs`): `lead`, and under it `queue`, `github`, `board`, `roles`, `tasks`, `worker_exec` and `worker_files`. The role cards grant those tools by id, so keep the ids.

- Templates are the `tiers` field of the `agentic_resources` (roles registry) record, plus each AI record's own fields: `handler_module`, `model`, `endpoint`, `credentials` (a store and a ref), prices, `can_call`, `callable_by`.
- The first agent is an ordinary AI record. Its directive is its `system_prompt_file` (read through its `file_bridge_id`), or a message sent with `send`. The caller defines the agents: nothing is created behind the records.
- Connectors, the queue, the memory store and the credential store are agent records with their own fields (`team_git.tools` with `repo_dir`, `workspace_root`, `remote`, `default_branch` and a `directive`; `team_github.tools`; `ram_store.tools`; `sqlite_store.tools`; `credential_store.tools`).
- `groups` is a field of the dispatcher (name → worker ids); the board's `team_groups` adds each group's `driver`; the cadencer's `group_settings` its `cadence`, `stall_after`, `quiet` and `enabled`. The board's `repos` (`owner/name`) and `github` name what it caches and through which connector.
- The roles registry record: `role_dirs` (the folders of role cards, in `roles/` of the repo: each card names a `tier` that must be a key of `tiers`), `tiers` and `limits` (`total`, `per_role: {role: n}`). It is on with `FANTASTIC_AGENTIC_RESOURCES=1`.
- A repo the puller serves is in its record's `repos`; its settings are the `repo_config` field, keyed by `owner/name`.

Keys are never written in a record. Name the credential ref, set the environment variables the credential store reads in your shell, and tell the user which names to set.

## 4. Run it on a remote box

Remote scripts run from the laptop, in `scripts/`:

- `remote-add.sh <name> <host>:<dir> [--bin PATH] [--port N] [--keys "NAME …"] [--dry-run]`: once, to save the remote.
- `remote-start.sh <name> [--port N] [--dry-run]`: start it detached. The values of the `--keys` variables are read on the laptop and sent over SSH stdin. They never appear in an argument list or a file.
- `remote-ls.sh [--dry-run]`: every remote and its state.
- `remote-attach.sh <name> [--port N] [--dry-run]`: the web UI through a tunnel, at http://127.0.0.1:N. The terminal UI is not supported over the tunnel yet.
- `remote-logs.sh <name> [-f] [-n LINES]`: the team's log.
- `remote-stop.sh <name> [--now] [--yes] [--dry-run]`: drains the team (SIGTERM). `--now` kills it. It asks first unless `--yes`.

## 5. Give it work

Work comes from GitHub issues. A maintainer adds the label `team-ready` to an issue, and a `## SPEC` comment is written by the maintainer or the app. The puller (`crates/team/src/puller.rs`) takes only issues whose latest `team-ready` event is by the maintainer. It writes `queue/<repo>-<n>.md` and a pending `.state` file. It never overwrites a state file. Read `queue/` to see what was taken.

## 6. Rules

- Never put a key, token or password in a file, an argument, a commit or a log. Keys come from the credential store. Run `scripts/check-secrets.sh .` before you commit.
- Never push to the default branch (`main`), and never force-push. Workers push work branches through the git connector only.
- Merging is the maintainer's decision.
- Ask before stopping a running team. `remote-stop.sh` asks unless `--yes` is given.

## 7. Troubleshooting

- `fantastic: the UI needs a terminal`: where there is none, run `fantastic_team webui` or `fantastic_team --headless`.
- `fantastic: unknown \`--tui\``: the team's own surface flags are gone; use fantastic's words (`webui --port N`, `canvas --port N`, `--headless`).
- `… is not set on this laptop`: export the variable named in the remote's keys, then run `remote-start.sh` again.
- `no remote \`name\``: add it first with `remote-add.sh`.
- `already running · pid …`: the team is up. Use `remote-stop.sh` before starting again.

## 8. Pack evals

A worker pack may carry sample tasks in `evals/*.yaml`. Each file is a YAML list of `{id, task, check}`. A check is one of `contains: text`, `regex: pattern`, `file_exists: path` or `exit_code: n`.

- `eval` is a verb of the `team_pack.tools` bundle, on the agent `pack` (the list, show and updates verbs are on `pack_listing`; both are in every team). From the shell, with the team running: `fantastic_team packs eval --pack <pack dir> --worker <worker agent id> [--timeout-s 60]`. The same call over `send` or MCP is `{"type": "eval", "pack": "<pack dir>", "worker": "<worker agent id>", "timeout_s": 60}`. `timeout_s` is optional and bounds each eval.
- Which workers it can reach: the `pack` agent may call the agents its record's `can_call` names, and each of them must list `pack` in its `callable_by`, or the call is refused (`pack may not call 'send' on <id>`). This holds for `install --worker` too.
- It sends each task to the worker as a `send` and checks the reply. It returns `pass_rate`, `passed`, `total`, `total_time_s`, `total_cost` (null when no eval reported a cost), `per_eval` and a one-line `text` such as `evals: 1/2 passed, pass rate 50%, time 0.3s, cost n/a`.
- It refuses a missing pack (`pack not found`), a pack with no evals (`pack has no evals`) and an empty worker. It creates no agent and writes nothing.
- `install` takes optional `worker`, `timeout_s` and `min_pass_rate` keys: `{"type": "install", "pack": "<pack dir>", "into": "<templates dir>", "worker": "<worker agent id>", "timeout_s": 60, "min_pass_rate": 0.5}`. With a worker, it runs the copied pack's evals on that worker after the copy and replies with the same report under `evals`, plus an advisory `enable: {allowed, min_pass_rate, reason}` verdict. `min_pass_rate` is a finite number in `0..=1`; any other value is ignored and the default `1.0` (every eval must pass) applies. `allowed` is true when the pass rate is at or above `min_pass_rate`, with the reason `pass rate N% is below the required M%` otherwise. A pack with no evals replies `evals: {"skipped": "pack has no evals"}` and `enable.allowed: true` (nothing to gate on). If the evals cannot be loaded it replies `evals: {"error": ...}` and `enable.allowed: false` (fail closed); the install still succeeds. A refused install runs no evals. Without `worker` the reply has no `evals` and no `enable` key. The install never fails or rolls back on the verdict, so the verdict is advice for the caller. `install` also takes `parent_id`, `approve` and `allow_code` for a pack that carries a `blueprint/` folder: with `parent_id` it builds the worker under that agent through `user.import_subtree`; without `approve` it replies the permission preview under `approval` and creates nothing; with `approve: true` it replies `imported` and `ids` and `installed_as: worker`. Without `parent_id` no agent is created.
- Hosted registry (`pack_hosted.rs`, verbs on `pack_listing`). `registry_url` is an optional https field of the `pack_listing` record: when set, `list`, `show`, `updates`, `page` and `browser` read the signed index from that url instead of `index_path` (every entry is still verified against `trusted_keys`), and `submit`, `report` and `takedown` post to it. Without it nothing reaches the network. An http `registry_url` is refused when the record is read. No call carries credentials; who may post to the registry is the registry operator's concern.
- `fetch {name, hash, stage, version?}` finds the entry by name and pinned hash in the signed index, refuses a revoked one, downloads the entry's https `url` (one JSON document `{"files": {"<path>": "<text>"}}` that includes `pack.yaml`; 10 s timeout, 16 MiB cap), and writes it under `stage` only after the manifest's hash equals the pin, the recomputed content hash equals the pin and the secrets scan passes. A download whose hash differs is refused and nothing is written. It installs nothing: run `install --pack <stage>` next.
- `submit {pack, url, signing_key, grants?}` reviews the pack folder like `publish` (manifest, content hash, scan), but holds it: any scan finding replies `{listed: false, status: "held", findings: [{path, rule}]}` and sends nothing. A clean pack is signed and POSTed as `{entry, files}` to `registry_url`; the reply is `{listed: true, ...}`. Without `registry_url` it is the error `no registry_url`. The scan refuses on any finding, so a held pack and a refused `publish` have the same cause; `submit` reports it as a status instead of an error.
- `report {name, version, reason}` POSTs `{name, version, reason}` to `<registry_url>/report`; the reason is 1 to 2000 characters. `takedown {name, version, signing_key}` revokes the version in the local index (the entry is signed again, so `revoked` is covered by the signature) and, with `registry_url`, POSTs `{takedown: {name, version}, index}` to it so the hosted index is revoked too; if the push fails the reply carries an `error` and `pushed: false` while the local revocation stands. `page` replies `{html}`, a static, escaped listing page (ports, permissions, evals, measured record, price, installed and revoked badges). `browser {selected?, opt_in?}` replies the view-model of the Packs panel in the web dispatcher: cards, the selected pack's permission preview, and updates with their permission diff only when `opt_in` is true. Hosting the listing page at a public address is not built.
- Not built yet: enforcing the verdict where a worker is created from a template; the measured record (`pack_record.rs`) built from these results, signed by the measuring shift, and published only through `publishable`; listing display of success rate and cost per task; evals run by real shifts rather than eval runs.
