"""One store agent: every AI agent's memory and the task registry use it, and the task rows survive a restart."""

import json
import os

from helpers import run_flow
from helpers.verdict import Verdict


def agents(team):
    r = team.send("kernel", {"type": "list_agents"})["result"]
    return {a["id"]: a for a in r["agents"]}


def task_rows(team):
    return team.send("tasks", {"type": "list"})["result"]


def run(ctx):
    v = Verdict("roles/one_store_survives_restart")
    team = ctx.boot_team(cards=True)

    live = agents(team)
    stores = [i for i, a in live.items() if a.get("handler_module") == "sqlite_store.tools"]
    v.check("the agent list holds exactly one sqlite_store, named memory", stores == ["memory"], str(stores))
    extra = [i for i in live if i.endswith("_memory") and i != "kernel_memory"]
    v.check("no per-agent *_memory agent exists", extra == [], str(extra))
    v.check("the store's record puts its file at @state/memory.db", live["memory"].get("path") == "@state/memory.db",
            str(live["memory"].get("path")))
    v.check("the lead's memory_store_id names it", live[ctx.lead].get("memory_store_id") == "memory",
            str(live[ctx.lead].get("memory_store_id")))
    v.check("the lead's chat completes through the fleet's provider, which keeps its own state apart",
            live[ctx.lead].get("provider_id") == "provider" and live["provider"].get("memory_store_id") == "provider_store",
            str((live[ctx.lead].get("provider_id"), live.get("provider", {}).get("memory_store_id"))))
    v.check("tasks' task_store_id names it", live["tasks"].get("task_store_id") == "memory",
            str(live["tasks"].get("task_store_id")))

    played = run_flow.hand_over(ctx, "Start a worker and record an issue.")
    v.check("the scripted model played every turn, none past the end",
            played and ctx.model.consumed, f"served {ctx.model.served} of {len(ctx.model.turns)}")

    live = agents(team)
    workers = [a for a in live.values() if a.get("role") == "worker"]
    v.check("one worker was spawned and it names the one store",
            len(workers) == 1 and workers[0].get("memory_store_id") == "memory", str(workers))
    if workers:
        by = live["memory"].get("callable_by", {})
        v.check("the store admits the worker", by.get(workers[0]["id"]) in ("*", ["*"]), str(by))
    by = live["memory"].get("callable_by", {})
    v.check("the store admits the lead and tasks", all(by.get(a) in ("*", ["*"]) for a in (ctx.lead, "tasks")), str(by))
    v.check("the spawn made no *_memory agent",
            [i for i in live if i.endswith("_memory") and i != "kernel_memory"] == [])

    db = os.path.join(ctx.run_dir, ".fantastic", "memory.db")
    v.check(".fantastic/memory.db exists in the team directory", os.path.isfile(db), db)

    # the operator may read what the lead keeps: `list` on tasks
    cb = dict(live["tasks"].get("callable_by") or {})
    cb["mcp"] = ["list"]
    granted = team.send("kernel", {"type": "update_agent", "id": "tasks", "callable_by": cb})
    v.check("the operator is granted `list` on tasks", granted.get("status") == "completed", str(granted))
    t1 = task_rows(team)
    v.check("tasks lists the recorded issue", "acme/widgets#1" in json.dumps(t1), str(t1))

    team.stop()
    team.start()
    t2 = task_rows(team)
    v.check("after a restart tasks lists the same rows", t2 == t1, f"{t1} vs {t2}")
    v.check("the one store is still the only sqlite_store",
            [i for i, a in agents(team).items() if a.get("handler_module") == "sqlite_store.tools"] == ["memory"])
    team.stop()
    return v.finish()
