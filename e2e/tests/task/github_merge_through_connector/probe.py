"""Merge through the connector, sent by a scripted lead (#273, Decision A). The probe makes three
branches and draft PRs itself; the lead, an AI chat granted only `merge` on `github`, merges each.
The green draft is made ready and squash-merged; red and pending checks refuse by name; nothing
else merged."""

import json
import os

from helpers import cli, forge as F, run_flow
from helpers.verdict import Verdict

BRANCHES = {"green": "team/issue-1", "red": "team/issue-2", "pending": "team/issue-3"}


def run(ctx):
    v = Verdict("task/github_merge_through_connector")
    keys = run_flow.fake_keys(ctx)
    team = cli.start(ctx, env={"E2E_APP_ID": keys["app_id"], "E2E_APP_PEM": keys["pem"]})
    os.makedirs(os.path.join(ctx.work, "work", "wt"), exist_ok=True)
    forge = ctx.forge
    # setup, sent directly by the probe: a branch with a file, pushed
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
    ctx.model.bind(green_pr=number["green"], red_pr=number["red"], pending_pr=number["pending"])
    before = forge.rev("main")
    team.send(cli.LEAD, {"type": "send", "text": f"Merge pull requests {number['red']}, {number['pending']} and {number['green']} of {F.REPO} through the github connector, one at a time, in that order, then report which merged."})
    cli.wait_served(ctx, 120)
    bodies = ctx.model.raw_bodies
    g, rd, pd = number["green"], number["red"], number["pending"]
    merges = [r for r in forge.requests if r["method"] == "PUT" and r["path"].endswith("/merge")]
    v.check("the scripted lead played every turn, none past the end", ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    v.check("red: the lead's merge was refused naming the check and its state",
            any("not passed: tests (failure)" in b for b in bodies))
    v.check("pending: the lead's merge was refused naming the check and its state",
            any("not passed: tests (in_progress)" in b for b in bodies))
    v.check("red and pending: the PRs are still open",
            forge.prs[rd]["state"] == "open" and forge.prs[pd]["state"] == "open")
    v.check("no merge request reached the forge for the red or pending PR",
            not any(r["path"].endswith(f"/pulls/{n}/merge") for r in merges for n in (rd, pd)))
    v.check("the forge got exactly one merge, for the green PR",
            len(merges) == 1 and merges[0]["path"] == f"/repos/{F.REPO}/pulls/{g}/merge",
            str([m["path"] for m in merges]))
    v.check("the merge reached the forge as the app (the minted installation token)",
            bool(merges) and merges[0]["headers"].get("authorization", "").endswith(forge.token),
            merges[0]["headers"].get("authorization", "")[:12] if merges else "no merge")
    v.check("the green draft PR was made ready before the merge",
            forge.prs[g]["draft"] is False and len(forge.to("POST", "/graphql")) == 1)
    v.check("the green PR is closed", forge.prs[g]["state"] == "closed")
    v.check("the forge's main has one new squashed commit with the PR's title",
            forge.main_log()[:2] == [f"Add green (#{g})", "seed"] and forge.rev("main") != before,
            str(forge.main_log()))
    v.check("the merge's answer reached the lead (the squash commit's sha)",
            forge.rev("main") != before and any(forge.rev("main") in b for b in bodies))
    v.check("the squashed commit holds the PR's file", forge.rev("main:green.txt") != "")
    v.check("the red and pending files are not on main",
            not forge.rev("main:red.txt") and not forge.rev("main:pending.txt"))
    token = forge.token
    v.check("no reply holds the token", token not in json.dumps(replies))
    v.check("no model request held the token", not any(token in b for b in bodies))
    v.check("the team's output never held the token", token not in team.log())
    team.stop()
    return v.finish()
