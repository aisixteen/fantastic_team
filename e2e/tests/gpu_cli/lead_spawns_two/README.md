# gpu_cli/lead_spawns_two

The gpu twin of `cli/lead_spawns_two` (#237): a lead with a roles registry (fantastic's
`agentic_resources`, `FANTASTIC_AGENTIC_RESOURCES=1`; the role bundle `cli/spawn_bundles/worker/bundle.yaml`,
its model the lead's) is asked, in plain words, to fix the failing test with two workers (one file each),
to commit, and to delete the workers it spawned. A real model answers; there is no script.

Start: `--headless`, then the lead and `roles` made with `send` + `create_agent` under the fixed ids
`lead` and `roles`. The tier carries the model's `credentials`, like any AI that needs a key.

Waits for the lead's `done` (or the test's `timeout`), then checks, as the cli test does: solved; two
`spawn` calls; the records are the ones it started with (both workers deleted); the registry counts none
running. The model's own path (turn counts, which call first) is only reported.

**Depends on aisixteen/fantastic#467 (reach race).** Until it merges, the workers this test spawns
carry the tier's `credentials` and are expected to fail their first turn, so `solved` fails: that is
the open bug, not this test. A run is not a regression for the suite until #467 is in the binary.

Needs `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW`; without one
the test is `SKIP`. See `e2e/README.md`, "The gpu tier".
