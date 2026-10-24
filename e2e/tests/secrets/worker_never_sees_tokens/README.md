# secrets/worker_never_sees_tokens (free)

The same fixture with fake keys: a model key, a throwaway app PEM and the installation token the fake forge mints. The scripted worker runs `env`, `printenv` and a shell that lists and `cat`s files in its HOME; it then makes, commits and pushes a worktree; and `cat`s the git configs.

Asserts: the worker's HOME is `./sandbox/home`; no model request, the team's stdout and stderr, or any file of the run (records, rows, repos, worktree) holds a key, the PEM or the token; the model key is sent only as `x-api-key`; the token reaches the forge in the git push only and is in neither `.git/config`; `scripts/check-secrets.sh` finds nothing in the run.

Mutation: make the script `cat` the PEM and "no model request holds the app key" fails.
