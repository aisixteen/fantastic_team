"""precreated_1_2: a lead and two workers in the blueprint; the lead tells each worker to fix one
file, they solve. The tree after is the tree before: the same 3, nothing extra."""

from helpers import cli
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("cli/precreated_1_2")
    cli.precreated(ctx, v, {"lead_1": "user", "w1_1": "lead_1", "w2_1": "lead_1"}, {"w1", "w2"},
                   "Fix the failing test: your two workers are w1_1 and w2_1.")
    return v.finish()
