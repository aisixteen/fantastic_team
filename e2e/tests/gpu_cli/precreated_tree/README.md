# gpu_cli/precreated_tree

The gpu twin of `cli/precreated_tree` (#237): a lead, sub-leads `s1` and `s2`, and a worker under each
(`w1` under `s1`, `w2` under `s2`), all made with `create_agent` under fixed ids. The lead is asked, in
plain words, to have the sub-leads get adder.py and muler.py fixed through their workers, committed. A real
model answers; there is no script.

Waits for the lead's `done` (or the test's `timeout`), then checks, as the cli test does: solved; the tree
is unchanged. No agent is spawned, so this test does not depend on aisixteen/fantastic#467.

Needs `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW`; without one
the test is `SKIP`. Five agents share at most 2 live conversations on the shared GPU: see `e2e/README.md`,
"The gpu tier".
