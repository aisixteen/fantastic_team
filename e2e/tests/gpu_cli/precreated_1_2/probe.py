"""gpu_cli/precreated_1_2: a lead and two workers made with create_agent; the lead tells each worker to fix one
file, they solve. The tree after is the tree before.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s, the same as `cli/precreated_1_2`; no script.
Every agent reads the door's key from the credential store (when the server has one)."""

from helpers import cli_scenarios as S, gpu_cli as gpu
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli/precreated_1_2")
    before = S.precreate(ctx, v, S.ONE_TWO, all_keyed=True)
    watch = gpu.play(ctx, v, "lead", "precreated_1_2")
    gpu.check_ins_ran(ctx, v)
    gpu.lead_on_default(ctx, v)
    gpu.workers_on_model(ctx, v)
    S.precreated_end(ctx, v, before)
    gpu.check_key_not_on_disk(ctx, v)
    gpu.report(ctx, watch)
    return v.finish()
