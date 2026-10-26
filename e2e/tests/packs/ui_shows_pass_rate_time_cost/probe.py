"""The web UI (packs panel) shows the pass rate, time and cost: for the Eval button and for an install."""

import json
import os
import re
import shutil
import subprocess
import sys

from helpers import packs_evals as P
from helpers import run_flow
from helpers.verdict import Verdict

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", "..", ".."))
PANEL = os.path.join(ROOT, "crates", "team", "web", "packs.html")
DOM = os.path.join(ROOT, "e2e", "helpers", "panel_dom.js")


def drive(team, node, pack, worker, mode):
    r = subprocess.run([node, DOM, PANEL, team.binary, team.dir, pack, worker, mode], env=team.env,
                       capture_output=True, text=True, timeout=180)
    try:
        return json.loads(r.stdout)
    except ValueError:
        return {"error": "harness: " + (r.stdout + r.stderr)[:500], "result_text": ""}


def shows_all(text):
    return {"pass rate 100%": "pass rate 100%" in text,
            "time <n>s": re.search(r"time \d+(\.\d+)?s", text) is not None,
            "cost": "cost " in text}


def run(ctx):
    v = Verdict("packs/ui_shows_pass_rate_time_cost")
    node = shutil.which("node")
    if not node:
        print("SKIP: node not found (the panel is driven by node and a stub DOM)")
        return 0
    run_flow.boot(ctx)
    P.wire_packs(ctx.team)
    team = ctx.team
    pack = P.make_pack(team, "p", {"a.yaml": P.PASS_EVAL})

    out = drive(team, node, pack, "lead", "eval")
    v.check("the Eval button shows no error", out["error"] == "", out["error"])
    got = shows_all(out["result_text"])
    v.check("the Eval button shows the pass rate, the time and the cost", all(got.values()),
            f"{got} in {out['result_text']!r}")

    out = drive(team, node, pack, "lead", "install")
    v.check("the Install (preview) button shows no error", out["error"] == "", out["error"])
    got = shows_all(out["result_text"])
    v.check("an install shows the pass rate, the time and the cost", all(got.values()),
            f"{got} in {out['result_text']!r}")
    v.check("the install did happen (the template is on disk)",
            os.path.exists(os.path.join(team.dir, "templates", "p-1.0.0", "manifest.yaml")))

    code = team.stop()
    v.check("the team stopped on SIGTERM", code in (0, -15), f"exit {code}")
    return v.finish()
