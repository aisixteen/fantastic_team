# turns/task/worker_finishes_small_spec (turns, a real model)

The free test `task/one_issue_to_draft_pr` on a real model. Run by hand only, never in CI, and it needs `ANTHROPIC_API_KEY` (or `E2E_ANTHROPIC_KEY`) in the environment; without one it is skipped.

The lead gets `Do the task in tasks/queue/acme-widgets-1.md`, and the probe waits for a pushed work branch (up to 20 minutes). It asserts outcomes only, not the order of the model's calls: a work branch other than `main` with at least one commit; the forge's `main` unchanged; `greeting.txt` on the branch saying hello; the commit author is the team bot. Then the probe sends `pr_create` as the operator, as the scripted test does, and the forge records exactly one draft `POST /pulls` with that head.

It prints the token totals; `cost.json` in the run folder holds the record.
