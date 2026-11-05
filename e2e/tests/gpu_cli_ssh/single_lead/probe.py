"""gpu_cli_ssh/single_lead: `cli_ssh/single_lead` on a real model, started two ways (A: the lead made
with create_agent; B: fantastic's recipe, the root AI `ai`), each with a box of its own: told in plain
words where the box is and what the wiring is, the lead fixes adder.py here and muler.py in its remote
workspace through the bridge, pushing both. The box and the checks are `helpers/ssh_twins.py`'s; the
task is `helpers/gpu_ssh.py`'s; no script. One lead is live at a time."""

from helpers import cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T
from helpers.verdict import Verdict


def variant(ctx, v, label: str, start) -> None:
    with T.Box(ctx, "box_" + label.lower()) as box:
        lead, before = start(ctx, v)
        box.publish()
        watch = G.play(ctx, v, lead, G.wiring(ctx, box) + G.TASKS["single_lead"], label=f"{label}: ")
        S.single_end(ctx, v, label, before)
        box.end(v, label=f"{label}: ")
        gpu.report(ctx, watch)
        ctx.team.stop()
    box.removed(v, f"{label}: ")


def run(ctx):
    v = Verdict("gpu_cli_ssh/single_lead")
    G.require(ctx)
    variant(ctx, v, "A", S.single_by_hand)
    variant(ctx, v, "B", S.single_by_recipe)
    return v.finish()
