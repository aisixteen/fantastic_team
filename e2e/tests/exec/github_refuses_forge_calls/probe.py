"""GitHub build: gh, git push and a call to api.github.com are refused before they run."""

from helpers import run_flow
from helpers.verdict import Verdict

REFUSAL = "command not allowed"


def run(ctx):
    v = Verdict("exec/github_refuses_forge_calls")
    run_flow.boot(ctx)
    played = run_flow.hand_over(ctx, "Run the commands you were given")
    v.check("the scripted model played every turn", played and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    bodies = ctx.model.raw_bodies
    last = bodies[-1] if bodies else ""
    for i, what in enumerate(["gh --version", "git push", "curl api.github.com"], start=1):
        v.check(f"{what} is refused before it runs",
                len(bodies) > i and REFUSAL in bodies[i] and (i == 1 or bodies[i].count(REFUSAL) >= i),
                f"after {i} calls: {bodies[i][-300:] if len(bodies) > i else 'no request'}")
    v.check("exactly the three forge commands were refused", last.count(REFUSAL) == 3,
            f"{last.count(REFUSAL)} refusals")
    v.check("an ordinary command still ran", "an-ordinary-command-still-runs" in last)
    ctx.team.stop()
    return v.finish()
