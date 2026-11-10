"""gpu_cli/precreated_tree: a lead, two sub-leads, and a worker under each, all laid as record files before the first start. The lead
tells each sub-lead, each sub-lead tells its worker, the workers solve. The tree is unchanged.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s, the same as `cli/precreated_tree`; no script.
The body is `gpu_cli.precreated_run`'s."""

from helpers import cli_scenarios as S, gpu_cli as gpu


def run(ctx):
    return gpu.precreated_run(ctx, "precreated_tree", S.TREE)
