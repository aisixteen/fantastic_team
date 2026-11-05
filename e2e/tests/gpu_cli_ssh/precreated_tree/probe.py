"""gpu_cli_ssh/precreated_tree: `cli_ssh/precreated_tree` on a real model: the lead, s1, w1 here; s2 and
w2 made by the lead on the remote kernel; the box and the wiring told in plain words. The box and the
checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

from helpers import cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli_ssh/precreated_tree")
    G.require(ctx)
    with T.Box(ctx) as box:
        before = S.precreate(ctx, v, {"lead": "user", "s1": "lead", "w1": "s1"})
        box.publish()
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box) + G.TASKS["precreated_tree"])
        S.precreated_end(ctx, v, before)
        box.end(v)
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
