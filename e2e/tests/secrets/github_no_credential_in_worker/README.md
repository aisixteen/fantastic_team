# secrets/github_no_credential_in_worker (free, build: github)

Extends `worker_never_sees_tokens` for the GitHub build. The worker runs `env`, `printenv`, lists and `cat`s files in its HOME, prints the credential variable names, then tries `gh auth token` and `git config --global credential.helper store`.

Asserts: HOME is `./sandbox/home`; no model request or the team's output holds the model key, the PEM or the token; the worker's environment names no `E2E_*` variable; the two forge attempts are refused with `command not allowed`.
