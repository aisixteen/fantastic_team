# exec/github_refuses_forge_calls (free, build: github)

The GitHub build (`patches/github-exec.patch`) refuses forge calls. The scripted worker runs `gh --version`, `git push origin HEAD` and `curl https://api.github.com/user`, then `echo`.

Asserts: each of the three is refused with `command not allowed` before it runs, the ordinary `echo` still runs. Mutation: give the worker's exec the general build's pattern (`.*`) and the refusals are missing. The harness reads the GitHub build's pattern from `patches/github-exec.patch` (`stock.github_allowed()`), so the binary should be built from the patched tree.
