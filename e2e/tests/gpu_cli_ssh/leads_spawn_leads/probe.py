"""gpu_cli_ssh/leads_spawn_leads: `cli_ssh/leads_spawn_leads` on a real model: the lead spawns two sub-leads on the local bundles registry
(all the backends are on the master), one of them asked by the remote for the box's work; the box and the wiring
told in plain words. The box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no
script. The body is `gpu_ssh.spawn_run`'s."""

from helpers import gpu_ssh as G


def run(ctx):
    return G.spawn_run(ctx, "leads_spawn_leads")
