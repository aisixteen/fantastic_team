# cli/team_yaml_dry_check

A team set up from a preset yaml (#292), checked by the dry check with no model call.

- A folder whose `team.yaml` lacks `provider.endpoint`: `fantastic_team --headless` and
  `fantastic_team check` both fail naming it, and nothing is written (no `.fantastic`).
- A whole `team.yaml` (the provider with `caching: provider_cache`, the credential refs as
  environment variable names, one connector pair against the fake forge, the worktree root, the
  roles' limits, the lead's provider and model): `fantastic_team check` applies the fresh folder
  and prints `ok` for the caching, the git roots under the team folder, the worktree root, the
  limits and the github `whoami` (the forge saw the app's `GET /app`). The team then starts on those
  records. The scripted model served no request, and neither `team.yaml` nor `.fantastic` holds a
  key.

Mutation: change `caching` to `none` in the yaml and the "caching" check fails; point `clone` outside
the folder and "git roots" fails; remove the forge's `/app` route and "whoami" fails.
