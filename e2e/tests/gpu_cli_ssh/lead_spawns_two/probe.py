"""gpu_cli_ssh/lead_spawns_two: `cli_ssh/lead_spawns_two` on a real model: one side spawned on the local roles registry, the
other on a registry the lead makes on the remote kernel; the box, the wiring and the role cards told in
plain words. The box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s;
no script."""

from helpers import cli, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T, team as team_mod
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli_ssh/lead_spawns_two")
    G.require(ctx)
    with T.Box(ctx, roles=True) as box:
        cli.start(ctx, roles=True)
        cli.add_base(ctx)
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]}, subtree=True)
        cli.make_registry(ctx, total=1, roles=cli.roles_dir("lead_spawns_two"))
        before = cli.tree(ctx.team)
        box.publish()
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box, T.cards_of("lead_spawns_two")) + G.TASKS["lead_spawns_two"])
        ok, why = cli.solved(ctx.repo)
        v.check("solved in the local clone (the test passes on HEAD, the fix is committed)", ok, why)
        after = cli.tree(ctx.team)
        left = [i for i, a in ctx.team.agents().items() if a.get("spawned_by")]
        v.check("no spawned agent is left here", not left, str(left))
        v.check("the local records are the ones it started with", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        v.check("something was spawned here", ("roles", "spawn") in team_mod.log_calls(ctx.team.log()))
        box.end(v)
        v.check("something was spawned on the remote kernel", bool(T.remote_spawns(box.remote_log_text)))
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
