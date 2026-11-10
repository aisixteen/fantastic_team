# gpu_cli/precreated_1_2

The gpu twin of `cli/precreated_1_2` (#237): a lead and two workers (`w1`, `w2` under `lead`) laid as record
files under fixed ids before the first start, each worker able to call the exec agent. The lead is asked, in plain words,
to have `w1` fix adder.py and `w2` muler.py, committed, and to check the test. A real model answers; there
is no script.

Waits for the lead's `done` (or the test's `timeout`), then checks, as the cli test does: solved; the same
3 agents under the same parents, nothing extra. No agent is spawned, so this test does not depend on
aisixteen/fantastic#467.

Needs `VLLM_ENDPOINT`, `FANTASTIC_AI_MODEL`, `VLLM_KV_SLOTS` (1 or 2) and `VLLM_KV_OVERFLOW`; without one
the test is `SKIP`. See `e2e/README.md`, "The gpu tier".
