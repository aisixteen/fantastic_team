"""GitHub build: the connector pushes and opens the draft PR as the app; the worker
never held the token."""

import os

from helpers import forge as F
from helpers import run_flow
from helpers.verdict import Verdict

BRANCH = "team/issue-1"


def run(ctx):
    v = Verdict("task/github_pr_through_connector")
    run_flow.boot(ctx)
    team = ctx.team
    pulled = team.send("puller", {"type": "pull"})
    v.check("the puller took the issue", pulled["result"].get("created") == ["acme/widgets#1"], str(pulled))
    played = run_flow.hand_over(ctx, "Do the task in tasks/queue/acme-widgets-1.md")
    v.check("the scripted model played every turn", played and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    token = ctx.forge.token
    pr = team.send("github", {"type": "pr_create", "repo": F.REPO, "branch": BRANCH,
                              "title": "Add a greeting file", "body": "Adds greeting.txt."})
    v.check("pr_create answered with the forge's number", pr["result"].get("number") == 7, str(pr))
    pulls = ctx.forge.pulls()
    v.check("the forge got exactly one POST /pulls", len(pulls) == 1)
    if pulls:
        auth = pulls[0]["headers"].get("authorization", "")
        v.check("it is a draft from the work branch", pulls[0]["json"].get("draft") is True
                and pulls[0]["json"].get("head") == BRANCH)
        v.check("it was opened as the app, with the minted installation token", auth.endswith(token), auth[:12])
    bodies = ctx.model.raw_bodies
    v.check("no model request ever held the token", not any(token in b for b in bodies))
    v.check("the worker's env printed no token or key", "E2E_" not in "".join(bodies)
            and ctx.secrets["model_key"] not in "".join(bodies))
    v.check("the team's output never held the token", token not in team.log())
    wt = os.path.join(ctx.run_dir, "work", "wt", "greeting")
    for cfg in (os.path.join(ctx.run_dir, "clone", ".git", "config"),):
        v.check("the clone's .git/config holds no token", token not in open(cfg).read())
    v.check("the work branch reached the forge", any(r["path"].endswith("git-receive-pack") and r.get("authorized")
                                                     for r in ctx.forge.requests))
    team.stop()
    return v.finish()
