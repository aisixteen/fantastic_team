# e2e for the team binary

Built the way fantastic's `e2e/` is: one folder per test, a plug picks what answers the model, a `costs:` tier on every test.

```
python3 e2e/run.py --only-free          every free test; no network, no key; builds the binary with cargo
python3 e2e/run.py --only task/one_issue_to_draft_pr --bin PATH
python3 e2e/run.py --build github --bin PATH   the tests for the GitHub build (default: general)
python3 e2e/run.py --selftest           the harness proves itself, no binary
python3 e2e/run.py --list
```

Needs `python3`, `git` (with `git http-backend`) and `openssl` (a throwaway app key is made per run, never committed; without openssl the launcher says so and runs nothing). Exit code is non-zero when any test fails. A workflow (#46) calls `--only-free`; there is no workflow here, CI is manual.

## Tiers

| costs | what answers | status |
| --- | --- | --- |
| `free` | a scripted model (`backends/scripted.py`) and a fake forge (`helpers/forge.py`) | built |
| `turns` | a real model through a provider plug (aisixteen/fantastic#302), run by hand | not built |
| `gpu` | a local-model plug on the GPU box (aisixteen/fantastic#303) | not built |

## Layout

- `run.py`: the launcher.
- `backends/`: plugs. A test's `test.yaml` names one (`plug: scripted`); a test never names a model, an endpoint or a key.
- `helpers/`: `forge.py` (fake GitHub API plus git over smart HTTP from a bare repo), `team.py` (start, stop, `send`, wiring), `run_flow.py` (fake keys, boot, hand the lead its message), `verdict.py` (named checks).
- `tests/<group>/<id>/`: `README.md`, `test.yaml` (`costs`, `timeout`, `plug`, `build`: `any` (default), `general` or `github`; `run.py --build` runs the tests for `any` and that build), `team.yaml`, `script.json` (the scripted plug's turns), `probe.py` (`run(ctx)` returns the exit code).
- `runs/`: one folder per run, gitignored (it holds model output).

## Two builds

The general build (stock exec) and the GitHub build (`patches/github-exec.patch`, built by `scripts/build-github.sh`) share the `any` tests. `exec/general_allows_gh` runs on the general binary only; `exec/github_refuses_forge_calls`, `secrets/github_no_credential_in_worker` and `task/github_pr_through_connector` on the GitHub binary only.

## How a free test runs

The launcher makes a bare repo and a seeded clone, starts the fake forge (the repo is `acme/widgets`, one issue labelled `team-ready` by the maintainer with a `## SPEC` comment), starts the plug, and starts `fantastic_team --dir RUN` with fake keys in ITS environment only (the credential store reads them with `backend: env`). The probe then drives the team with `fantastic_team send`.

The anthropic backend sends no `tools` array: a model calls a tool by writing a `<call name="send">` element in its text. So the scripted plug streams Anthropic SSE whose text is that element; a script turn lists `calls`, and the plug renders them.

## Things the harness works around (found while building it)

These are facts about the binary at the commit this was built on, not decisions of the suite. Each workaround is in `helpers/team.py` or `helpers/run_flow.py` and goes away when the cause is fixed.

1. `fantastic_team --dir D` with a `team.yaml` in a fresh D does not leave a running tree: the start-up applies the tree in an in-process kernel whose seeded `store` is sealed, so nothing persists and the shell then stops with "no agent 'kernel_memory'". The harness starts once with no `team.yaml`, adds an open file store with `create_agent`, stops, writes `team.yaml`, and starts again.
2. `worker_files` and the puller's queue folder are sealed by default (`edge sealed by policy`); the harness opens them with `update_agent ingress_rule=allow_all_ingress`. The credential store, the github connector and the puller also need `callable_by`/`can_call` grants a `team.yaml` cannot write.
3. The binary never calls `prepare_sandbox`: the harness makes `work/`, `work/wt`, `work/tasks` and `sandbox/home` first. The worker's HOME is the relative `./sandbox/home`.
4. `directive` on a connector is not passed to its record (github refuses without it); the test's `team.yaml` puts it in `record`.
5. A model cannot pass `title` to `github pr_create`: `title` is a call-level field of the `<call>` grammar. The scripted model therefore stops after the push, and the probe sends `pr_create` itself, as the operator. The PR request is still what the github connector sends.

## Later

- `driver/rulebook_reply_and_escalation` waits for the group driver (#95).
- `dispatcher/digest_reaches_driver` waits for #58's delivery.
- the `turns` and `gpu` tiers.
