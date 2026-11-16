"""One open issue to a draft pull request request, step by step."""

import os
import subprocess

from helpers import forge as F
from helpers import run_flow, team as T
from helpers.verdict import Verdict

BRANCH = "team/issue-1"
TITLE = "Add a greeting file"
BODY = "Adds greeting.txt, checked with cat."


def git(*args, cwd=None):
    if cwd and not os.path.isdir(cwd):  # nothing ran, so there is no repo: None, which no check accepts
        return None
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True,
                       env={"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent",
                            "GIT_CONFIG_NOSYSTEM": "1"})
    return r.stdout.strip()


def run(ctx):
    v = Verdict("task/one_issue_to_draft_pr")
    work = os.path.join(ctx.run_dir, "work")
    bare = os.path.join(ctx.run_dir, "forge", "acme", "widgets.git")
    main_before = git("rev-parse", "main", cwd=bare)
    run_flow.boot(ctx)
    team = ctx.team

    # 1. the task registry pulls the open issue from the tracker as a waiting task
    pulled = team.send("tasks", {"type": "pull"})
    v.check("the registry took the issue", pulled["result"].get("created") == ["acme/widgets#1"],
            str(pulled))
    task = team.send("tasks", {"type": "get", "id": "acme/widgets#1"})["result"]
    v.check("the task waits, held by nobody, with the issue as its source",
            task.get("status") == "waiting" and task.get("holder") is None
            and task.get("source") == "gh:acme/widgets#1", str(task))
    again = team.send("tasks", {"type": "pull"})
    v.check("a second pull adds nothing", again["result"].get("created") == []
            and again["result"].get("skipped") == [["acme/widgets#1", "already a task"]], str(again))

    # 2. the lead plays the script: read the issue, worktree, write, run, commit, push
    played = run_flow.hand_over(ctx, "Do the task acme/widgets#1: its instructions are issue 1 of acme/widgets, read it through github")
    v.check("the scripted model played every turn, none past the end",
            played and ctx.model.consumed, f"served {ctx.model.served} of {len(ctx.model.turns)}")
    seen = T.log_calls(team.log())
    wanted = [("github", "issue_view"), ("git", "worktree_add"), ("worker_files", "write"),
              ("worker_exec", "run"), ("git", "commit"), ("github", "push")]
    v.check("the rows show the calls in order", T.in_order(seen, wanted), str(seen[-40:]))

    # 3. the worktree and the commit
    wt = os.path.join(work, "wt", "greeting")
    v.check("the worktree exists under workspace_root with the new file",
            open(os.path.join(wt, "greeting.txt")).read() == "hello from the team\n"
            if os.path.exists(os.path.join(wt, "greeting.txt")) else False)
    author = git("log", "-1", "--format=%an <%ae>", BRANCH, cwd=wt)
    v.check("the commit on the work branch is by the team bot", author == "Team Bot <team-bot@example.invalid>", author)
    v.check("the commit message is the model's, signed off",
            (git("log", "-1", "--format=%B", BRANCH, cwd=wt) or "").startswith("Add greeting.txt (#1)")
            and "Signed-off-by: Team Bot" in (git("log", "-1", "--format=%B", BRANCH, cwd=wt) or ""))
    v.check("the forge's main is untouched", main_before is not None
            and git("rev-parse", "main", cwd=bare) == main_before)
    v.check("the work branch reached the forge at that commit",
            bool(git("rev-parse", BRANCH, cwd=bare))
            and git("rev-parse", BRANCH, cwd=bare) == git("rev-parse", BRANCH, cwd=wt))
    gitreqs = [r for r in ctx.forge.requests if r["path"].startswith(f"/{F.REPO}.git/")]
    v.check("every git request that got through carried the minted token",
            any(r.get("authorized") for r in gitreqs)
            and all(r.get("authorized") or "authorization" not in r["headers"] for r in gitreqs))

    # 3b. no PR yet: the model never asked for one
    v.check("no pull request was made by the push", len(ctx.forge.pulls()) == 0)

    # 4. the draft PR request the github connector sends
    pr = team.send("github", {"type": "pr_create", "repo": F.REPO, "branch": BRANCH,
                              "title": TITLE, "body": BODY})
    v.check("pr_create answered with the forge's number", pr["result"].get("number") == 7
            and pr["result"].get("draft") is True, str(pr))
    pulls = ctx.forge.pulls()
    v.check("the forge got exactly one POST /pulls", len(pulls) == 1)
    if pulls:
        body = pulls[0]["json"]
        v.check("it is a draft", body.get("draft") is True)
        v.check("head is the work branch, base is main", body.get("head") == BRANCH and body.get("base") == "main", str(body))
        v.check("title and body are the task's", body.get("title") == TITLE and body.get("body", "").startswith(BODY), str(body))
        v.check("it carried the minted installation token",
                pulls[0]["headers"].get("authorization", "").endswith(ctx.forge.token))

    code = team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
    return v.finish()
