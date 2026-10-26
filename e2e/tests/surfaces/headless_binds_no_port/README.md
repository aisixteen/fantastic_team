# surfaces/headless_binds_no_port

With no surface flag the team runs headless: it binds no TCP port, draws no
terminal UI, prints no canvas URL, and a verb (`list_agents`) still works over
`.fantastic/cli/mcp.sock`. A control starts a second team with `--port N` and
checks the same listener probe then sees N (so "no listener" cannot pass
vacuously). Needs `lsof` or `/proc/net/tcp`; without both it prints SKIP and exits 0.

Mutation: start the first team with `--port N` and "it binds no TCP port" fails.
