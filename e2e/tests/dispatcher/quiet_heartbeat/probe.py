"""Quiet mode: a lead with an idle member and work queued gets a short heartbeat every third quiet beat."""

from helpers import run_flow, team as T
from helpers.verdict import Verdict

HEARTBEAT = "heartbeat: 2 idle, 1 queued"


def run(ctx):
    v = Verdict("dispatcher/quiet_heartbeat")
    run_flow.boot(ctx)
    T.wire_dispatcher(ctx.team)
    pulled = ctx.team.send("board", {"type": "pull"})
    v.check("the board pulled the forge's open issue", pulled.get("status") == "completed"
            and pulled["result"].get("tasks") == 1, str(pulled))
    rows = ctx.team.send("board", {"type": "rows"}).get("result", {})
    v.check("one task is queued, both workers idle",
            rows.get("load", {}).get("tasks", {}).get("queued") == 1
            and rows["load"]["workers"]["idle"] == 2, str(rows.get("load")))
    v.check("no heartbeat was sent before the cadence was set",
            not any("heartbeat:" in b for b in ctx.model.raw_bodies))

    r = ctx.team.send("cadencer", {"type": "set", "cadence": 1})
    v.check("the operator sets the cadence to 1s", r.get("status") == "completed", str(r))
    first = run_flow.wait_model_body(ctx, "w1: idle", 30)
    v.check("the first beat sent a regular digest (w1 and w2 idle)",
            first is not None and "w2: idle" in first)
    v.check("the regular digest is not a heartbeat",
            not any("heartbeat:" in b for b in ctx.model.raw_bodies))

    beat = run_flow.wait_model_body(ctx, HEARTBEAT, 20)
    v.check("a heartbeat with the idle and queued counts reached the driver", beat is not None)
    st = ctx.team.send("cadencer", {"type": "status"}).get("result", {})
    v.check("it came only after three quiet beats: four ticks at least",
            st.get("ticks", 0) >= 4, str(st))
    v.check("two digests so far: the regular one and the heartbeat",
            st.get("digests_sent", 0) >= 2, str(st))
    v.check("no delivery failed", st.get("delivery_errors") == 0, str(st))
    ctx.team.stop()
    return v.finish()
