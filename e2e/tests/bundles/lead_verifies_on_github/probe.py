"""bundles/lead_verifies_on_github: the lead checks a pull request through the github connector
(`pr_status`), not the local clone, and reports the task done from what GitHub says (team#289)."""

from helpers import cli, forge as forge_mod, run_flow, team as T
from helpers.verdict import Verdict

TASK = "Pull request 7 of acme/widgets is the result of your task: verify it, then report."


def run(ctx):
    v = Verdict("bundles/lead_verifies_on_github")
    keys = run_flow.fake_keys(ctx)
    cli.start(ctx, env={"E2E_APP_ID": keys["app_id"], "E2E_APP_PEM": keys["pem"]})
    # the local clone holds nothing of the task: the lead has no exec agent to look there
    ctx.team.send(cli.LEAD, {"type": "send", "text": TASK})
    cli.wait_served(ctx, 60)
    v.check("the scripted model played every turn, none past the end", ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    f, repo = ctx.forge, forge_mod.REPO
    v.check("the forge was asked for the pull request, its check runs and its reviews",
            bool(f.to("GET", f"/repos/{repo}/pulls/7"))
            and bool(f.to("GET", f"/repos/{repo}/commits/{forge_mod.PR_SHA}/check-runs"))
            and bool(f.to("GET", f"/repos/{repo}/pulls/7/reviews")),
            str([(r["method"], r["path"]) for r in f.requests][-8:]))
    bodies = ctx.model.raw_bodies
    v.check("the forge's answer reached the lead (state open, the head sha)",
            len(bodies) > 1 and "open" in bodies[-1] and forge_mod.PR_SHA in bodies[-1], bodies[-1][-300:] if bodies else "")
    calls = T.log_calls(ctx.team.log())
    v.check("the lead made no local command (exec run or start) to verify",
            not any(a == cli.EXEC and verb in ("run", "start") for a, verb in calls), str(calls[-10:]))
    v.check("its last answer reports the task done, not failed",
            "task is done" in ctx.model.turns[-1] and "failed" not in ctx.model.turns[-1])
    ctx.team.stop()
    return v.finish()
