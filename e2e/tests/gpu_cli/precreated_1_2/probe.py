"""gpu_cli/precreated_1_2: a lead and two workers laid as record files before the first start; the lead tells each worker to fix one
file, they solve. The tree after is the tree before.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s, the same as `cli/precreated_1_2`; no script.
Every agent reads the door's key from the credential store (when the server has one). The body is
`gpu_cli.precreated_run`'s."""

from helpers import cli_scenarios as S, gpu_cli as gpu


def run(ctx):
    return gpu.precreated_run(ctx, "precreated_1_2", S.ONE_TWO)
