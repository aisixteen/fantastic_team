"""roles/lead_grants_worker_key: the main lead grants a spawned worker its key on the credential
store, and the worker's first turn reads it (#223)."""

import re

from helpers import cli, key_grants as kg
from helpers.verdict import Verdict

TASK = "Start a worker, give it its key, send it its first message."
# the entries the store has before the run, as the script restates them (`$self` as the store's own id)
HAS = {"credentials", "lead", "mcp", "user"}


def run(ctx):
    v = Verdict("roles/lead_grants_worker_key")
    team = kg.boot(ctx)
    before = kg.restated(kg.store_map(team), kg.STORE)
    v.check("the store holds the entries the script restates (update its script with them)",
            set(before) == HAS, str(sorted(before)))
    tree = cli.tree(team)

    team.send("lead", {"type": "send", "text": TASK})
    seen = kg.poll(lambda: kg.worker_with_get(team), 90)
    v.check("the store names the spawned worker with `get` alone (the lead's write)", seen is not None,
            str(kg.store_map(team)))
    seen = kg.restated(seen, kg.STORE) if seen else seen
    v.check("the scripted model played every turn, none past the end",
            cli.wait_served(ctx, 120) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    lanes = set(ctx.model.request_lanes)
    v.check("the worker asked the model: its first turn read its key (a refused read asks nothing)",
            "w1" in lanes, str(sorted(lanes)))
    key = ctx.secrets["model_key"]
    v.check("every request carried the model's key from the store",
            ctx.model.keys and set(ctx.model.keys) == {key}, str(set(ctx.model.keys)))
    refused = [m for b in ctx.model.raw_bodies for m in re.findall(r"\w+ may not call '\w+' on \w+", b)]
    v.check("no call of the lead or the worker was refused", not refused, str(refused[:3]))
    worker = next((i for i, a in team.agents().items() if a.get("spawned_by")), None)
    v.check("the lead spawned one worker", worker is not None, str(sorted(cli.tree(team))))
    if seen is not None and worker is not None:
        v.check("every entry the store had stays, and the worker is on it",
                all(k in seen for k in before) and worker in seen, str(sorted(seen)))
    if worker is None:
        return v.finish()
    rows = team.send(worker, {"type": "history"})["result"]["rows"]
    kinds = [r["kind"] for r in rows]
    v.check("the worker's first turn had no refusal: its one message, then its answer",
            kinds.count("user") == 1 and "error" not in kinds, str(kinds))
    gone = team.send("kernel", {"type": "delete_agent", "id": worker})
    v.check("the probe deletes the worker (the script leaves it for the history above)",
            gone.get("status") == "completed", str(gone))
    after = kg.restated(kg.store_map(team), kg.STORE)
    v.check("the worker is off the store, which is the one it was before; the tree is unchanged",
            set(after) == set(before) and cli.tree(team) == tree, f"{sorted(after)} / {sorted(before)}")
    return v.finish()
