"""gpu_cli_ssh/precreated_tree: `cli_ssh/precreated_tree` on a real model: the lead, s1, w1, s2 and w2 on the master;
the remote asks s2 (a backend on the master) for the box's work; the box and the wiring told in plain words. The
box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

from helpers import cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli_ssh/precreated_tree")
    G.require(ctx)
    with T.Box(ctx) as box:
        before = S.precreate(ctx, v, S.TREE, all_keyed=ctx.model.keyed)
        box.open_backends(["s2"])
        box.publish()
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box, ["s2"]) + G.TASKS["precreated_tree"])
        gpu.check_ins_ran(ctx, v)
        gpu.lead_on_default(ctx, v)
        gpu.workers_on_model(ctx, v)
        S.precreated_end(ctx, v, before)
        box.end(v)
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
