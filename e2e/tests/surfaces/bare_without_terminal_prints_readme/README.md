# surfaces/bare_without_terminal_prints_readme

The binary is fantastic's command line. Fantastic's bare word (no words, no flags) is the terminal UI;
with stdout not a terminal (stdin `/dev/null`, then a pipe) it prints fantastic's readme and exits 0
within seconds: it draws nothing and starts nothing (no socket, no `.fantastic` in the folder).
The removed `--tui` flag is refused as unknown (exit 2). The with-a-terminal path is not covered here
(CI has no tty).

Mutation: make the binary start a kernel without a terminal and "nothing was started" fails.
