"""Install a pack with evals through the CLI: the reply has pass rate, time, cost and the verdict."""

import os

from helpers import packs_evals as P
from helpers import run_flow
from helpers.verdict import Verdict


def install(team, name):
    return P.cli(team, "install", "--pack", f"packs/{name}", "--into", "templates",
                 "--worker", "lead", "--timeout-s", "60")


def run(ctx):
    v = Verdict("packs/install_reports_evals_and_verdict")
    run_flow.boot(ctx)
    P.wire_packs(ctx.team)
    team = ctx.team
    P.make_pack(team, "good", {"a.yaml": P.PASS_EVAL})
    P.make_pack(team, "mixed", {"a.yaml": P.PASS_EVAL + P.FAIL_EVAL})

    # a pack whose evals all pass
    rc, row = install(team, "good")
    r = P.result(row)
    ev = r.get("evals") or {}
    v.check("install exits 0", rc == 0, str(row)[:400])
    v.check("the pass rate is 1.0, 1 of 1", ev.get("pass_rate") == 1.0 and ev.get("passed") == 1
            and ev.get("total") == 1, str(ev)[:400])
    v.check("the time is a number", isinstance(ev.get("total_time_s"), (int, float)), str(ev)[:300])
    v.check("the cost key is there (null: the scripted model reports none)", "total_cost" in ev, str(ev)[:300])
    text = ev.get("text", "")
    v.check("the text names pass rate, time and cost",
            "pass rate 100%" in text and "time " in text and "cost " in text, text)
    en = r.get("enable") or {}
    v.check("the verdict allows enable and names the rate",
            en.get("allowed") is True and "100%" in en.get("reason", ""), str(en))
    v.check("the template is on disk",
            os.path.exists(os.path.join(team.dir, "templates", "good-1.0.0", "manifest.yaml")))

    # a pack with one failing eval: installed all the same, the verdict blocks enable
    rc, row = install(team, "mixed")
    r = P.result(row)
    ev, en = r.get("evals") or {}, r.get("enable") or {}
    v.check("a failing eval does not refuse the install (exit 0)", rc == 0, str(row)[:400])
    v.check("the pass rate is 0.5", ev.get("pass_rate") == 0.5 and ev.get("passed") == 1
            and ev.get("total") == 2, str(ev)[:400])
    v.check("the text says 50%", "pass rate 50%" in ev.get("text", ""), ev.get("text", ""))
    v.check("the verdict blocks enable and names the rate",
            en.get("allowed") is False and "50%" in en.get("reason", ""), str(en))
    v.check("the template is still installed",
            os.path.exists(os.path.join(team.dir, "templates", "mixed-1.0.0", "manifest.yaml")))

    v.check("the scripted model served the 3 eval tasks, none past the end",
            ctx.model.served >= 3 and ctx.model.overrun == 0, f"served {ctx.model.served}, overrun {ctx.model.overrun}")
    code = team.stop()
    v.check("the team stopped on SIGTERM", code in (0, -15), f"exit {code}")
    return v.finish()
