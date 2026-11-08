"""gpu_cli_ssh/leads_spawn_leads: `cli_ssh/leads_spawn_leads` on a real model: the lead spawns two sub-leads on the local roles registry
(all the backends are on the master), one of them asked by the remote for the box's work; the box and the wiring
told in plain words. The
box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

from helpers import cli, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T, team as team_mod
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli_ssh/leads_spawn_leads")
    G.require(ctx)
    with T.Box(ctx) as box:
        cli.start(ctx, roles=True)
        cli.add_base(ctx)
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "update_agent", "delete_agent"]}, subtree=True)
        cli.make_registry(ctx, total=4, roles=cli.roles_dir("leads_spawn_leads"),
                          credentials=cli.model_credentials(ctx))
        cli.add_cadencer(ctx)
        before = cli.tree(ctx.team)
        box.publish()
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box) + G.TASKS["leads_spawn_leads"])
        ok, why = cli.solved(ctx.repo)
        v.check("solved in the local clone (the test passes on HEAD, the fix is committed)", ok, why)
        after = cli.tree(ctx.team)
        left = [i for i, a in ctx.team.agents().items() if a.get("spawned_by")]
        v.check("no spawned agent is left", not left, str(left))
        v.check("the records are the ones it started with", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        v.check("something was spawned", ("roles", "spawn") in team_mod.log_calls(ctx.team.log()))
        box.end(v)
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
