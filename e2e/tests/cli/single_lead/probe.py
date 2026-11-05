"""single_lead: one lead solves the fixture, started two ways.

A: `--headless`, then the lead made with `send` + `create_agent` under the fixed id `lead`.
B: no file. `--headless ai anthropic --model M --endpoint URL` (fantastic's recipe: the root AI
   `ai`), then `send` of `create_agent` / `update_agent` payloads.
The script plays twice in a row (turns 1-6 for A, 7-12 for B); one lead is on at a time.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s (the gpu twin uses them too).
"""

import time

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict

TASK = "Fix the failing test in the repo and commit the fix."
PER_RUN = 6


def hand_over(ctx, lead, upto):
    r = ctx.team.send(lead, {"type": "send", "text": TASK})
    ok = r.get("status") == "completed"
    end = time.time() + 120
    while time.time() < end and ctx.model.served < upto:
        time.sleep(0.3)
    time.sleep(2.0)  # the last turn's reply is routed after the last request
    return ok and ctx.model.served >= upto


def run(ctx):
    v = Verdict("cli/single_lead")
    lead, before = S.single_by_hand(ctx, v)
    v.check("A: it played every turn", hand_over(ctx, lead, PER_RUN), f"served {ctx.model.served}")
    S.single_end(ctx, v, "A", before)
    ctx.team.stop()

    lead, before = S.single_by_recipe(ctx, v)
    v.check("B: it played every turn", hand_over(ctx, lead, 2 * PER_RUN), f"served {ctx.model.served}")
    S.single_end(ctx, v, "B", before)
    ctx.team.stop()
    v.check("the scripted model played exactly its 12 turns, none past the end",
            ctx.model.consumed, f"served {ctx.model.served} of {len(ctx.model.turns)}")
    return v.finish()
