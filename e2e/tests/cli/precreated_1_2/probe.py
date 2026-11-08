"""precreated_1_2: a lead and two workers made with create_agent; the lead tells each worker to fix one
file, they solve. The tree after is the tree before: the same 3, nothing extra. The steps and the
outcome checks are `helpers/cli_scenarios.py`'s (the gpu twin uses them too)."""

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict

TASK = "Fix the failing test: your two workers are w1 and w2."


def run(ctx):
    v = Verdict("cli/precreated_1_2")
    before = S.precreate(ctx, v, S.ONE_TWO)
    ctx.team.send(cli.LEAD, {"type": "send", "text": TASK})
    v.check("the scripted model played every turn, none past the end",
            cli.wait_served(ctx, 120) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    S.precreated_end(ctx, v, before)
    v.check("each agent of the tree that was told a task asked the model", {"w1", "w2"} <= set(ctx.model.request_lanes),
            str(sorted(set(ctx.model.request_lanes))))
    cli.check_ins_played(ctx, v)
    return v.finish()
