"""usage_report: after a scripted run, the usage report is read from the team on demand.

The report is the provider's own `usage` verb, sent from the CLI (`fantastic_team send`): no new
verb. It breaks the counts down per model (`by_model`) and per calling chat (`by_agent`), with
the team total on top (requests, input, cache write and read, output, `cost_usd` or `unpriced`).
The test runs the single_lead scenario A, reads the report, and checks it against what the scripted
model served; then gives the provider a `prices` map and reads again.
"""

import time

from helpers import cli, cli_scenarios as S
from helpers.verdict import Verdict

TASK = "Fix the failing test in the repo and commit the fix."
TURNS = 6
TOTAL = {"requests": "requests", "input": "input_tokens", "cache_write": "cache_write_tokens",
         "cache_read": "cache_read_tokens", "output": "output_tokens"}  # per-model key: the total's key


def usage(ctx) -> dict:
    return ctx.team.send(cli.PROVIDER, {"type": "usage"}).get("result") or {}


def close(a, b) -> bool:
    return abs(a - b) <= 1e-9 + 1e-6 * abs(b)


def run(ctx):
    v = Verdict("cli/usage_report")
    lead, before = S.single_by_hand(ctx, v)
    r = ctx.team.send(lead, {"type": "send", "text": TASK})
    end = time.time() + 120
    while time.time() < end and ctx.model.served < TURNS:
        time.sleep(0.3)
    time.sleep(2.0)  # the last turn's reply is routed after the last request
    v.check("the lead played the script", r.get("status") == "completed" and ctx.model.served >= TURNS,
            f"served {ctx.model.served}")

    u = usage(ctx)
    by_model, by_agent = u.get("by_model") or {}, u.get("by_agent") or {}
    v.check("the provider's usage has by_model and by_agent", bool(by_model) and bool(by_agent), str(u)[:300])
    m = by_model.get(cli.MODEL) or {}
    v.check("per model: the scripted model served every request the plug saw",
            m.get("requests") == ctx.model.served and m.get("input", 0) > 0 and m.get("output", 0) > 0,
            f"{m} served {ctx.model.served}")
    v.check("per model: no prices on the record, so no cost_usd (never $0)",
            "cost_usd" not in m and "cost_usd" not in u, str(m))
    v.check("per agent: the lead is keyed by its chat id and carries the counts",
            (by_agent.get(lead) or {}).get("requests", 0) >= 1, str(by_agent)[:300])
    for key, total in TOTAL.items():
        v.check(f"the team total's {total} is the sum of the models and of the agents",
                u.get(total, -1) == sum(x.get(key, 0) for x in by_model.values())
                == sum(x.get(key, 0) for x in by_agent.values()),
                f"{u.get(total)} {[x.get(key) for x in by_model.values()]} {[x.get(key) for x in by_agent.values()]}")
    v.check("the same report is read twice, unchanged while no turn runs", usage(ctx).get("requests") == u.get("requests"),
            "")

    # prices for another model only: this one is unpriced, never $0
    other = ctx.team.send("kernel", {"type": "update_agent", "id": cli.PROVIDER,
                                     "prices": {"some-other-model": {"input": 1.0, "output": 2.0}}})
    um = (usage(ctx).get("by_model") or {}).get(cli.MODEL) or {}
    v.check("a prices map without this model: it is unpriced, not $0",
            other.get("status") == "completed" and um.get("unpriced") is True and "cost_usd" not in um, str(um))

    # prices naming this model: cost_usd per model, per agent and in total
    prices = {cli.MODEL: {"input": 3.0, "output": 15.0}}
    done = ctx.team.send("kernel", {"type": "update_agent", "id": cli.PROVIDER, "prices": prices})
    v.check("send update_agent gave the provider a prices map", done.get("status") == "completed", str(done)[:300])
    p = usage(ctx)
    pm = (p.get("by_model") or {}).get(cli.MODEL) or {}
    want = (m["input"] * 3.0 + m["output"] * 15.0) / 1e6
    v.check("priced: the model's cost_usd is its tokens at the record's prices and it is not unpriced",
            "cost_usd" in pm and close(pm["cost_usd"], want) and not pm.get("unpriced"), f"{pm} want {want}")
    pa = (p.get("by_agent") or {}).get(lead) or {}
    v.check("priced: the agent's cost_usd and the total's are the model's (one model served)",
            close(pa.get("cost_usd", -1), want) and close(p.get("cost_usd", -1), want), f"{pa} {p.get('cost_usd')}")
    ctx.team.stop()
    return v.finish()
