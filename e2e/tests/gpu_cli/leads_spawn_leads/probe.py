"""gpu_cli/leads_spawn_leads: a lead spawns two sub-leads, each spawns a worker, the workers solve the
fixture, and each parent deletes what it spawned.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s, the same as `cli/leads_spawn_leads`; no script.
The workers' tier carries the model's credential ref (they need the door's key, if there is one)."""

from helpers import cli_scenarios as S, gpu_cli as gpu
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("gpu_cli/leads_spawn_leads")
    before = S.spawn_leads_start(ctx, v, credentials=True)
    watch = gpu.play(ctx, v, "lead", "leads_spawn_leads")
    gpu.check_ins_ran(ctx, v)
    gpu.lead_on_default(ctx, v)
    gpu.workers_on_model(ctx, v)
    S.spawn_leads_end(ctx, v, before)
    gpu.check_key_not_on_disk(ctx, v)
    gpu.report(ctx, watch)
    return v.finish()
