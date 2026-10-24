"""A real model does the small spec: pull the issue, hand the lead its task, wait for a
pushed work branch, then check outcomes only. A real model may order its calls
differently from the scripted one, so the call order is not asserted."""

import os
import subprocess

from helpers import forge as F
from helpers import run_flow
from helpers.verdict import Verdict

TITLE = "Add a greeting file"
BODY = "Adds greeting.txt, checked with cat."
GREETING = "greeting.txt"  # named in the seeded spec
WAIT_S = 1200.0  # the model's time; the test's timeout is larger


def git(*args, cwd=None):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True,
                       env={"PATH": os.environ.get("PATH", ""), "HOME": "/nonexistent",
                            "GIT_CONFIG_NOSYSTEM": "1"})
    return r.stdout.strip()


def pushed_branches(bare: str) -> list[str]:
    refs = git("for-each-ref", "--format=%(refname:short)", "refs/heads/", cwd=bare).split()
    return [r for r in refs if r != "main"]


def run(ctx):
    v = Verdict("turns/task/worker_finishes_small_spec")
    bare = os.path.join(ctx.run_dir, "forge", "acme", "widgets.git")
    main_before = git("rev-parse", "main", cwd=bare)
    run_flow.boot(ctx)
    team = ctx.team

    # 1. the puller turns the labelled issue into a task file
    pulled = team.send("puller", {"type": "pull"})
    v.check("the puller took the issue", pulled["result"].get("created") == ["acme/widgets#1"],
            str(pulled))

    # 2. the lead gets its task; the model works on its own until it pushes a branch
    run_flow.kick(ctx, "Do the task in tasks/queue/acme-widgets-1.md")
    pushed = run_flow.wait_done(ctx, lambda: bool(pushed_branches(bare)), wait_s=WAIT_S)
    m = ctx.model
    v.check("the model pushed a work branch before the wait ran out", pushed,
            f"replies {m.served}, errored {m.errored}, kick {ctx.kicked}")
    branches = pushed_branches(bare)
    branch = branches[0] if branches else ""

    # 3. outcomes on the forge
    v.check("a branch other than main exists on the forge", bool(branch), str(branches))
    commits = git("rev-list", "--count", f"main..{branch}", cwd=bare) if branch else "0"
    v.check("the branch has at least one commit of its own", commits.isdigit() and int(commits) >= 1,
            commits)
    v.check("the forge's main is unchanged", git("rev-parse", "main", cwd=bare) == main_before)
    greeting = git("show", f"{branch}:{GREETING}", cwd=bare) if branch else ""
    v.check(f"{GREETING} is on the branch and says hello", "hello" in greeting.lower(), greeting[:200])
    author = git("log", "-1", "--format=%an <%ae>", branch, cwd=bare) if branch else ""
    v.check("the commit author is the team bot", author == "Team Bot <team-bot@example.invalid>",
            author)

    # 4. the draft PR request, sent as the operator (see README item 5)
    pr = team.send("github", {"type": "pr_create", "repo": F.REPO, "branch": branch,
                              "title": TITLE, "body": BODY})
    v.check("pr_create answered with a draft", pr["result"].get("draft") is True, str(pr))
    pulls = ctx.forge.pulls()
    v.check("the forge got exactly one POST /pulls, a draft with that head",
            len(pulls) == 1 and pulls[0]["json"].get("draft") is True
            and pulls[0]["json"].get("head") == branch, str(pulls))

    # 5. what it cost (cost.json is written when the plug stops)
    totals = m.totals()
    print(f"  tokens: {totals['input_tokens']} in, {totals['output_tokens']} out, "
          f"{totals['replies']} replies (cost.json on stop)")

    code = team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
    return v.finish()
