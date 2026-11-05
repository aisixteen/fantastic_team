"""gpu_cli/precreated_tree: a lead, two sub-leads, and a worker under each, all made with create_agent. The lead
tells each sub-lead, each sub-lead tells its worker, the workers solve. The tree is unchanged.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s, the same as `cli/precreated_tree`; no script.
Every agent reads the door's key from the credential store (when the server has one)."""

from helpers import cli_scenarios as S, gpu_cli as gpu
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli/precreated_tree")
    before = S.precreate(ctx, v, S.TREE, all_keyed=True)
    watch = gpu.play(ctx, v, "lead", "precreated_tree")
    S.precreated_end(ctx, v, before)
    gpu.check_key_not_on_disk(ctx, v)
    gpu.report(ctx, watch)
    return v.finish()
