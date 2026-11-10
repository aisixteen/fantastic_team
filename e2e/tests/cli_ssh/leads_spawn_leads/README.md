# cli_ssh/leads_spawn_leads

The ssh twin of `cli/leads_spawn_leads` (#242, #252): one task split across the local kernel (the master) and a remote one in a Podman box, wired the weak way.

The lead spawns `s1` and `s2` on the master's roles registry. `s1` spawns its worker, which fixes adder.py and pushes. The remote agent sends `s2` its work and watches it (`s2` is minted by the spawn: opened to the leg at that turn); `s2` spawns its worker (on the master; its tools are the box's exec agent), which pulls, fixes muler.py and pushes. Each sub-lead deletes its worker, the lead deletes both sub-leads.

Checks: The checks of `cli/leads_spawn_leads`: 4 spawns, all here, none left, no orphan; the work program exited 0; the box clone's last commit is the worker's fix.

The wiring, the shared checks (`Box.end`) and what the twin needs (Podman, else `SKIP`; builds the Linux binary once per source tree) are the twins' in `e2e/README.md` ("The weak wiring", "The twins"). The scripted plug's `script.json` is written by `python3 -m helpers.ssh_twins --write` in `e2e/`; the fake forge, no network.
