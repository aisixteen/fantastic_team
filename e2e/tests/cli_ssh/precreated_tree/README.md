# cli_ssh/precreated_tree

The ssh twin of `cli/precreated_tree` (#242, #252): one task split across the local kernel (the master) and a remote one in a Podman box, wired the weak way.

The lead, `s1` with `w1`, `s2` with `w2`: all on the master (made before the task). `s1` has `w1` fix adder.py; the remote agent (the work program in the box) sends `s2` its work and watches it, and `s2` has `w2` (on the master, its tools the box's exec agent) pull, fix muler.py and push. The lead reads `w2`'s history back.

Checks: The local checks of `cli/precreated_tree`; every agent told a task asked the model, all on the master; the work program exited 0; `w2`'s history (its push) came back.

The wiring, the shared checks (`Box.end`) and what the twin needs (Podman, else `SKIP`; builds the Linux binary once per source tree) are the twins' in `e2e/README.md` ("The weak wiring", "The twins"). The scripted plug's `script.json` is written by `python3 -m helpers.ssh_twins --write` in `e2e/`; the fake forge, no network.
