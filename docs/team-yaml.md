# team.yaml: a team's setup as one file

A `team.yaml` in a new team folder declares the team's own tree. It is folded into the
team's blueprint when the team's records are seeded into a fresh folder (`hydrate.rs`,
`blueprint_text`/`seed`); nothing is created or set at a boot. After that the records on disk are the team: editing
`team.yaml` changes nothing. [examples/team.yaml](../examples/team.yaml) is the two-repo battle setup.

| Key | Becomes |
|---|---|
| `credentials` | the `credentials` store's refs. Each field names an environment variable (`env: {api_key: FANT_AK}`); a value is refused. A github connector's ref holds `token` (personal: `env: {token: GH_TOKEN}`) or `app_id` + `private_key` (app; the key variable holds PEM text or a .pem path). Without a team.yaml the stock tree reads a personal token from `GH_TOKEN` (`github_credentials`). |
| `provider` | the `anthropic_backend` agent under the lead: `endpoint`, `caching` (`provider_cache`), `models`, `credentials` (a ref above), optional `id` (default `anthropic`) and `context_window`. |
| `connectors` | per repo, a `team_git.tools` and a `team_github.tools` agent: `repo` (`owner/name`, the allowlist), `clone` (a folder inside the team folder), `app` (a ref above), `author` (`name`, `email`), optional `remote`, `default_branch`, `api_url`, `directive`, `name`. The first repo's agents are `git` and `github`, the ids the role cards name; the next are `git_<name>` and `github_<name>`. |
| `worktrees` | the worktree root inside the team folder; the git connectors' `workspace_root` is a folder in it, and the workers' files and exec are rooted at it. |
| `roles` | `limits: {total, per_role}` on the roles registry. |
| `lead` | the lead's `model` (one of `provider.models`); its `provider_id` is the provider above. |

There are no tabs and no kernel list in it. A missing required field fails the seed naming it
(`team.yaml: missing provider.endpoint, lead.model`) and nothing is written.
