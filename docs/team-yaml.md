# team.yaml: a team's setup as one file

A `team.yaml` in a new team folder declares the team's own tree. `fantastic_team` applies it
once, at the first start of a fresh folder (the records are seeded with the rest of the team's
fixed agents, nothing is created at a boot). After that the records on disk are the team: editing
`team.yaml` changes nothing. [examples/team.yaml](../examples/team.yaml) is the two-repo battle setup.

| Key | Becomes |
|---|---|
| `credentials` | the `credentials` store's refs. Each field names an environment variable (`env: {api_key: FANT_AK}`); a value is refused. |
| `provider` | the `anthropic_backend` agent under the lead: `endpoint`, `caching` (`provider_cache`), `models`, `credentials` (a ref above), optional `id` (default `anthropic`) and `context_window`. `default_model` is kept in its memory store at the first start. |
| `connectors` | per repo, a `team_git.tools` and a `team_github.tools` agent: `repo` (`owner/name`, the allowlist), `clone` (a folder inside the team folder), `app` (a ref above), `author` (`name`, `email`), optional `remote`, `default_branch`, `api_url`, `directive`, `name`. The first repo's agents are `git` and `github`, the ids the role cards name; the next are `git_<name>` and `github_<name>`. |
| `worktrees` | the worktree root inside the team folder; the git connectors' `workspace_root` is a folder in it, and the workers' files and exec are rooted at it. |
| `roles` | `limits: {total, per_role}` on the roles registry. |
| `lead` | the lead's `model` (one of `provider.models`); its `provider_id` is the provider above. |

There are no tabs and no kernel list in it. A missing required field fails the start naming it
(`team.yaml: missing provider.endpoint, lead.model`) and nothing is written.

## The dry check

`fantastic_team check`, run in the team folder before the first message, applies a fresh folder
and then checks, with no model call: the provider's `caching` is `provider_cache`, the lead's
provider and model, each git connector's clone and worktree folder are inside the team folder, each
github connector allows its repo and answers `whoami` for the credential, the workers' tools are
rooted at the worktree root and the roles' limits are the file's. Each line prints `ok` or `FAIL`;
any failure exits non-zero. The credentials are read from the environment, so export them first.
