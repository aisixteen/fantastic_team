# turns/secrets/real_key_stays_in_x_api_key (turns, a real model)

The free test `secrets/worker_never_sees_tokens` with a real model and the real key as a needle. Run by hand only, never in CI. It needs `ANTHROPIC_API_KEY` (or `E2E_ANTHROPIC_KEY`) in the environment; without one it is skipped.

The real key is read from the environment at run time and goes only into the team's environment (its credential store). It asserts: no model request holds the key, the app key chunk or the minted token; the relay saw the key only as `x-api-key`; the worker's HOME is the sandbox where it printed one; no team log and no file of the run holds the key (outside `secrets/` and `cost.json`, which is checked on its own); the repo's `scripts/check-secrets.sh` finds nothing in the run.

The model's call order is not asserted. The test waits up to 20 minutes for a pushed branch or a plug error.
