"""Merge through the connector. A green draft PR is made ready and
squash-merged; red and pending checks refuse it by name, nothing merged."""

import json
import os

from helpers import forge as F
from helpers import run_flow
from helpers.verdict import Verdict

BRANCHES = {"green": "team/issue-1", "red": "team/issue-2", "pending": "team/issue-3"}


def run(ctx):
    v = Verdict("task/github_merge_through_connector")
    run_flow.boot(ctx)
    team, forge = ctx.team, ctx.forge
    # the team's own verbs, sent as the lead would call them: a branch with a file, pushed
    for name, branch in BRANCHES.items():
        wt = os.path.join(ctx.run_dir, "work", "wt", name)
        a = team.send("git", {"type": "worktree_add", "path": wt, "branch": branch, "base": "main"})
        with open(os.path.join(wt, f"{name}.txt"), "w") as f:
            f.write(f"{name}\n")
        b = team.send("git", {"type": "commit", "path": wt, "message": f"Add {name}.txt"})
        c = team.send("github", {"type": "push", "branch": branch, "path": wt})
        v.check(f"{name}: the branch was made, committed and pushed",
                all(x.get("status") == "completed" for x in (a, b, c)) and bool(forge.rev(f"refs/heads/{branch}")),
                str((a, b, c))[:300])
    forge.checks[BRANCHES["red"]] = [("lint", "completed", "success"), ("tests", "completed", "failure")]
    forge.checks[BRANCHES["pending"]] = [("tests", "in_progress", None)]
    replies, number = [], {}
    for name, branch in BRANCHES.items():
        pr = team.send("github", {"type": "pr_create", "repo": F.REPO, "branch": branch,
                                  "title": f"Add {name}", "body": f"Adds {name}.txt."})
        replies.append(pr)
        number[name] = pr["result"].get("number")
        v.check(f"{name}: a draft PR was opened", number[name] in forge.prs and forge.prs[number[name]]["draft"], str(pr))
    before = forge.rev("main")
    merges = lambda: forge.to("PUT", f"/repos/{F.REPO}/pulls/{number['green']}/merge")
    for name in ("red", "pending"):
        r = team.send("github", {"type": "merge", "repo": F.REPO, "number": number[name]})
        replies.append(r)
        text = json.dumps(r)
        check = "tests"
        v.check(f"{name}: merge is refused naming the check", "error" in json.dumps(r["result"]) and check in text, text[:300])
        v.check(f"{name}: the PR is still open and main did not move",
                forge.prs[number[name]]["state"] == "open" and forge.rev("main") == before)
    v.check("no merge request reached the forge for the red or pending PR",
            not any(r["method"] == "PUT" and r["path"].endswith("/merge") for r in forge.requests))
    r = team.send("github", {"type": "merge", "repo": F.REPO, "number": number["green"]})
    replies.append(r)
    v.check("the green draft PR merged", r["result"].get("merged") is True, str(r))
    v.check("it was made ready before the merge", forge.prs[number["green"]]["draft"] is False
            and len(forge.to("POST", "/graphql")) == 1)
    v.check("the forge's main has one new squashed commit with the PR's title",
            forge.main_log()[:2] == [f"Add green (#{number['green']})", "seed"], str(forge.main_log()))
    v.check("the squashed commit holds the PR's file", forge.rev("main:green.txt") != "")
    v.check("the red and pending files are not on main", not forge.rev("main:red.txt")
            and not forge.rev("main:pending.txt"))
    token = forge.token
    v.check("no reply holds the token", token not in json.dumps(replies))
    v.check("no model request held the token", not any(token in b for b in ctx.model.raw_bodies))
    v.check("the team's output never held the token", token not in team.log())
    team.stop()
    return v.finish()
