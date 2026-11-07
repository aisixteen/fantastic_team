# gpu_cli_ssh/leads_spawn_leads

`cli_ssh/leads_spawn_leads` on a real model (the gpu tier, #237, #242, #252): the vllm plug, no script. The lead is told in plain words where the box is (host, port, key path, login, the tunnel's port, the git server as the box dials it, the master's guid) and what the wiring is, the bridge's grants and the remote work program included (`helpers/gpu_ssh.py`); it writes every command itself. All the backends are on the master. The box and the checks are `helpers/ssh_twins.py`'s.

`SKIP` without the vllm variables or without Podman; a keyed server (`VLLM_API_KEY`) is no longer a `SKIP`: the key stays on the master and the box holds none (checked). The plug counts the local agents' usage, which is every agent's.

```
python3 e2e/run.py --only gpu_cli_ssh/leads_spawn_leads --bin /ABSOLUTE/path/fantastic_team
```
