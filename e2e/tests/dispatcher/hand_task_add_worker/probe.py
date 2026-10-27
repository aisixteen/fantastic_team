"""The dispatcher adds a worker to a group, hands it a task, and removes it again."""

import json

from helpers import run_flow, team as T
from helpers.verdict import Verdict


def ids_of(listing: dict) -> list:
    agents = listing.get("result", {}).get("agents", [])
    return [a if isinstance(a, str) else a.get("id") for a in agents]


def control(team, **kw) -> dict:
    return team.send("dispatcher", dict(kw, type="control"))


def tree_ids(team) -> list:
    tree = team.send("dispatcher", {"type": "tree", "node": "g"}).get("result", {})
    return [c.get("row", {}).get("id") for c in tree.get("children", [])]


def board_members(team) -> list:
    """The workers the board's rows put under group g."""
    rows = team.send("board", {"type": "rows"}).get("result", {}).get("rows", [])
    return [r["id"] for r in rows if r.get("parent") == "g"]


def run(ctx):
    v = Verdict("dispatcher/hand_task_add_worker")
    run_flow.boot(ctx)
    T.wire_dispatcher(ctx.team)
    team = ctx.team

    added = control(team, node="g", action="add_worker", id="w2", template="lead_t").get("result", {})
    v.check("add_worker returns the new id and no error", added.get("id") == "w2" and "error" not in added,
            json.dumps(added))
    v.check("the tree of g lists w2 beside w1", "w2" in tree_ids(team) and "w1" in tree_ids(team),
            str(tree_ids(team)))
    live = ids_of(team.send("kernel", {"type": "list_agents"}))
    v.check("the worker exists and the team's one store serves it, no memory of its own",
            "w2" in live and "memory" in live and "w2_memory" not in live, str(live))
    v.check("the board's rows put w2 under g", "w2" in board_members(team), str(board_members(team)))

    handed = control(team, node="w2", action="hand_task", task="fix issue 1").get("result", {})
    v.check("hand_task is sent, with no error", handed.get("sent") is True and "error" not in handed,
            json.dumps(handed))
    body = run_flow.wait_model_body(ctx, "task fix issue 1", 30)
    v.check("the worker's model got `task fix issue 1`", body is not None)

    asked = control(team, node="w2", action="remove_worker").get("result", {})
    v.check("remove_worker without confirm asks first",
            "Remove w2" in str(asked.get("question", "")) and asked.get("sent") is False, json.dumps(asked))
    v.check("the worker is still there after the question",
            "w2" in ids_of(team.send("kernel", {"type": "list_agents"})))
    gone = control(team, node="w2", action="remove_worker", confirm=True).get("result", {})
    v.check("remove_worker with confirm is sent, with no error",
            gone.get("sent") is True and "error" not in gone, json.dumps(gone))
    live = ids_of(team.send("kernel", {"type": "list_agents"}))
    v.check("the worker is gone and the team's one store stays",
            "w2" not in live and "w2_memory" not in live and "memory" in live, str(live))
    v.check("the tree no longer lists w2", "w2" not in tree_ids(team), str(tree_ids(team)))
    v.check("the board's rows no longer have w2", "w2" not in board_members(team)
            and "w1" in board_members(team), str(board_members(team)))
    team.stop()
    return v.finish()
