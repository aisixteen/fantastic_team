"""The cadencer's own timer sends the group's driver a digest: it arrives as a model request."""

from helpers import run_flow, team as T
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("dispatcher/digest_reaches_driver")
    run_flow.boot(ctx)
    T.wire_dispatcher(ctx.team)
    r = ctx.team.send("cadencer", {"type": "set", "cadence": 2})
    v.check("the operator sets the cadence to 2s", r.get("status") == "completed"
            and r["result"].get("cadence") == 2, str(r))
    body = run_flow.wait_model_body(ctx, "w1: idle", 30)
    v.check("the driver's model got a request holding the digest", body is not None)
    body = body or ""
    v.check("the digest names both workers", "w1: idle" in body and "w2: idle" in body)
    v.check("the digest ends with the load line", "load:" in body)
    st = ctx.team.send("cadencer", {"type": "status"}).get("result", {})
    v.check("the timer ticked by itself", st.get("ticks", 0) >= 1, str(st))
    v.check("a digest was counted as sent", st.get("digests_sent", 0) >= 1, str(st))
    v.check("no delivery failed", st.get("delivery_errors") == 0, str(st))
    v.check("the digest volume is metered", st.get("digest_bytes_per_hour", 0) > 0, str(st))
    calls = T.log_calls(ctx.team.log())
    v.check("the probe never sent a tick", ("cadencer", "tick") not in calls)
    ctx.team.stop()
    return v.finish()
