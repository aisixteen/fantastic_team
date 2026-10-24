"""General build: the worker's exec is stock, so a forge command is not refused."""

from helpers import run_flow
from helpers.verdict import Verdict

REFUSAL = "command not allowed"


def run(ctx):
    v = Verdict("exec/general_allows_gh")
    run_flow.boot(ctx)
    played = run_flow.hand_over(ctx, "Run the commands you were given")
    v.check("the scripted model played every turn", played and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    bodies = ctx.model.raw_bodies
    last = bodies[-1] if bodies else ""
    v.check("both commands came back to the model", len(bodies) >= 3, f"{len(bodies)} requests")
    v.check("no worker command was refused as 'command not allowed'",
            not any(REFUSAL in b for b in bodies[1:]))
    v.check("no worker command was refused as unauthorized", "unauthorized" not in last)
    v.check("the shell command that names gh ran", "reached-the-shell" in last)
    ctx.team.stop()
    return v.finish()
