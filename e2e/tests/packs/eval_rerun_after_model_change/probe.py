"""`eval <pack>` after a model change gives a new rate."""

import socket
import time

from helpers import packs_evals as P
from helpers import run_flow
from helpers.verdict import Verdict


def dead_endpoint() -> str:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()  # nothing listens here now
    return f"http://127.0.0.1:{port}"


def run(ctx):
    v = Verdict("packs/eval_rerun_after_model_change")
    run_flow.boot(ctx)
    P.wire_packs(ctx.team)
    team = ctx.team
    pack = P.make_pack(team, "p", {"a.yaml": P.PASS_EVAL, "b.yaml": P.PASS_EVAL.replace("id: a", "id: c")})

    rc, row = P.cli(team, "eval", "--pack", pack, "--worker", "lead", "--timeout-s", "60")
    first = P.result(row)
    v.check("the first eval exits 0 with rate 1.0", rc == 0 and first.get("pass_rate") == 1.0, str(row)[:400])
    v.check("its text shows 100%", "pass rate 100%" in first.get("text", ""), first.get("text", ""))
    time.sleep(2)  # the lead's last nudge may still be in flight
    served = ctx.model.served
    v.check("the scripted model served both tasks", served >= 2 and ctx.model.overrun == 0, f"served {served}")

    # the model changes: the lead now points at an endpoint nothing listens on
    r = team.send("kernel", {"type": "update_agent", "id": "lead", "endpoint": dead_endpoint()})
    v.check("the lead's model was changed", r.get("status") == "completed", str(r))

    rc, row = P.cli(team, "eval", "--pack", pack, "--worker", "lead", "--timeout-s", "60")
    second = P.result(row)
    v.check("the second eval gives a new rate", second.get("pass_rate") is not None
            and second.get("pass_rate") != first.get("pass_rate"), str(row)[:600])
    v.check("its text shows the new rate", "pass rate 0%" in second.get("text", ""), second.get("text", ""))
    v.check("no eval passed after the change", second.get("passed") == 0, str(second)[:400])
    v.check("the dead model was not served again", ctx.model.served == served, f"served {ctx.model.served}, was {served}")

    code = team.stop()
    v.check("the team stopped on SIGTERM", code in (0, -15), f"exit {code}")
    return v.finish()
