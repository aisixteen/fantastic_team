"""gpu_cli_ssh/precreated_1_2: `cli_ssh/precreated_1_2` on a real model: the lead, w1 and w2 on the master; the
remote asks w2 (a backend on the master) for the box's work; the lead is told the box and the wiring in plain
words. The box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

from helpers import cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli_ssh/precreated_1_2")
    G.require(ctx)
    with T.Box(ctx) as box:
        before = S.precreate(ctx, v, S.ONE_TWO, all_keyed=ctx.model.keyed)
        box.open_backends(["w2"])
        box.publish()
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box, ["w2"]) + G.TASKS["precreated_1_2"])
        gpu.check_ins_ran(ctx, v)
        gpu.lead_on_default(ctx, v)
        gpu.workers_on_model(ctx, v)
        S.precreated_end(ctx, v, before)
        box.end(v)
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
