"""GitHub auth by configuration (#303): a push and a PR in personal mode, local only with no
credential. App mode is covered by task/github_merge_through_connector."""

import json
import os

from helpers import cli, forge as F, stock
from helpers.verdict import Verdict

LOCAL = "no GitHub credential defined: local work only"


def _work(ctx, team, name):
    """A worktree on `team/<name>` with one commit; returns (path, replies)."""
    wt = os.path.join(ctx.work, "work", "wt", name)
    a = team.send("git", {"type": "worktree_add", "path": wt, "branch": f"team/{name}", "base": "main"})
    with open(os.path.join(wt, f"{name}.txt"), "w") as f:
        f.write(f"{name}\n")
    b = team.send("git", {"type": "commit", "path": wt, "message": f"Add {name}.txt"})
    return wt, (a, b)


def run(ctx):
    v = Verdict("task/github_auth_by_env_ref")

    # ---- personal: the token comes from E2E_GH_TOKEN, no mint ----
    team = cli.start(ctx, sub="personal", env={"E2E_GH_TOKEN": ctx.forge.token})
    stock.make_credentials(ctx, team, github=True, github_mode="personal")
    os.makedirs(os.path.join(ctx.work, "work", "wt"), exist_ok=True)
    stock.make_forge(ctx, team, puller=False)
    forge = ctx.forge
    token = forge.token
    refl = team.send("github", {"type": "reflect"})
    v.check("personal: reflect shows auth.mode personal", refl["result"].get("auth") == {"mode": "personal"}, str(refl)[:300])
    wt, made = _work(ctx, team, "personal")
    v.check("personal: the worktree and the commit were made", all(x.get("status") == "completed" for x in made), str(made)[:300])
    push = team.send("github", {"type": "push", "branch": "team/personal", "path": wt})
    pr = team.send("github", {"type": "pr_create", "repo": F.REPO, "branch": "team/personal",
                              "title": "Add personal", "body": "Adds personal.txt."})
    v.check("personal: the branch is on the forge", bool(forge.rev("refs/heads/team/personal")), str(push)[:300])
    v.check("personal: the git push was authorized",
            any(r["path"].endswith("git-receive-pack") and r.get("authorized") for r in forge.requests))
    posts = forge.to("POST", f"/repos/{F.REPO}/pulls")
    v.check("personal: exactly one draft PR from the branch, sent with the token",
            len(posts) == 1 and posts[0]["headers"].get("authorization", "").endswith(token)
            and forge.prs.get(pr["result"].get("number"), {}).get("draft") is True, str(pr)[:300])
    v.check("personal: no installation lookup and no token mint reached the forge",
            forge.to("GET", f"/repos/{F.REPO}/installation") == []
            and forge.to("POST", "/app/installations/42/access_tokens") == [])
    v.check("personal: the token is in no reply and not in the team's log",
            token not in json.dumps([refl, push, pr, made]) and token not in team.log())
    team.stop()

    # ---- local: no variable, no credential ----
    n0 = len(forge.requests)
    team = cli.start(ctx, sub="local")
    stock.make_credentials(ctx, team, github=True, github_mode="personal")
    os.makedirs(os.path.join(ctx.work, "work", "wt"), exist_ok=True)
    stock.make_forge(ctx, team, puller=False)
    refl = team.send("github", {"type": "reflect"})
    auth = refl["result"].get("auth", {})
    v.check("local: the team booted and reflect shows auth.mode local with the note",
            auth.get("mode") == "local" and LOCAL in auth.get("note", ""), str(refl)[:300])
    wt, made = _work(ctx, team, "local")
    v.check("local: the team works locally (worktree and commit)", all(x.get("status") == "completed" for x in made), str(made)[:300])
    calls = {
        "push": {"type": "push", "branch": "team/local", "path": wt},
        "pr_create": {"type": "pr_create", "repo": F.REPO, "branch": "team/local", "title": "T", "body": "B"},
        "issues": {"type": "issues", "repo": F.REPO},
        "comment": {"type": "comment", "repo": F.REPO, "number": 1, "body": "hi"},
    }
    for name, payload in calls.items():
        r = team.send("github", payload)
        text = json.dumps(r)
        v.check(f"local: {name} says local only, no 401", LOCAL in text and "401" not in text, text[:300])
    v.check("local: nothing reached the forge", len(forge.requests) == n0, f"{len(forge.requests) - n0} requests")
    team.stop()
    return v.finish()
