"""lead_spawns_two: the lead spawns two workers through the roles registry, they solve the
fixture (one file each), the lead deletes them. Lanes keep the one scripted plug in order:
w1 works, then w2 (`after`), then the lead's own held turn."""

import re

from helpers import cli, team as team_mod
from helpers.verdict import Verdict

TASK = "Fix the failing test: have two workers do it, then delete them."


def run(ctx):
    v = Verdict("cli/lead_spawns_two")
    cli.start(ctx, roles=True)
    cli.add_base(ctx)
    new = cli.import_blueprint(ctx)
    v.check("the import made the lead and its roles registry", sorted(new) == ["lead_1", "roles_1"], str(new))
    team = ctx.team
    before = cli.tree(team)
    count = team.send("roles_1", {"type": "get_count"})["result"]
    v.check("the registry is on and counts nothing running", count == {"running": 0, "max": 2}, str(count))

    team.send("lead_1", {"type": "send", "text": TASK})
    cli.wait_served(ctx, 120)
    v.check("the scripted model played every turn, none past the end", ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")

    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    calls = team_mod.log_calls(team.log())
    spawns = [c for c in calls if c == ("roles_1", "spawn")]
    v.check("the lead spawned 2 workers", len(spawns) == 2, str(spawns))
    lanes = set(ctx.model.request_lanes)
    v.check("each worker asked the model (a lane of its own)", {"w1", "w2"} <= lanes, str(lanes))
    spawned = re.findall(r"<id>(anthropic_backend_\w+)</id>", ctx.model.raw_bodies[-1])[:2]
    v.check("the lead saw two minted worker ids", len(set(spawned)) == 2, str(spawned))
    after = cli.tree(team)
    v.check("the records are the ones it started with (both workers deleted)", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    v.check("the deleted workers are gone", not any(s in after for s in spawned), str(spawned))
    count = team.send("roles_1", {"type": "get_count"})["result"]
    v.check("the registry counts nothing running", count["running"] == 0, str(count))
    return v.finish()
