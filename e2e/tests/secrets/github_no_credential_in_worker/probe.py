"""GitHub build: a worker's env and HOME hold no token, model key or PEM, and the
forge CLI and credential helper it might use to get one are refused."""

import re

from helpers import run_flow
from helpers.verdict import Verdict

REFUSAL = "command not allowed"


def run(ctx):
    v = Verdict("secrets/github_no_credential_in_worker")
    keys = run_flow.boot(ctx)
    played = run_flow.hand_over(ctx, "Inspect your environment, then do what the script says")
    v.check("the scripted model played every turn", played and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    bodies = ctx.model.raw_bodies
    last = bodies[-1] if bodies else ""
    needles = {"model key": keys["model_key"], "app key": keys["pem_chunk"],
               "installation token": ctx.forge.token, "app key header": "BEGIN RSA PRIVATE KEY"}
    v.check("the worker's commands ran and printed their output", "HOME=" in last)
    homes = {m for b in bodies for m in re.findall(r"<line>(?:HOME|XDG_CONFIG_HOME)=([^<]*)</line>", b)}
    v.check("the worker's HOME is the sandbox", homes == {"./sandbox/home", "./sandbox/home/.config"}, str(homes))
    for what, needle in needles.items():
        v.check(f"no model request holds the {what}", not any(needle in b for b in bodies))
        v.check(f"the team's output holds no {what}", needle not in ctx.team.log())
    v.check("the worker's environment names no credential variable (E2E_*)", "E2E_" not in last)
    v.check("gh auth token and a credential.helper setting are refused", last.count(REFUSAL) == 2,
            f"{last.count(REFUSAL)} refusals")
    ctx.team.stop()
    return v.finish()
