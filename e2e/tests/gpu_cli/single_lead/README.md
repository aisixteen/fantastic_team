# gpu_cli/single_lead

The gpu twin of `cli/single_lead` (#237): one lead solves the shared fixture (a git repo with one
failing test; solved = the test fails on the start commit, passes on HEAD, the work tree is clean) on
a real model, the shared vLLM server, with no script.

- A, the lead made by hand: `fantastic_team --headless`, then `send` of `create_agent` for the lead,
  under the fixed id `lead`, with the vllm record (`helpers/cli.py make_agent`).
- B, the recipe: `--headless ai vllm -p PORT --host H --kv-overflow O` (the root AI `ai`), then `send`
  of `update_agent`: the server's URL as given (the recipe only knows `http://H:P/v1`), the model, the
  budgets, `kv_slots` (never read from the shared server), the credential ref and the exec grant.

The probe hands the lead the task in plain words (`helpers/gpu_cli.py TASKS`), waits for the lead to
answer `done` (or the test's `timeout`), then checks: solved, and the records are the ones the run
started with. Which tool it called first and how many turns it took are not asserted; the log has a
`report:` line for them.

Needs `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW`; without
one the test is `SKIP`. `VLLM_API_KEY` is optional. See `e2e/README.md`, "The gpu tier".

Run: `python3 e2e/run.py --only gpu_cli/single_lead --bin ABS_PATH`
