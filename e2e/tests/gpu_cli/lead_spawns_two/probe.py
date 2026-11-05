"""gpu_cli/lead_spawns_two: the lead spawns two workers through the roles registry, they solve the fixture
(one file each), the lead deletes them.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s, the same as `cli/lead_spawns_two`; no script.
The workers' tier carries the model's credential ref (they need the door's key, if there is one)."""

from helpers import cli_scenarios as S, gpu_cli as gpu
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli/lead_spawns_two")
    before = S.spawn_two_start(ctx, v, credentials=True)
    watch = gpu.play(ctx, v, "lead", "lead_spawns_two")
    S.spawn_two_end(ctx, v, before)
    gpu.check_key_not_on_disk(ctx, v)
    gpu.report(ctx, watch)
    return v.finish()
