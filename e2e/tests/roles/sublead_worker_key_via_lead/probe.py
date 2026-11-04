"""roles/sublead_worker_key_via_lead: a sub-lead's worker gets its key from the main lead (#223)."""

import re

from helpers import cli, key_grants as kg
from helpers.verdict import Verdict

TASK = "Start a sub-lead, give it its key, and send it its first message. It will ask you for its worker's key."
HAS = {"credentials", "lead", "mcp", "user"}


def spawned(team):
    return {i: a for i, a in team.agents().items() if a.get("spawned_by")}


def both_with_get(team):
    m = kg.store_map(team)
    n = [k for k, v in m.items() if k.startswith("anthropic_backend_") and v == ["get"]]
    return m if len(n) >= 2 else None


def run(ctx):
    v = Verdict("roles/sublead_worker_key_via_lead")
    team = kg.boot(ctx)
    before = kg.restated(kg.store_map(team), kg.STORE)
    v.check("the store holds the entries the script restates (update its script with them)",
            set(before) == HAS, str(sorted(before)))
    tree = cli.tree(team)

    team.send("lead", {"type": "send", "text": TASK})
    seen = kg.poll(lambda: both_with_get(team), 120)
    v.check("the store names the sub-lead and its worker with `get` alone (the main lead's writes)",
            seen is not None, str(kg.store_map(team)))
    v.check("the scripted model played every turn, none past the end",
            cli.wait_served(ctx, 120) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    lanes = set(ctx.model.request_lanes)
    v.check("the main lead, the sub-lead and its worker all asked the model",
            {"main", "s1", "w1"} <= lanes, str(sorted(lanes)))
    key = ctx.secrets["model_key"]
    v.check("every request carried the model's key from the store",
            ctx.model.keys and set(ctx.model.keys) == {key}, str(set(ctx.model.keys)))
    refused = [m for b in ctx.model.raw_bodies for m in re.findall(r"\w+ may not call '\w+' on \w+", b)]
    v.check("no call was refused", not refused, str(refused[:3]))
    rec = spawned(team)
    subs = [i for i, a in rec.items() if a.get("role") == "lead"]
    workers = [i for i, a in rec.items() if a.get("role") == "worker"]
    v.check("the main lead spawned the sub-lead and the sub-lead the worker",
            len(subs) == 1 and len(workers) == 1 and rec[subs[0]]["parent_id"] == "lead"
            and rec[workers[0]]["parent_id"] == subs[0], str({i: a["parent_id"] for i, a in rec.items()}))
    if len(subs) != 1 or len(workers) != 1:
        return v.finish()
    sub, worker = subs[0], workers[0]
    if seen is not None:
        seen = kg.restated(seen, kg.STORE)
        v.check("every entry the store had stays, and both are on it",
                all(k in seen for k in before) and sub in seen and worker in seen, str(sorted(seen)))
    for who, id in (("sub-lead", sub), ("worker", worker)):
        rows = team.send(id, {"type": "history"})["result"]["rows"]
        kinds = [r["kind"] for r in rows]
        v.check(f"the {who}'s first turn had no refusal", "error" not in kinds and "user" in kinds, str(kinds))
    asked = [b for b, l in zip(ctx.model.raw_bodies, ctx.model.request_lanes) if l == "main"
             and f"<need_key>{worker}</need_key>" in b]
    v.check("the main lead heard the sub-lead's ask, which names the worker", bool(asked))
    gone = team.send("kernel", {"type": "delete_agent", "id": sub})
    v.check("the probe deletes the sub-lead with its worker", gone.get("status") == "completed", str(gone))
    after = kg.restated(kg.store_map(team), kg.STORE)
    v.check("both are off the store, which is the one it was before; the tree is unchanged",
            set(after) == set(before) and cli.tree(team) == tree, f"{sorted(after)} / {sorted(before)}")
    return v.finish()
