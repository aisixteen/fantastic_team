"""cli_ssh/precreated_tree: the lead, s1 and w1 under it, s2 and w2 under it: all on the local kernel (the
master; made before the task); the box holds tools only. s1 has w1 fix adder.py and push; the remote agent
(the work program in the box) sends s2 its work and watches it, over the bridge: s2's turn runs on the
master, its stream comes back to the box, and s2 has w2 (on the master, its tools the box's exec agent)
pull, fix muler.py and push. The lead reads w2's conversation back. The wiring and the shared checks are
`helpers/ssh_twins.py`'s; the local checks are `cli/precreated_tree`'s."""

from helpers import cli, cli_scenarios as S, podman_remote, ssh_twins as T
from helpers.verdict import Verdict

TASK = "Fix the failing test: s1, s2 and their workers are here; the box's workspace is s2's, asked by the remote."


def run(ctx):
    v = Verdict("cli_ssh/precreated_tree")
    podman_remote.require()
    with T.Box(ctx) as box:
        before = S.precreate(ctx, v, S.TREE)
        box.open_backends(["s2"])
        box.publish()
        box.bind()
        v.check("the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, TASK) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        S.precreated_end(ctx, v, before)
        v.check("each agent of the tree that was told a task asked the model, all on the master (s1, w1, s2 and w2)",
                {"s1", "s2", "w1", "w2"} <= set(ctx.model.request_lanes), str(sorted(set(ctx.model.request_lanes))))
        v.check("the remote work ran through the bridge: the work program exited 0",
                T.came_back(ctx, None, "<exit_code>0</exit_code>"), str(T.forward_replies(ctx)[-1:])[:300])
        v.check("w2's results are its history on the master (its calls through the bridge, in the lead's next turn)",
                T.came_back(ctx, "w2", "<target_id>remote</target_id>", via="w2 history"))
        cli.check_ins_played(ctx, v, ("main", "s1", "s2"))
        box.end(v, streams=[("s2", "s2")])
    box.removed(v)
    return v.finish()
