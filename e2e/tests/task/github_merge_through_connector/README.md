# task/github_merge_through_connector (free, build: any)

Issue #273: the probe sends the team's own verbs (the scripted lead cannot chat: `lead` is a bare provider on the pinned fantastic): `git` worktree_add and commit, `github` push for three branches (green, red, pending checks), then opens a draft PR for each and sends `merge`.

Asserts: the green draft PR is marked ready, then squash-merged (the fake forge's main gains one commit with the PR's file, and the PR is closed); the red and the pending PR are refused naming the check, with no merge request reaching the forge and main unchanged; no reply, model request or team log holds the installation token. The fake forge keeps the PRs, serves check runs per head branch (`forge.checks`), the GraphQL ready mutation and `PUT /pulls/N/merge`.
