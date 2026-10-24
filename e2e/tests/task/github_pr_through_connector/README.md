# task/github_pr_through_connector (free, build: github)

The one-issue task on the GitHub build: the lead pulls the issue, makes a worktree, commits, and pushes through the github connector; the probe then sends `pr_create`.

Asserts: the fake forge gets one draft PR from the work branch, opened with the minted installation token (the app); the worker's env, every model request, the team's output and `.git/config` never held the token or a key.
