"""The coder project_lead: reads, proposes, waits for "yes", grows the tree, two PRs, nothing cleaned up."""

import os
import shutil
import time

from helpers import run_flow, stock as S, team as T
from helpers.verdict import Verdict

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))
TITLES = ["Add a greeting file", "Add a greeting file 2", "Add a greeting file 3", "Add a greeting file 4"]


def agents(team):
    return {a["id"]: a for a in team.send("kernel", {"type": "list_agents"})["result"]["agents"]}


def admit(team, who, tools):
    """The lead's admit step, as the operator: name `who` in each tool's whole `callable_by`."""
    have = agents(team)
    for t in tools:
        cb = dict(have[t].get("callable_by") or {})
        cb[who] = "*"
        r = team.send("kernel", {"type": "update_agent", "id": t, "callable_by": cb})
        if r.get("status") != "completed":
            raise RuntimeError(f"cannot admit {who} on {t}: {r}")


def fleet(team):
    return team.send("roles", {"type": "fleet"})["result"].get("roles", {})


def workers(team):
    return fleet(team).get("worker", [])


def settle(ctx, served, limit=60.0):
    """Wait until the model has served `served` turns and then stays quiet."""
    end = time.time() + limit
    while time.time() < end and ctx.model.served < served:
        time.sleep(0.2)
    return run_flow.wait_model_quiet(ctx.model, quiet=2.5, limit=20.0)


