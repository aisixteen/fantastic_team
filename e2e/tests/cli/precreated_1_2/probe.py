"""precreated_1_2: a lead and two workers made with create_agent; the lead tells each worker to fix one
file, they solve. The tree after is the tree before: the same 3, nothing extra."""

from helpers import cli
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("cli/precreated_1_2")
    cli.precreated(ctx, v, {"lead": "user", "w1": "lead", "w2": "lead"}, {"w1", "w2"},
                   "Fix the failing test: your two workers are w1 and w2.")
    return v.finish()
