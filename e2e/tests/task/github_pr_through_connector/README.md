# task/github_pr_through_connector (free)

The one-issue task: the lead pulls the issue, makes a worktree, commits, pushes and sends `pr_create` through the github connector.

Asserts: the fake forge gets one draft PR from the work branch, opened with the minted installation token (the app); the worker's env, every model request, the team's output and `.git/config` never held the token or a key.