def run(ctx):
    v = Verdict("roles/project_lead_plans_and_runs_issues")
    forge = ctx.forge
    forge.extra_issues = 3  # four open issues in all
    team = ctx.boot_team(cards=True)

    # the probe plays the user: it starts the project_lead through the registry, naming the team's
    # provider (the fleet's `provider`) and the model; its workers take the same ones. The operator
    # holds what the card grants first (`S.spawn_project_lead`), and gives the spawned lead the team's store.
    # this team's workers may also use the GitHub connector: the worker bundle, edited in place
    w = os.path.join(team.dir, ".fantastic", "bundles", "worker", "bundle.yaml")
    text = open(w).read()
    open(w, "w").write(text.replace("    worker_files: '*'\n", "    worker_files: '*'\n    github: [issue_view, pr_create, pr_status]\n", 1))
    team.send("kernel", {"type": "reload_bundle", "name": "worker"})
    made = S.spawn_project_lead(ctx, team)
    pl = made.get("id")
    v.check("the registry spawned the project_lead", bool(pl) and made.get("model") == "scripted-model", str(made))
    if not pl:
        team.stop()
        return v.finish()
    # a worker tells its parent with a plain `send` (team#247): the script names the
    # project_lead, whose id is minted only now
    ctx.model.bind(pl=pl)
    # the operator names the project_lead on the tools it was given (a spawned
    # agent is not named in them, and only the tools' parent or the operator can write them)
    admit(team, pl, ["github", "queue", "board"])

    # 1. "do your best choice": it reads, then proposes and waits
    team.send(pl, {"type": "send", "text": "do your best choice"})
    settle(ctx, 2)
    v.check("it asked the model twice (read, then propose)", ctx.model.served == 2, str(ctx.model.served))
    seen = T.log_calls(team.log())
    for want in (("github", "issues"), ("roles", "get_count")):
        v.check(f"it called {want[0]} {want[1]}", want in seen, str(seen[-20:]))
    proposal = ctx.model.turns[1]  # the say text, then the done call
    v.check("the proposal names the four issues",
            all(f"#{i + 1} {TITLES[i]}" in proposal for i in range(4)), proposal[:200])
    v.check("before yes no worker exists", workers(team) == []
            and team.send("roles", {"type": "get_count"})["result"]["running"] == 1,
            str(fleet(team)))

    # 2. "yes": worker 1, then (on "continue") worker 2. A spawned worker's first message is
    # given no task (the spawn has none), so it asks the model nothing; the probe plays the
    # lead's admit step (the connector and queue it was given), sends the task
    # again, and the worker plays its turns: it sends its parent a message (`send`, which
    # spawn let it make), the message starts the project_lead's turn, then the worker closes.
    handled = []
    for n in (1, 2):
        before = 0 if n == 1 else 7  # turns served before this round
        team.send(pl, {"type": "send", "text": "yes" if n == 1 else "continue"})
        settle(ctx, before + 4 if n == 1 else before + 2)  # the project_lead's two turns
        fresh = [x["id"] for x in workers(team) if x["id"] not in handled]
        v.check(f"worker {n} was started", len(fresh) == 1, str(fleet(team)))
        if len(fresh) != 1:
            break
        wid = fresh[0]
        handled.append(wid)
        team.wait_for(lambda: f"{wid} call send" in team.log(), 20.0)
        time.sleep(1.0)
        started = before + (4 if n == 1 else 2)
        v.check(f"worker {n} asked the model nothing before it was admitted",
                ctx.model.served == started, str(ctx.model.served))
        gated = agents(team)
        v.check(f"worker {n} is named on neither the connector nor the queue before the lead admits it",
                all(wid not in (gated[t].get("callable_by") or {}) for t in ("github", "queue")),
                str({t: gated[t].get("callable_by") for t in ("github", "queue")}))
        admit(team, wid, ["github", "queue"])
        opened_to = agents(team)
        v.check(f"worker {n} is named on both after the admit",
                all((opened_to[t].get("callable_by") or {}).get(wid) in ("*", ["*"]) for t in ("github", "queue")),
                str({t: opened_to[t].get("callable_by") for t in ("github", "queue")}))
        task = f"The queue is queue. Your id is the id in your own reflect. Work on issue {n}."
        team.send(wid, {"type": "send", "text": task})
        settle(ctx, started + 3)  # the worker's send, the project_lead's turn it starts, the worker's close
        woke = ctx.model.raw_bodies[started + 1] if len(ctx.model.raw_bodies) > started + 1 else ""
        v.check(f"worker {n}'s send to its parent started the project_lead's turn",
                f"Issue {n} is done: PR opened for issue {n}." in woke and all(t in woke for t in TITLES),
                woke[-300:])
        # `title` is a routing field of a model call, so a worker's call cannot carry the
        # connector's `title` argument (see the README): the PR is opened through the
        # connector as the operator once the worker has told its parent.
        opened = team.send("github", {"type": "pr_create", "repo": "acme/widgets",
                                      "branch": f"team/issue-{n}", "title": TITLES[n - 1],
                                      "body": f"Work for issue {n}."})["result"]
        v.check(f"the connector opened the PR for issue {n}", opened.get("draft") is True, str(opened))
    # 3. the last word
    team.send(pl, {"type": "send", "text": "continue"})
    settle(ctx, len(ctx.model.turns))

    v.check("the scripted model played every turn, none past the end",
            ctx.model.consumed, f"served {ctx.model.served} of {len(ctx.model.turns)}")
    bodies = ctx.model.raw_bodies
    v.check("the proposal stayed in the project_lead's next request",
            len(bodies) > 2 and all(TITLES[i] in bodies[2] for i in range(4)))
    pulls = forge.pulls()
    heads = sorted(p["json"].get("head") for p in pulls)
    v.check("the forge recorded two draft PRs, one per worker",
            heads == ["team/issue-1", "team/issue-2"] and all(p["json"].get("draft") for p in pulls), str(heads))
    ws = workers(team)
    v.check("fleet lists both workers", len(ws) == 2 and all(x.get("id") for x in ws), str(ws))
    have = agents(team)
    v.check("the project_lead and both workers are still in the tree",
            pl in have and all(x["id"] in have for x in ws))
    v.check("nothing was deleted", "delete_agent" not in team.log())
    v.check("the last answer ticks the plan in place", "[x] #1" in ctx.model.turns[-1] and "[x] #2" in ctx.model.turns[-1])
    print("  note: the dispatcher chat entry, the plan view and the approval gate are LATER (UI); skipped")
    team.stop()
    return v.finish()
