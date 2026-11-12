"""cli_ssh/checkins_busy_stuck (#284): the ssh twin of `cli/checkins_busy_stuck`. The lead fixes muler.py in the
box itself (through the bridge) and pushes, then spawns two sub-leads on the local bundles registry. s1 stays
generating (a slow first turn) across the lead's two check-ins, then pulls, fixes adder.py, pushes and reports
with a plain send; s2 is idle with no new row across both. The lead leaves s1 alone and stops s2, deletes
both and says `task failed: …`. The wiring and the shared checks are `helpers/ssh_twins.py`'s; the outcome
checks are `cli/checkins_busy_stuck`'s (`helpers/cli_scenarios.py`)."""

from helpers import cli, cli_scenarios as S, podman_remote, ssh_twins as T
from helpers.verdict import Verdict

TASK = "Fix the failing test: muler.py in your workspace on the remote box, adder.py with a sub-lead here; look at them on each check-in."


def run(ctx):
    v = Verdict("cli_ssh/checkins_busy_stuck")
    podman_remote.require()
    with T.Box(ctx) as box:
        before = S.busy_stuck_start(ctx, v)
        box.publish()
        box.bind()
        v.check("the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, TASK) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        ok, why = cli.solved(ctx.repo)
        v.check("solved in the local clone (the test passes on HEAD, the fix is committed)", ok, why)
        S.busy_stuck_end(ctx, v, before, T.S1_REPORT)
        v.check("each sub-lead asked the model, both on the master", {"s1", "s2"} <= set(ctx.model.request_lanes),
                str(sorted(set(ctx.model.request_lanes))))
        cli.check_ins_played(ctx, v)
        box.end(v)
    box.removed(v)
    return v.finish()
