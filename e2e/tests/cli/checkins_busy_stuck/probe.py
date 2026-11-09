"""cli/checkins_busy_stuck (#284): a lead with two sub-leads looks at them on two check-ins in a row. One
sub-lead stays generating across both (a slow scripted turn): the lead does not stop it, it finishes and
reports. The other is not generating and adds no row across both (it awaits with nothing in flight, so its
run ended): the lead stops it and says `task failed: …`. The check-in turns are the lead's own scripted
ones (a tick wakes the lead as a notice, answered outside the script); they read each sub-lead's `status`
and `history` as the lead's card says. The outcome checks are `helpers/cli_scenarios.py`'s (the ssh twin
uses them too)."""

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict

TASK = "Fix the failing test with two sub-leads; look at them on each check-in."


def run(ctx):
    v = Verdict("cli/checkins_busy_stuck")
    before = S.busy_stuck_start(ctx, v)
    ctx.team.send("lead", {"type": "send", "text": TASK})
    v.check("the scripted model played every turn, none past the end",
            cli.wait_served(ctx, 150) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    S.busy_stuck_end(ctx, v, before)
    v.check("each sub-lead asked the model (a lane of its own)", {"s1", "s2"} <= set(ctx.model.request_lanes),
            str(sorted(set(ctx.model.request_lanes))))
    cli.check_ins_played(ctx, v)
    return v.finish()
