"""precreated_tree: a lead, two sub-leads, and a worker under each, all laid as record files before the first start. The lead
tells each sub-lead, each sub-lead tells its worker, the workers solve. The tree is unchanged. The
steps and the outcome checks are `helpers/cli_scenarios.py`'s (the gpu twin uses them too)."""

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict

TASK = "Fix the failing test: your sub-leads are s1 and s2."


def run(ctx):
    v = Verdict("cli/precreated_tree")
    before = S.precreate(ctx, v, S.TREE)
    ctx.team.send(cli.LEAD, {"type": "send", "text": TASK})
    v.check("the scripted model played every turn, none past the end",
            cli.wait_served(ctx, 120) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    S.precreated_end(ctx, v, before)
    v.check("each agent of the tree that was told a task asked the model",
            {"s1", "s2", "w1", "w2"} <= set(ctx.model.request_lanes), str(sorted(set(ctx.model.request_lanes))))
    cli.check_ins_played(ctx, v, ("main", "s1", "s2"))
    ctx.team.stop()
    return v.finish()
