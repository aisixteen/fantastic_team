# task/github_auth_by_env_ref (free)

Issue #303: GitHub auth is configuration. The ref of the `github` connector holds `token` (personal) or `app_id` + `private_key` (app); with no credential the team works local only. App mode is covered by `task/github_merge_through_connector` and `task/github_pr_through_connector`; this test covers the other two.

Phase personal: the credential store ref is `env: {token: E2E_GH_TOKEN}` and the variable is set (the fake forge's token stands in for a PAT). `reflect` says `auth.mode: personal`. A worktree is made and committed; `push` and `pr_create` go through. Asserts: the branch is on the forge, the git request was authorized, exactly one draft PR was posted from the branch with the token as the Authorization, no installation lookup and no token mint reached the forge, and the token is in no reply and not in the team's log.

Phase local: the same ref, the variable unset. The team boots; `reflect` says `auth.mode: local` with the note. A worktree and a commit work. `push`, `pr_create`, `issues` and `comment` each answer `no GitHub credential defined: local work only`, never a 401, and not one request reaches the forge.

Mutation: give the personal phase the app ref (`github_mode="app"`) and the PR check fails.
