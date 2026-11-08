# team_git — the git connector

One agent is one local clone and one remote. It makes worktrees, commits
and pushes work branches. Any remote works: SSH (`git@host:org/repo.git`,
`ssh://…`), HTTPS (any host), or a local path, public or private.

Record fields: `repo_dir` (the clone), `workspace_root` (optional; every
worktree path must be inside it), `remote` (a remote name or a URL), `default_branch`,
optional `credentials` (a credential store ref, used for HTTPS only),
`username`, `author_name` and `author_email` (together), `ssh_command`,
`timeout_s`.

Verbs: `worktree_add`, `worktree_remove`, `commit`, `push`, `fetch`,
`diff_stat`, `status`, `reflect`, `boot`.

Where the worktrees go:
- No `workspace_root` on the record: `<default folder>/<agent id>`, made at
  the first `worktree_add`. The default folder is the system temp folder's
  `fantastic_team/worktrees`, or the binary's `--worktrees DIR`.
- Another folder: `update_agent` of this record with `workspace_root`, and
  only after the user agrees to that folder. Ask first.

Rules:
- Paths stay inside `workspace_root`.
- `push` refuses the default branch, `main` and `master`; it never forces.
- An HTTPS token is read from the credential store at each fetch and push,
  and it never leaves the agent: not in argv, a URL, a git config or a reply.
- SSH uses the host's configured keys with `BatchMode=yes`.
