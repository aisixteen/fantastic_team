"""single_lead: one lead solves the fixture, started two ways.

A: the lead laid as a record file (fixed id `lead`) before the first start of `--headless`.
B: no file. `--headless ai anthropic --model M --endpoint URL` (fantastic's recipe: the root AI
   `ai`), then `send` of `create_agent` / `update_agent` payloads.
The script plays twice in a row (the first half for A, the second for B); one lead is on at a time.
The steps and the outcome checks are `helpers/cli_scenarios.py`'s (the gpu twin uses them too).

THE FOLDS (team #323): after the commit the lead makes a thought and prints a large reply (message 1 ends with
`done`); between the messages the probe finds the two rows in the lead's history and gives their ids to the plug;
message 2 recalls both. `helpers/folds.py` reads the request bodies: whole, then an anchor, recalled whole, an
anchor again.
"""

from helpers import cli, cli_scenarios as S, folds, run_flow
from helpers.verdict import Verdict

TASK = "Fix the failing test in the repo and commit the fix."


def run(ctx):
    v = Verdict("cli/single_lead")
    per_run = len(ctx.model.turns) // 2
    at = folds.at_of(ctx.model.script)
    lead, before = S.single_by_hand(ctx, v)
    folds.widen(ctx, v, lead)
    v.check("A: the first message played the fix and the large reply, up to its `done`",
            run_flow.hand_over(ctx, TASK, lead=lead, upto=at + 3), f"served {ctx.model.served} of {at + 3}")
    found = folds.bind_rows(ctx, v, lead, "A: ")
    v.check("A: it played every turn", run_flow.hand_over(ctx, folds.MORE, lead=lead, upto=per_run), f"served {ctx.model.served}")
    S.single_end(ctx, v, "A", before)
    folds.check(ctx, v, "A: ", at, found, lead)
    ctx.team.stop()

    # B (fantastic's `ai anthropic` recipe) waits for fantastic#518: the recipe still makes a
    # provider as the root chat, and a provider holds no conversation now
    print("  B: skipped until fantastic#518 (the recipe makes a provider-only root)")
    v.check("the scripted model played exactly A's turns, none past them",
            ctx.model.served == per_run and not ctx.model.overrun, f"served {ctx.model.served} of {per_run}")
    return v.finish()
