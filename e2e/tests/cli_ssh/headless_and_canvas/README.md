# cli_ssh/headless_and_canvas

The ssh twin of `cli/headless_and_canvas` (#242): one task split across the local kernel and a remote one in a Podman box, wired the weak way.

The local kernel serves `canvas --port P`, the remote runs `--headless`. The lead makes `rlead` on the remote kernel, which makes its canvas (`create_agent` of a `canvas_page` under itself); the lead and `rlead` each add a note to NOTES.md and push. Then the local kernel is restarted as `webui --port P2`, the lead starts the remote again and makes the bridge again, and finally deletes everything it made there.

Checks: The box listens on sshd and the tunnel's port only; the local kernel on its page's port and the tunnel's local end only; the canvas's socket (`canvas_ws` -> `user.forward` -> the bridge) and the team page's socket list `rlead` and its canvas; both notes on the git server; then the local test's last checks (`webui` with `canvas` refused, `--headless` opens no port).

The team page's socket in this folder: `/team_ws/ws` is answered by `canvas_ws` here (both legs are recorded once `canvas` and then `webui` ran in one folder, and the second mount of the one route wins, `crates/team/src/webui.rs:10-12`), so the call goes through `user.forward` as the canvas page's does.

The wiring (`helpers/ssh_twins.py`): the lead's exec agent `start`s `ssh -L N:127.0.0.1:N` with the remote kernel in that session, one raw `ssh ... fantastic_team send` makes the remote `web` host, its `web_ws` leg `remote_ws`, the edges and the remote exec agent, `git clone` runs in the box, and a local `ws_bridge` agent `remote` dials the tunnel; remote agents are made and told through `forward` (to `user`, whose `forward` makes the call). At the end: the remote agents deleted through the bridge, the bridge deleted, SIGTERM to the remote kernel over ssh (the tunnel closes with it).

Shared checks (`Box.end`): solved on the test's git server with one commit from each side (found in each clone's reflog), linear history; the bridge gone, the tunnel's port refusing, no exec job running; the remote kernel stopped, and, started once more, holding its stock, the surface and the exec agent only, no spawned agent, no orphan; the container removed.

Needs Podman with a running machine (else `SKIP`), builds the Linux binary once per source tree (`helpers/podman_remote.py`). Free: the scripted plug (`script.json`, written by `python3 -m helpers.ssh_twins --write` in `e2e/`), the fake forge, no network.
