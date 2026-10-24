"""A run with fake keys: none of them is in a model request, the rows, the logs
or the worker's output; the model key goes only in x-api-key and the minted token
only to the forge."""

import base64
import os
import re
import subprocess

from helpers import forge as F
from helpers import run_flow
from helpers.verdict import Verdict


def files_holding(root, needles, skip=()):
    """Paths under `root` whose bytes hold any needle (binary files too)."""
    hits = []
    for base, dirs, names in os.walk(root):
        dirs[:] = [d for d in dirs if os.path.join(base, d) not in skip]
        for n in names:
            p = os.path.join(base, n)
            try:
                data = open(p, "rb").read()
            except OSError:
                continue
            for what, needle in needles.items():
                if needle.encode() in data:
                    hits.append(f"{os.path.relpath(p, root)} ({what})")
    return hits


def run(ctx):
    v = Verdict("secrets/worker_never_sees_tokens")
    keys = run_flow.boot(ctx)
    team = ctx.team
    team.send("puller", {"type": "pull"})
    played = run_flow.hand_over(ctx, "Run the checks, then do the task in tasks/queue/acme-widgets-1.md")
    v.check("the scripted model played every turn", played and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    token = ctx.forge.token
    needles = {"model key": keys["model_key"], "app key": keys["pem_chunk"], "installation token": token,
               "app key header": "BEGIN RSA PRIVATE KEY"}

    # what the worker saw: the replies the model was handed
    bodies = ctx.model.raw_bodies
    last = bodies[-1] if bodies else ""
    v.check("the worker's commands ran and printed their output",
            "HOME=" in last and os.path.join("sandbox", "home") in last)
    # the team writes the sandbox as a path relative to its folder (`./sandbox/home`)
    v.check("the worker's HOME is the sandbox (./sandbox/home)",
            any("<line>HOME=./sandbox/home</line>" in b for b in bodies))
    homes = {m for b in bodies for m in re.findall(r"<line>(?:HOME|XDG_CONFIG_HOME)=([^<]*)</line>", b)}
    v.check("neither the user's home nor the team's own HOME is the worker's",
            homes == {"./sandbox/home", "./sandbox/home/.config"}, str(homes))
    for what, needle in needles.items():
        v.check(f"no model request holds the {what}", not any(needle in b for b in bodies))
    v.check("the worker's environment names no credential variable (E2E_*)", "E2E_" not in last)
    v.check("the model key is sent only as x-api-key",
            ctx.model.keys and all(k == keys["model_key"] for k in ctx.model.keys))

    # rows and logs: everything the run wrote, but the throwaway key file itself
    log = team.log()
    for what, needle in needles.items():
        v.check(f"the team's stdout and stderr hold no {what}", needle not in log)
    skip = {os.path.join(ctx.run_dir, "secrets")}
    hits = files_holding(ctx.run_dir, needles, skip)
    v.check("no file of the run holds a key or token (rows, records, repos, worktree)", not hits, "; ".join(hits))
    cfg = os.path.join(ctx.run_dir, "clone", ".git", "config")
    wt_cfg = subprocess.run(["git", "-C", os.path.join(ctx.run_dir, "work", "wt", "greeting"),
                             "config", "--list"], capture_output=True, text=True).stdout
    v.check("neither .git/config holds the token", token not in open(cfg).read() and token not in wt_cfg)

    # the token went where it should: the forge's git endpoint, once the push needed it
    pushes = [r for r in ctx.forge.requests if r["path"] == f"/{F.REPO}.git/git-receive-pack"]
    want = "Basic " + base64.b64encode(f"x-access-token:{token}".encode()).decode()
    v.check("the minted token reached the forge in the git push only",
            len(pushes) == 1 and pushes[0]["headers"].get("authorization") == want)

    # the repo's own scanner over the run
    subprocess.run(["git", "init", "-q"], cwd=ctx.run_dir, capture_output=True)
    scan = subprocess.run(["sh", os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                              *[".."] * 4, "scripts", "check-secrets.sh"), ctx.run_dir],
                          capture_output=True, text=True)
    v.check("scripts/check-secrets.sh finds nothing in the run", scan.returncode == 0, scan.stderr)

    team.stop()
    return v.finish()
