"""single_lead: one lead solves the fixture, started two ways.

A: `--headless`, then the blueprint folder imported under `user`.
B: no file. `--headless ai anthropic --model M --endpoint URL` (fantastic's recipe: the root AI
   `ai`), then `send` of `create_agent` / `update_agent` payloads.
The script plays twice in a row (turns 1-6 for A, 7-12 for B); one lead is on at a time.
"""

import time

from helpers import cli
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


def check_end(v, ctx, label, before, lead):
    after = cli.ids(ctx.team)
    ok, why = cli.solved(ctx.repo)
    v.check(f"{label}: solved (the test passes on HEAD, the fix is committed)", ok, why)
    v.check(f"{label}: the records are the ones it started with", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    return ok


def variant_blueprint(ctx, v):
    cli.start(ctx, sub="blueprint_run")
    cli.add_base(ctx)
    new = cli.import_blueprint(ctx)
    v.check("A: the import made the lead alone", len(new) == 1 and new[0].startswith("lead"), str(new))
    lead = new[0]
    before = cli.ids(ctx.team)
    v.check("A: the lead is an anthropic backend record",
            before.get(lead) == "anthropic_backend.tools", str(before))
    v.check("A: it played every turn", hand_over(ctx, lead, PER_RUN),
            f"served {ctx.model.served}")
    check_end(v, ctx, "A", before, lead)
    ctx.team.stop()


def variant_recipe(ctx, v):
    cli.start(ctx, ["--headless", "ai", "anthropic", "--model", "scripted-model",
                    "--endpoint", ctx.model.endpoint], sub="recipe_run")
    cli.create(ctx.team, {"id": "credentials", "parent_id": "user",
                          "handler_module": "credential_store.tools", "backend": "env",
                          "refs": {f"model/{ctx.model.host}": {"host": ctx.model.host,
                                                                "env": {"api_key": "E2E_MODEL_KEY"}}}})
    cli.create(ctx.team, dict(cli.tool_record(ctx.repo, ctx.work), id=cli.EXEC, parent_id="user"))
    ai = ctx.team.agents().get("ai")
    v.check("B: the recipe made the root AI `ai` on the anthropic backend",
            ai is not None and ai.get("handler_module") == "anthropic_backend.tools", str(ai)[:300])
    grants = dict(ai.get("can_call") or {})
    grants[cli.EXEC] = ["*"]
    r = ctx.team.send("kernel", {"type": "update_agent", "id": "ai",
                                 "credentials": {"store": "credentials", "ref": f"model/{ctx.model.host}"},
                                 "can_call": grants})
    v.check("B: send update_agent gave it the key and the exec grant", r.get("status") == "completed", str(r)[:300])
    before = cli.ids(ctx.team)
    v.check("B: it played every turn", hand_over(ctx, "ai", 2 * PER_RUN), f"served {ctx.model.served}")
    check_end(v, ctx, "B", before, "ai")
    ctx.team.stop()


def run(ctx):
    v = Verdict("cli/single_lead")
    variant_blueprint(ctx, v)
    variant_recipe(ctx, v)
    v.check("the scripted model played exactly its 12 turns, none past the end",
            ctx.model.consumed, f"served {ctx.model.served} of {len(ctx.model.turns)}")
    return v.finish()
