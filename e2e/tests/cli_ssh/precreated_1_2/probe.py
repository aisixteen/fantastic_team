"""cli_ssh/precreated_1_2: the lead, w1 and w2 on the local kernel (the master; made before the task, and
`w3`, kept for the call in flight below); the box holds tools only. w1 fixes adder.py and pushes. The remote
agent (the work program, a job of the box's exec agent) sends w2 its work and watches it, over the bridge: w2's
turn runs on the master, its status and parts stream back to the box, its tools are the box's exec agent (pull,
fix muler.py, push): message 1. Then the operator's phase, the RECONNECT twin: the watch grant sealed on the
master (the box's watch gets nothing), the tunnel broken while a call to w3 is in flight (it fails
`bridge_down` on the box), the master redials the known port, the next call answers, and a watcher the box
made before the break keeps streaming w2's turn (no new `watch`). The lead reads w2's history back and closes
(message 2). The wiring and the shared checks are `helpers/ssh_twins.py`'s; the local checks are
`cli/precreated_1_2`'s (`helpers/cli_scenarios.py`)."""

from helpers import cli, cli_scenarios as S, podman_remote, ssh_twins as T
from helpers.verdict import Verdict

TASK = "Fix the failing test: w1 and w2 are your workers here; the box's workspace is w2's to fix, asked by the remote."
CLOSE = "Close the remote: delete the bridge, stop the remote kernel and the tunnel, and pull."


def run(ctx):
    v = Verdict("cli_ssh/precreated_1_2")
    podman_remote.require()
    n = T.phases_1_2()
    with T.Box(ctx) as box:
        S.precreate(ctx, v, S.ONE_TWO)
        cli.make_agent(ctx, "w3", cli.LEAD, {cli.EXEC: "*"})  # the backend of the call that is in flight when the tunnel breaks
        before = cli.tree(ctx.team)
        box.open_backends(["w2"])
        box.publish()
        box.bind()
        v.check("1: the scripted model played the lead's first message, w1's fix and w2's fix",
                T.hand(ctx, cli.LEAD, TASK, upto=n["one"] + n["w1"] + n["w2"])
                and T.wait_lanes(ctx, {"main": n["one"], "w1": n["w1"], "w2": n["w2"]}),
                f"served {ctx.model.served}, cursors {ctx.model.cursor}")
        v.check("each worker asked the model, both on the master (w1, and w2 for the remote)",
                {"w1", "w2"} <= set(ctx.model.request_lanes), str(sorted(set(ctx.model.request_lanes))))
        v.check("the remote work ran through the bridge: the work program exited 0",
                T.came_back(ctx, None, "<exit_code>0</exit_code>"), str(T.forward_replies(ctx)[-1:])[:300])
        box.check_stream(v, "w2", "w2")
        T.reconnect_phase(ctx, box, v)
        v.check("2: the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, CLOSE) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        v.check("w2's results are its history on the master (its calls through the bridge, in the lead's next turn)",
                T.came_back(ctx, "w2", "<target_id>remote</target_id>", via="w2 history"))
        S.precreated_end(ctx, v, before)
        box.end(v, streams=[("w2", "w2"), ("b", "master")])
    box.removed(v)
    return v.finish()
