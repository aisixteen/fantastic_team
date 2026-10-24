# github connector (`team_github.tools`)

One GitHub App, many repos. The connector is optional: bind it in `team.yaml` with a `directive`, which says what the lead and its workers may do with it.

Record fields: `app` (`{credentials: {store, ref}}`: the ref holds `app_id` and `private_key`, a PEM RSA key, bound to `api_url`'s host), `repos` (the allowlist, `owner/name`), `api_url` (default `https://api.github.com`), `git` (a git connector agent whose worktrees push uses), `directive`, `timeout_s`.

Verbs: `push`, `pr_create`, `issues`, `issue_view`, `issue_create`, `issue_state`, `label_events`, `comment`, `label`, `pr_status`, `whoami`, `directive`, `reflect`.

`issue_create {repo, title, body, project?}` opens an issue as the app. Given `project` (a ProjectV2 node id), it also adds the issue to that board. `issue_state {repo, number, state, sender?}` opens or closes an issue. Closing an issue labelled `epic` is refused unless `sender` is `maintainer`; reopening is never refused. A pull request number is refused.

`label_events {repo, number}` returns only the `labeled` and `unlabeled` events of an issue as `{event, label, actor, created_at}`, oldest first. It is how a reader checks who applied a label; an unknown or missing issue is a GitHub error, named, with no token in the reply.

Rules: a repo outside `repos` is refused before any request. Pushes go through the named git agent's own branch rules (never the default branch, main or master, never a force). Pull requests are drafts only. The key is read from the credential store at each call; a JWT and a token scoped to the one repo are minted per call and never returned, stored or logged.

Register your own app with permissions: contents read/write, pull requests read/write, issues read/write, checks read, metadata read. Store `app_id` and `private_key` under one ref in the credential store, bound to `api_url`.
