"""The dispatcher adds a worker to a group, hands it a task, and removes it again."""

import json

from helpers import run_flow
from helpers.verdict import Verdict


def ids_of(listing: dict) -> list:
    agents = listing.get("result", {}).get("agents", [])
    return [a if isinstance(a, str) else a.get("id") for a in agents]


def control(team, **kw) -> dict:
    return team.send("dispatcher", dict(kw, type="control"))


def tree_ids(team) -> list:
    """The ids beside w1 in the tree the page's sidebar draws (the root's full `reflect`): the
    children of w1's parent."""
    tree = team.send("kernel", {"type": "reflect", "detail": "full"}).get("result", {}).get("tree") or {}

    def parent_of(node, id_):
        kids = node.get("children") or []
        if any(k.get("id") == id_ for k in kids):
            return node
        for k in kids:
            found = parent_of(k, id_)
            if found is not None:
                return found
        return None

    at = parent_of(tree, "w1") if isinstance(tree, dict) else None
    return [k.get("id") for k in (at or {}).get("children") or []]


def group_g(team) -> list:
    """The members of group g, as the dispatcher's own record (`list_agents`) holds them."""
    agents = team.send("kernel", {"type": "list_agents"}).get("result", {}).get("agents", [])
    rec = next((a for a in agents if isinstance(a, dict) and a.get("id") == "dispatcher"), {})
    return list((rec.get("groups") or {}).get("g") or [])


def run(ctx):
    v = Verdict("dispatcher/hand_task_add_worker")
    run_flow.boot(ctx)
    team = ctx.team

    added = control(team, node="g", action="add_worker", id="w2", template="lead_t").get("result", {})
    v.check("add_worker returns the new id and no error", added.get("id") == "w2" and "error" not in added,
            json.dumps(added))
    v.check("the tree lists w2 beside w1", "w2" in tree_ids(team) and "w1" in tree_ids(team),
            str(tree_ids(team)))
    v.check("the dispatcher's record lists w2 in group g beside w1", group_g(team) == ["w1", "w2"],
            str(group_g(team)))
    live = ids_of(team.send("kernel", {"type": "list_agents"}))
    v.check("the worker exists and the team's one store serves it, no memory of its own",
            "w2" in live and "memory" in live and "w2_memory" not in live, str(live))

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
    v.check("group g no longer lists w2", group_g(team) == ["w1"], str(group_g(team)))
    team.stop()
    return v.finish()
