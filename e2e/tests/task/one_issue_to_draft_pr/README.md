# task/one_issue_to_draft_pr (free)

A `team-ready` issue with a `## SPEC` comment, end to end, on the scripted model and the fake forge.

Asserts, in order: the puller wrote `queue/acme-widgets-1.md` and a pending `.state`; the scripted model played every turn; the rows show read, `worktree_add`, write, `worker_exec run`, `commit`, `push` in order; the worktree exists with the file; the commit on the work branch is by the team bot and signed off; the forge's `main` is untouched and the branch reached it; git over HTTP carried the minted token; then the PR request: exactly one `POST /pulls`, `draft: true`, head the work branch, base `main`, the task's title and body, the installation token in `Authorization`; the team stops on SIGTERM.

`pr_create` is sent by the probe, not the model (see `e2e/README.md`, item 5).
