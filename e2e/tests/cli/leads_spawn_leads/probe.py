"""leads_spawn_leads: a lead spawns two sub-leads, each spawns a worker, the workers solve the
fixture, and each parent deletes what it spawned. Nothing that was spawned is left, and no agent
is left without its parent. The steps and the outcome checks are `helpers/cli_scenarios.py`'s (the
gpu twin uses them too); the lanes are this test's own."""

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict

TASK = "Fix the failing test with sub-leads: they use workers; delete what you spawned."


def run(ctx):
    v = Verdict("cli/leads_spawn_leads")
    before = S.spawn_leads_start(ctx, v)
    ctx.team.send("lead", {"type": "send", "text": TASK})
    v.check("the scripted model played every turn, none past the end",
            cli.wait_served(ctx, 150) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    S.spawn_leads_end(ctx, v, before)
    v.check("every lane asked the model", {"s1", "s2", "w1", "w2"} <= set(ctx.model.request_lanes),
            str(sorted(set(ctx.model.request_lanes))))
    cli.check_ins_played(ctx, v)
    return v.finish()
