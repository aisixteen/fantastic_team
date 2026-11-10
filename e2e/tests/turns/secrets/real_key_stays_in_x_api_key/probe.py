"""The free secrets test on a real model: the real key is a needle. It goes only in
x-api-key; no request body, no team log, no file of the run (outside secrets/ and
cost.json) holds it, nor the app key or the minted token. Outcomes only."""

import json
import os
import re
import subprocess

from helpers import app, run_flow
from helpers.verdict import Verdict

WAIT_S = 1200.0
SKIP_NAMES = {"cost.json"}  # the relay's record: checked on its own below


def pushed_branches(bare):
    r = subprocess.run(["git", "for-each-ref", "--format=%(refname:short)", "refs/heads/"],
                       cwd=bare, capture_output=True, text=True)
    return [b for b in r.stdout.split() if b != "main"]


def run(ctx):
    v = Verdict("turns/secrets/real_key_stays_in_x_api_key")
    keys = run_flow.boot(ctx)
    team = ctx.team
    bare = os.path.join(ctx.run_dir, "forge", "acme", "widgets.git")
    team.send("puller", {"type": "pull"})
    run_flow.kick(ctx, "Run the checks, then do the task in tasks/queue/acme-widgets-1.md")
    run_flow.wait_done(ctx, lambda: bool(pushed_branches(bare)), wait_s=WAIT_S)
    m = ctx.model
    v.check("the model answered at least once, none errored", m.served >= 1 and not m.errored,
            f"replies {m.served}, errored {m.errored}")

    token = ctx.forge.token
    real = keys["model_key"]
    needles = {"model key": real, "app key": keys["pem_chunk"], "installation token": token,
               "app key header": "BEGIN RSA PRIVATE KEY"}
    bodies = m.raw_bodies

    # what the model was sent: none of the secrets, and only the sandbox as HOME
    for what, needle in needles.items():
        v.check(f"no model request holds the {what}", not any(needle in b for b in bodies))
    v.check("the model key is sent only as x-api-key",
            bool(m.keys) and all(k == real for k in m.keys))
    homes = {h for b in bodies for h in re.findall(r"<line>(?:HOME|XDG_CONFIG_HOME)=([^<]*)</line>", b)}
    v.check("the worker's HOME is the sandbox (./sandbox/home), if the worker printed it",
            homes <= {"./sandbox/home", "./sandbox/home/.config"}, str(homes))
    own = [os.path.join(ctx.run_dir, "home"), os.path.expanduser("~")]
    v.check("neither the team's own HOME nor the user's home reaches a model request",
            not any(p and len(p) > 1 and p in b for p in own for b in bodies))

    # what the team wrote: rows, logs, repos, worktrees (the throwaway key file itself excepted)
    log = team.log()
    for what, needle in needles.items():
        v.check(f"the team's stdout and stderr hold no {what}", needle not in log)
    hits = [f"{p} ({what})" for what, needle in needles.items()
            for p in app.files_holding(ctx.run_dir, needle, skip_files=SKIP_NAMES)]
    v.check("no file of the run holds a key or token (outside secrets/ and cost.json)", not hits,
            "; ".join(hits))
    v.check("the cost record holds no key", real not in json.dumps(m.totals()))

    totals = m.totals()
    print(f"  tokens: {totals['input_tokens']} in, {totals['output_tokens']} out, "
          f"{totals['replies']} replies (cost.json on stop)")
    team.stop()
    return v.finish()
