"""precreated_tree: a lead, two sub-leads, and a worker under each, all made with create_agent. The lead
tells each sub-lead, each sub-lead tells its worker, the workers solve. The tree is unchanged."""

from helpers import cli
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("cli/precreated_tree")
    cli.precreated(ctx, v, {"lead": "user", "s1": "lead", "w1": "s1", "s2": "lead", "w2": "s2"}, {"s1", "s2", "w1", "w2"},
                   "Fix the failing test: your sub-leads are s1 and s2.")
    ctx.team.stop()
    return v.finish()
