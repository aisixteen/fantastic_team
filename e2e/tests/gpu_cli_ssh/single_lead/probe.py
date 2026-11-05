"""gpu_cli_ssh/single_lead: `cli_ssh/single_lead` on a real model (A, the lead made with create_agent):
told in plain words where the box is and what the wiring is, the lead fixes adder.py here and muler.py
in its remote workspace through the bridge, pushing both. The box and the checks are
`helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

from helpers import cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli_ssh/single_lead")
    G.require(ctx)
    with T.Box(ctx) as box:
        lead, before = S.single_by_hand(ctx, v)
        box.publish()
        watch = G.play(ctx, v, lead, G.wiring(ctx, box) + G.TASKS["single_lead"])
        S.single_end(ctx, v, "A", before)
        box.end(v)
        gpu.report(ctx, watch)
    box.removed(v)
    return v.finish()
