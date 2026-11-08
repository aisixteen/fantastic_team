"""lead_spawns_two: the lead spawns two workers through the roles registry, they solve the
fixture (one file each), the lead deletes them. Lanes keep the one scripted plug in order:
w1 works, then w2 (`after`), then the lead's own held turn. The steps and the outcome checks are
`helpers/cli_scenarios.py`'s (the gpu twin uses them too); the lanes are this test's own."""

import re

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict

TASK = "Fix the failing test: have two workers do it, then delete them."


def run(ctx):
    v = Verdict("cli/lead_spawns_two")
    before = S.spawn_two_start(ctx, v)
    ctx.team.send("lead", {"type": "send", "text": TASK})
    cli.wait_served(ctx, 120)
    v.check("the scripted model played every turn, none past the end", ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    S.spawn_two_end(ctx, v, before)
    lanes = set(ctx.model.request_lanes)
    v.check("each worker asked the model (a lane of its own)", {"w1", "w2"} <= lanes, str(lanes))
    spawned = re.findall(r"<id>(\w+_[0-9a-f]{6,})</id>", ctx.model.raw_bodies[-1])[:2]
    v.check("the lead saw two minted worker ids", len(set(spawned)) == 2, str(spawned))
    v.check("the deleted workers are gone", not any(s in cli.tree(ctx.team) for s in spawned), str(spawned))
    return v.finish()
