"""precreated_tree: a lead, two sub-leads, and a worker under each, all in the blueprint. The lead
tells each sub-lead, each sub-lead tells its worker, the workers solve. The tree is unchanged."""

from helpers import cli
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("cli/precreated_tree")
    cli.precreated(ctx, v, {"lead_1": "user", "s1_1": "lead_1", "w1_1": "s1_1", "s2_1": "lead_1", "w2_1": "s2_1"}, {"s1", "s2", "w1", "w2"},
                   "Fix the failing test: your sub-leads are s1_1 and s2_1.")
    ctx.team.stop()
    return v.finish()
