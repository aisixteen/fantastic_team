# cli_ssh/lead_spawns_two

The ssh twin of `cli/lead_spawns_two` (#242, #252): one task split across the local kernel (the master) and a remote one in a Podman box, wired the weak way.

The lead spawns `w1` and `w2` on the master's bundles registry. `w1` fixes adder.py, pushes and reports to the lead with a plain `send` (the last turn of its lane: the send waits until the lead is idle, so the lead, awaiting, takes the report as a turn and deletes `w1` at once; a `done` after the send would never be served). The remote agent sends `w2` its work and watches it (`w2` is minted by the spawn: the operator opens it to the bridge leg when the script reaches that turn, `grant_leg`); `w2`'s tools are the box's exec agent (pull, fix muler.py, push). The lead deletes `w2` at the end.

Checks: The checks of `cli/lead_spawns_two`: 2 spawns, both here, the registry counts nothing running at the end; `w1`'s report reached the lead and gave it a turn (#247, #264); the work program exited 0; the box clone's last commit is `w2`'s fix.

The wiring, the shared checks (`Box.end`) and what the twin needs (Podman, else `SKIP`; builds the Linux binary once per source tree) are the twins' in `e2e/README.md` ("The weak wiring", "The twins"). The scripted plug's `script.json` is written by `python3 -m helpers.ssh_twins --write` in `e2e/`; the fake forge, no network.
