"""One store agent: every AI agent's memory and the task registry use it, and its rows survive a restart."""

import json
import os
import shutil

from helpers import run_flow
from helpers.verdict import Verdict

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))


def agents(team):
    r = team.send("kernel", {"type": "list_agents"})["result"]
    return {a["id"]: a for a in r["agents"]}


def history(team):
    """The lead's chat rows as (role, content) pairs, ids dropped."""
    r = team.send("lead", {"type": "history"})["result"]
    return r, [json.dumps({k: v for k, v in row.items() if k != "id"}, sort_keys=True)
               for row in r.get("rows", [])]


def task_rows(team):
    return team.send("tasks", {"type": "list"})["result"]


def run(ctx):
    v = Verdict("roles/one_store_survives_restart")
    shutil.copytree(os.path.join(ROOT, "roles"), os.path.join(ctx.run_dir, "roles"))
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    team = ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                          "E2E_APP_PEM": keys["pem"]},
                         wire=["credentials", "github", "worker_files"])

    live = agents(team)
    stores = [i for i, a in live.items() if a.get("handler_module") == "sqlite_store.tools"]
    v.check("the agent list holds exactly one sqlite_store, named memory", stores == ["memory"], str(stores))
    extra = [i for i in live if i.endswith("_memory") and i != "kernel_memory"]
    v.check("no per-agent *_memory agent exists", extra == [], str(extra))
    v.check("the store's file is .fantastic/memory.db", live["memory"].get("path") == ".fantastic/memory.db",
            str(live["memory"].get("path")))
    v.check("the lead's memory_store_id names it", live["lead"].get("memory_store_id") == "memory",
            str(live["lead"].get("memory_store_id")))
    tiers = live["roles"].get("tiers", {})
    v.check("every tier's memory_store_id names it",
            len(tiers) == 3 and all(t.get("memory_store_id") == "memory" for t in tiers.values()), str(tiers))
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
    v.check("the store admits the lead and tasks", all(by.get(a) in ("*", ["*"]) for a in ("lead", "tasks")), str(by))
    v.check("the spawn made no *_memory agent",
            [i for i in live if i.endswith("_memory") and i != "kernel_memory"] == [])

    db = os.path.join(ctx.run_dir, ".fantastic", "memory.db")
    v.check(".fantastic/memory.db exists in the team directory", os.path.isfile(db), db)

    # the operator may read what the lead keeps: the chat verbs on the lead, and `list` on tasks
    cb = dict(live["tasks"].get("callable_by") or {})
    cb["mcp"] = ["list"]
    granted = team.send("kernel", {"type": "update_agent", "id": "tasks", "callable_by": cb})
    v.check("the operator is granted `list` on tasks", granted.get("status") == "completed", str(granted))
    h1, rows1 = history(team)
    t1 = task_rows(team)
    v.check("the lead has history rows", len(rows1) >= 3 and "error" not in h1, str(h1)[:300])
    v.check("tasks lists the recorded issue", "acme/widgets#1" in json.dumps(t1), str(t1))

    team.stop()
    team.start()
    h2, rows2 = history(team)
    t2 = task_rows(team)
    v.check("after a restart the lead's history rows are back, unchanged", rows2 == rows1 and rows1,
            f"{len(rows1)} before, {len(rows2)} after")
    v.check("after a restart tasks lists the same rows", t2 == t1, f"{t1} vs {t2}")
    v.check("the one store is still the only sqlite_store",
            [i for i, a in agents(team).items() if a.get("handler_module") == "sqlite_store.tools"] == ["memory"])
    team.stop()
    return v.finish()
