# gpu_cli/leads_spawn_leads

The gpu twin of `cli/leads_spawn_leads` (#237): a lead with a bundles registry and the cards
`sublead` and `worker` (`cli/leads_spawn_leads/bundles/`) is asked, in plain words, to fix the failing test
through two sub-leads, each of which spawns one worker; whoever spawns an agent deletes it. A real model
answers; there is no script.

Waits for the lead's `done` (or the test's `timeout`), then checks, as the cli test does: solved; 4
`spawn` calls; no spawned agent left; no orphans; the records are the ones it started with; the registry
counts none running. The model's own path is only reported.

**Depends on aisixteen/fantastic#467 (reach race).** Until it merges, the workers this test spawns
carry the tier's `credentials` and are expected to fail their first turn, so `solved` fails: that is
the open bug, not this test. A run is not a regression for the suite until #467 is in the binary.

Needs `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW`; without one
the test is `SKIP`. The shared GPU keeps at most 2 conversations live, so a lead, two sub-leads and two
workers rotate through the slots (`VLLM_KV_OVERFLOW=evict`) or are refused (`reject`): see
`e2e/README.md`, "The gpu tier".
