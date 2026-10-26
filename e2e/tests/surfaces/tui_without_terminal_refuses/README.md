# surfaces/tui_without_terminal_refuses

`--tui` with stdout not a terminal (stdin /dev/null, then a pipe) exits 2 within
seconds with "--tui needs a terminal" and the alternatives (no flags, `--port N`);
it draws nothing and starts nothing (no socket, no tree in the dir). The
with-a-terminal path is not covered here (CI has no tty).

Mutation: remove the `is_terminal` guard in `crates/team/src/main.rs` and the
exit-2 and message checks fail.
