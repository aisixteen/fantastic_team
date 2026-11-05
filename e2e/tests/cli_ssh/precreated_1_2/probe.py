"""cli_ssh/precreated_1_2: the lead and w1 on the local kernel (made before the task), w2 on a
remote kernel in a Podman box, made there by the lead through the bridge. w1 fixes adder.py and
pushes, w2 pulls, fixes muler.py and pushes; the lead reads w2's conversation back through the
bridge and deletes it there. The wiring and the shared checks are `helpers/ssh_twins.py`'s; the
local checks are `cli/precreated_1_2`'s (`helpers/cli_scenarios.py`)."""

from helpers import cli, cli_scenarios as S, podman_remote, ssh_twins as T
from helpers.verdict import Verdict

TASK = "Fix the failing test: w1 is your worker here, and w2 is yours to make on the remote kernel."


def run(ctx):
    v = Verdict("cli_ssh/precreated_1_2")
    podman_remote.require()
    with T.Box(ctx) as box:
        before = S.precreate(ctx, v, {"lead": "user", "w1": "lead"})
        box.publish()
        box.bind()
        v.check("the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, TASK) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        S.precreated_end(ctx, v, before)
        v.check("each worker asked the model, w1 here and w2 on the remote kernel",
                {"w1", "w2"} <= set(ctx.model.request_lanes), str(sorted(set(ctx.model.request_lanes))))
        v.check("w2's results came back through the bridge (its history, in the lead's next turn)",
                T.came_back(ctx, "w2", "<item>push</item>"))
        box.end(v)
    box.removed(v)
    return v.finish()
