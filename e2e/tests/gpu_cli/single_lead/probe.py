"""gpu_cli/single_lead: one lead solves the fixture on a real model, started two ways.

A: `--headless`, then the lead made with `send` + `create_agent` under the fixed id `lead`.
B: fantastic's recipe `--headless ai vllm -p PORT --host H --kv-overflow O` (the root AI `ai`), then
   `send` of `update_agent` (the server's `/v1` URL as given, the model, the budgets, the slots, the
   key, the exec grant).
The steps and the outcome checks are `helpers/cli_scenarios.py`'s, the same as `cli/single_lead`; no
script. One lead is live at a time, so the run keeps to the shared GPU's slots.
"""

from helpers import cli_scenarios as S, gpu_cli as gpu
from helpers.verdict import Verdict


def variant(ctx, v, label, start):
    lead, before = start(ctx, v)
    watch = gpu.play(ctx, v, lead, "single_lead", label=f"{label}: ")
    S.single_end(ctx, v, label, before)
    gpu.report(ctx, watch)
    ctx.team.stop()


def run(ctx):
    v = Verdict("gpu_cli/single_lead")
    variant(ctx, v, "A", S.single_by_hand)
    variant(ctx, v, "B", S.single_by_recipe)
    gpu.check_key_not_on_disk(ctx, v)
    return v.finish()
