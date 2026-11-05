"""gpu_cli_ssh/precreated_1_2: `cli_ssh/precreated_1_2` on a real model: the lead and w1 here, w2 made by
the lead on the remote kernel; the lead is told the box and the wiring in plain words. The box and the
checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

from helpers import cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli_ssh/precreated_1_2")
    G.require(ctx)
    with T.Box(ctx) as box:
        before = S.precreate(ctx, v, {"lead": "user", "w1": "lead"})
        box.publish()
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box) + G.TASKS["precreated_1_2"])
        S.precreated_end(ctx, v, before)
        box.end(v)
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
