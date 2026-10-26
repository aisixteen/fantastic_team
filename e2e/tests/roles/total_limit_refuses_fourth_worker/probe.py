"""A total limit of 3: the fourth spawn is refused with its reason, nothing more is started."""

import os
import shutil

from helpers import run_flow
from helpers.verdict import Verdict

REASON = "all roles together are at the limit (3 running, max 3)"
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))


def run(ctx):
    v = Verdict("roles/total_limit_refuses_fourth_worker")
    shutil.copytree(os.path.join(ROOT, "roles"), os.path.join(ctx.run_dir, "roles"))
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    team = ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                          "E2E_APP_PEM": keys["pem"]},
                         wire=["credentials", "github", "worker_files"])

    roles = team.send("roles", {"type": "roles"})["result"]
    names = sorted(r["name"] for r in roles.get("roles", []))
    v.check("the registry lists the six cards and no error",
            names == ["issue_keeper", "issue_writer", "lead", "project_lead", "spec_reviewer", "worker"]
            and roles.get("errors") == [], str(roles))
    before = team.send("roles", {"type": "get_count"})["result"]
    v.check("the boot limit is in: max 3, none running", before == {"running": 0, "max": 3}, str(before))

    played = run_flow.hand_over(ctx, "Start four workers.")
    v.check("the scripted model played every turn, none past the end",
            played and ctx.model.consumed, f"served {ctx.model.served} of {len(ctx.model.turns)}")

    count = team.send("roles", {"type": "get_count"})["result"]
    v.check("get_count is {running: 3, max: 3}", count == {"running": 3, "max": 3}, str(count))
    fleet = team.send("roles", {"type": "fleet"})["result"]
    workers = fleet.get("roles", {}).get("worker", [])
    v.check("fleet lists three workers", len(workers) == 3, str(fleet))

    bodies = ctx.model.raw_bodies
    # request i+1 carries the result of spawn i+1
    v.check("the model got five requests", len(bodies) == 5, f"{len(bodies)}")
    if len(bodies) >= 5:
        v.check("the third spawn was not refused", "at the limit" not in bodies[3], "")
        v.check("the fourth spawn's result is the limit refusal with its reason",
                REASON in bodies[4] and "limit" in bodies[4], "")
    v.check("a refused spawn made no fourth worker", len({w["id"] for w in workers}) == 3)

    side = team.send("dispatcher", {"type": "sidebar"})["result"]
    v.check("the dispatcher sidebar carries fleet {running: 3, max: 3}",
            side.get("fleet") == {"running": 3, "max": 3}, str(side.get("fleet")))
    team.stop()
    return v.finish()
