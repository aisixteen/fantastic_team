# exec/general_allows_gh (free, build: general)

The general build's worker exec is stock. The scripted worker runs `gh --version` and a shell that names `gh`.

Asserts: no reply holds `command not allowed` or `unauthorized`, and the shell command ran. Mutation: patch the build with `patches/github-exec.patch` and the test fails (that is the GitHub build's `exec/github_refuses_forge_calls`).
