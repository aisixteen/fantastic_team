"""Stop asks first. Only a confirmed stop is relayed to the worker, and the worker takes it."""

import json

from helpers import run_flow, team as T
from helpers.verdict import Verdict


def ids_of(listing: dict) -> list:
    agents = listing.get("result", {}).get("agents", [])
    return [a if isinstance(a, str) else a.get("id") for a in agents]


def control(team, **kw) -> dict:
    return team.send("dispatcher", dict(kw, type="control"))


def run(ctx):
    v = Verdict("dispatcher/stop_asks_then_stops")
    run_flow.boot(ctx)
    T.wire_dispatcher(ctx.team)
    team = ctx.team

    added = control(team, node="g", action="add_worker", id="w2", template="lead_t")
    r = added.get("result", {})
    v.check("the dispatcher added worker w2 to group g", r.get("sent") is True and "error" not in r,
            json.dumps(added))
    v.check("w2 is a live agent", "w2" in ids_of(team.send("kernel", {"type": "list_agents"})))

    asked = control(team, node="w2", action="stop").get("result", {})
    v.check("a stop without confirm answers with a question naming the worker",
            "Stop w2" in str(asked.get("question", "")), json.dumps(asked))
    v.check("the unconfirmed stop was not sent", asked.get("sent") is False)
    log = T.log_calls(team.log())
    v.check("the worker never got a stop", ("w2", "stop") not in log)
    v.check("the worker is still there", "w2" in ids_of(team.send("kernel", {"type": "list_agents"})))

    done = control(team, node="w2", action="stop", confirm=True)
    r = done.get("result", {})
    v.check("a confirmed stop is sent, with no error", r.get("sent") is True and "error" not in r,
            json.dumps(done))
    v.check("the stop reached the worker", ("w2", "stop") in T.log_calls(team.log()))
    status = team.send("w2", {"type": "status"}).get("result", {})
    v.check("the worker is not generating", status.get("generating") is False, json.dumps(status))
    team.stop()
    return v.finish()
