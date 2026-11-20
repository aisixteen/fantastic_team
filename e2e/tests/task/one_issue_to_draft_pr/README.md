# task/one_issue_to_draft_pr (free)

An open issue, end to end, on the scripted model and the fake forge.

Asserts, in order: the task registry's `pull` took the open issue as the waiting task `acme/widgets#1` (source `gh:acme/widgets#1`, no holder; a second pull skips it); the scripted model played every turn; the rows show `issue_view`, `worktree_add`, write, `worker_exec run`, `commit`, `push`, `pr_create` in order; the worktree exists with the file; the commit on the work branch is by the team bot and signed off; the forge's `main` is untouched and the branch reached it; git over HTTP carried the minted token; then the PR request of the model's `pr_create` (sent in the routing form, its `title` inside `<payload>`): exactly one `POST /pulls`, `draft: true`, head the work branch, base `main`, the task's title and body, the installation token in `Authorization`; the team stops on SIGTERM.
