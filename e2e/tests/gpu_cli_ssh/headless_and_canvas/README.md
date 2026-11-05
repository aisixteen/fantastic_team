# gpu_cli_ssh/headless_and_canvas

`cli_ssh/headless_and_canvas` on a real model (the gpu tier, #237, #242): the vllm plug, no script. The lead is told in plain words where the box is (host, port, key path, login, the tunnel's port, the git server as the box dials it) and what the wiring is (`helpers/gpu_ssh.py`); it writes every command itself. Four messages, as the scripted twin: the canvas (local `canvas --port P`); the bridge deleted and the remote stopped; the remote and the bridge again (local `webui --port P2`); the cleanup. Then `webui` with `canvas` is refused and `--headless` opens no port. The box and the checks are `helpers/ssh_twins.py`'s.

`SKIP` without the vllm variables, without Podman, or with a keyed server (`VLLM_API_KEY`: the box holds no key). The plug counts the local agents' usage only.

```
python3 e2e/run.py --only gpu_cli_ssh/headless_and_canvas --bin /ABSOLUTE/path/fantastic_team
```
