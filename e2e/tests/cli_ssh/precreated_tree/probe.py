"""cli_ssh/precreated_tree: the lead, s1 and w1 under it on the local kernel (made before the task);
s2 and w2 under it on a remote kernel in a Podman box, made there by the lead through the bridge.
s1 has w1 fix adder.py and push; then s2 (remote) has w2 (remote) pull, fix muler.py and push. The
lead reads w2's conversation back through the bridge and deletes both there. The wiring and the
shared checks are `helpers/ssh_twins.py`'s; the local checks are `cli/precreated_tree`'s."""

from helpers import cli, cli_scenarios as S, podman_remote, ssh_twins as T
from helpers.verdict import Verdict

TASK = "Fix the failing test: s1 and its worker are here; make s2 and its worker w2 on the remote kernel."


def run(ctx):
    v = Verdict("cli_ssh/precreated_tree")
    podman_remote.require()
    with T.Box(ctx) as box:
        before = S.precreate(ctx, v, {"lead": "user", "s1": "lead", "w1": "s1"})
        box.publish()
        box.bind()
        v.check("the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, TASK) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        S.precreated_end(ctx, v, before)
        v.check("each agent of the tree that was told a task asked the model, s1 and w1 here, s2 and w2 remote",
                {"s1", "s2", "w1", "w2"} <= set(ctx.model.request_lanes), str(sorted(set(ctx.model.request_lanes))))
        v.check("w2's results came back through the bridge (its history, in the lead's next turn)",
                T.came_back(ctx, "w2", "<item>push</item>"))
        box.end(v)
    box.removed(v)
    return v.finish()
