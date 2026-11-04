"""leads_spawn_leads: a lead spawns two sub-leads, each spawns a worker, the workers solve the
fixture, and each parent deletes what it spawned. Nothing that was spawned is left, and no agent
is left without its parent."""

from helpers import cli, team as team_mod
from helpers.verdict import Verdict

TASK = "Fix the failing test with sub-leads: they use workers; delete what you spawned."


def run(ctx):
    v = Verdict("cli/leads_spawn_leads")
    cli.start(ctx, roles=True)
    cli.add_base(ctx)
    new = cli.import_blueprint(ctx)
    v.check("the import made the lead and its roles registry", sorted(new) == ["lead_1", "roles_1"], str(new))
    team = ctx.team
    before = cli.tree(team)

    team.send("lead_1", {"type": "send", "text": TASK})
    v.check("the scripted model played every turn, none past the end",
            cli.wait_served(ctx, 150) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)

    calls = team_mod.log_calls(team.log())
    spawns = [c for c in calls if c == ("roles_1", "spawn")]
    v.check("4 spawns: 2 sub-leads by the lead, 1 worker by each", len(spawns) == 4, str(spawns))
    v.check("every lane asked the model", {"s1", "s2", "w1", "w2"} <= set(ctx.model.request_lanes),
            str(sorted(set(ctx.model.request_lanes))))

    after = cli.tree(team)
    agents = team.agents()
    left = [i for i, a in agents.items() if a.get("spawned_by")]
    v.check("no spawned agent is left", not left, str(left))
    orphans = [i for i, (_, p) in after.items() if p and p not in after]
    v.check("no orphans: every agent's parent exists", not orphans, str(orphans))
    v.check("the records are the ones it started with", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    count = team.send("roles_1", {"type": "get_count"})["result"]
    v.check("the registry counts nothing running", count["running"] == 0, str(count))
    return v.finish()
