"""Failing evals block enable; passing evals allow it."""

import os

from helpers import packs_evals as P
from helpers import run_flow
from helpers.verdict import Verdict


def agent_ids(team):
    r = team.send("kernel", {"type": "list_agents", "limit": 500})
    return {a.get("id") for a in (P.result(r).get("agents") or [])}


def enable(team, name, extra=()):
    return P.cli(team, "install", "--pack", f"packs/{name}", "--into", "templates",
                 "--worker", "lead", "--timeout-s", "60", "--parent-id", "lead", *extra)


def run(ctx):
    v = Verdict("packs/failing_evals_block_enable")
    run_flow.boot(ctx)
    P.wire_packs(ctx.team)
    team = ctx.team
    P.make_pack(team, "okpack", {"a.yaml": P.PASS_EVAL}, blueprint=True)
    P.make_pack(team, "badpack", {"a.yaml": P.PASS_EVAL + P.FAIL_EVAL}, blueprint=True)
    before = agent_ids(team)

    # the preview (no --approve) is not gated: the gate applies at enable
    rc, row = enable(team, "badpack")
    r = P.result(row)
    v.check("a preview of the failing pack is not blocked", rc == 0 and "approval" in r and "error" not in r,
            str(row)[:500])
    v.check("the preview still reports the failing verdict", (r.get("enable") or {}).get("allowed") is False,
            str(r.get("enable")))

    # failing evals: enable is refused, nothing is created, the template stays
    rc, row = enable(team, "badpack", ["--approve"])
    r = P.result(row)
    v.check("enable with failing evals exits non-zero", rc != 0, f"rc {rc}")
    v.check("the error says enable refused", str(r.get("error", "")).startswith("enable refused"), str(r)[:500])
    v.check("the verdict is not allowed", (r.get("enable") or {}).get("allowed") is False, str(r.get("enable")))
    v.check("nothing was imported", "imported" not in r and "ids" not in r, str(r)[:500])
    v.check("the template is kept",
            os.path.exists(os.path.join(team.dir, "templates", "badpack-1.0.0", "manifest.yaml")))
    v.check("no agent was created", agent_ids(team) == before, str(agent_ids(team) - before))

    # passing evals: enable goes through
    rc, row = enable(team, "okpack", ["--approve"])
    r = P.result(row)
    v.check("enable with passing evals exits 0", rc == 0, str(row)[:600])
    v.check("the verdict allows it", (r.get("enable") or {}).get("allowed") is True, str(r.get("enable")))
    v.check("it is installed as a worker", r.get("installed_as") == "worker", str(r)[:600])
    ids = r.get("ids") or {}
    v.check("the blueprint's agents were created", bool(ids), str(r)[:600])
    now = agent_ids(team)
    v.check("the new agents are live", bool(ids) and all(i in now for i in ids.values() if isinstance(i, str)),
            f"ids {ids} new {now - before}")

    v.check("the scripted model served the 2 + 2 + 1 eval tasks (preview, refused, enabled), none past the end",
            ctx.model.served >= 5 and ctx.model.overrun == 0,
            f"served {ctx.model.served}, overrun {ctx.model.overrun}")
    code = team.stop()
    v.check("the team stopped on SIGTERM", code in (0, -15), f"exit {code}")
    return v.finish()
