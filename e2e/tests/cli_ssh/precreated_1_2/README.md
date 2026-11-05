# cli_ssh/precreated_1_2

The ssh twin of `cli/precreated_1_2` (#242): one task split across the local kernel and a remote one in a Podman box, wired the weak way.

The lead and `w1` here (made before the task); `w2` made by the lead on the remote kernel through the bridge. `w1` fixes adder.py and pushes, then `w2` pulls, fixes muler.py and pushes; the lead reads `w2`'s history back through the bridge and deletes it there.

Checks: The local checks of `cli/precreated_1_2`; each worker asked the model; `w2`'s history (its push) came back through the bridge.

The wiring (`helpers/ssh_twins.py`): the lead's exec agent `start`s `ssh -L N:127.0.0.1:N` with the remote kernel in that session, one raw `ssh ... fantastic_team send` makes the remote `web` host, its `web_ws` leg `remote_ws`, the edges and the remote exec agent, `git clone` runs in the box, and a local `ws_bridge` agent `remote` dials the tunnel; remote agents are made and told through `forward` (to `user`, whose `forward` makes the call). At the end: the remote agents deleted through the bridge, the bridge deleted, SIGTERM to the remote kernel over ssh (the tunnel closes with it).

Shared checks (`Box.end`): solved on the test's git server with one commit from each side (found in each clone's reflog), linear history; the bridge gone, the tunnel's port refusing, no exec job running; the remote kernel stopped, and, started once more, holding its stock, the surface and the exec agent only, no spawned agent, no orphan; the container removed.

Needs Podman with a running machine (else `SKIP`), builds the Linux binary once per source tree (`helpers/podman_remote.py`). Free: the scripted plug (`script.json`, written by `python3 -m helpers.ssh_twins --write` in `e2e/`), the fake forge, no network.
