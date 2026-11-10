# secrets/github_no_credential_in_worker (free)

Extends `worker_never_sees_tokens`. The worker runs `env`, `printenv`, lists and `cat`s files in its HOME and prints the credential variable names.

Asserts: HOME is `./sandbox/home`; no model request or the team's output holds the model key, the PEM or the token; the worker's environment names no `E2E_*` variable.
