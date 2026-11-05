# cli_ssh/lead_spawns_two

The ssh twin of `cli/lead_spawns_two` (#242): one task split across the local kernel and a remote one in a Podman box, wired the weak way.

The lead spawns `w1` on the local roles registry and `w2` on one it makes on the remote kernel (the role card written in the box). `w1` fixes adder.py and pushes, then `w2` pulls, fixes muler.py and pushes; the lead deletes each where it lives, and the remote registry.

Checks: The checks of `cli/lead_spawns_two`, split: 1 spawn here, 1 there, both registries count nothing running at the end; the box clone's last commit came back through the bridge (a spawned agent's history is its spawner's to read: the registry grants the spawner pause/resume/stop on it).

The wiring (`helpers/ssh_twins.py`): the lead's exec agent `start`s `ssh -L N:127.0.0.1:N` with the remote kernel in that session, one raw `ssh ... fantastic_team send` makes the remote `web` host, its `web_ws` leg `remote_ws`, the edges and the remote exec agent, `git clone` runs in the box, and a local `ws_bridge` agent `remote` dials the tunnel; remote agents are made and told through `forward` (to `user`, whose `forward` makes the call). At the end: the remote agents deleted through the bridge, the bridge deleted, SIGTERM to the remote kernel over ssh (the tunnel closes with it).

Shared checks (`Box.end`): solved on the test's git server with one commit from each side (found in each clone's reflog), linear history; the bridge gone, the tunnel's port refusing, no exec job running; the remote kernel stopped, and, started once more, holding its stock, the surface and the exec agent only, no spawned agent, no orphan; the container removed.

Needs Podman with a running machine (else `SKIP`), builds the Linux binary once per source tree (`helpers/podman_remote.py`). Free: the scripted plug (`script.json`, written by `python3 -m helpers.ssh_twins --write` in `e2e/`), the fake forge, no network.
