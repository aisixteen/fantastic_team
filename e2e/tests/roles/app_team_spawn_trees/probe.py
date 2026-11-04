"""roles/app_team_spawn_trees: the team the app makes runs lead -> sub-lead -> worker and
lead -> project_lead -> worker, and each parent deletes what it spawned (#227)."""

import os
import signal
import time

from helpers import app, cli, records, run_flow, team as team_mod
from helpers.verdict import Verdict

TASK = "Run a sub-lead and a project lead, each with a worker; delete what you spawned."
REPO_ROLES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "..", "roles")


def seed(ctx, folder, env, v):
    """Run the binary once in `folder` so it is seeded, make the home's credential store (the
    model's key from the environment), and stop it: what the app needs in a folder with no team."""
    port = app.free_port()
    proc = app.Proc([ctx.binary, "canvas", "--port", str(port)], env, folder,
                    os.path.join(ctx.run_dir, "seed.log"))
    try:
        v.check("a first run seeds the directory",
                app.wait_http(f"http://127.0.0.1:{port}/canvas_page/file/index.html", 30), proc.log())
        t = team_mod.Team(ctx.binary, folder, env)
        records.create(t, {"id": "credentials", "parent_id": "user", "handler_module": "credential_store.tools",
                           "backend": "env",
                           "refs": {f"model/{ctx.model.host}": {"host": ctx.model.host,
                                                                 "env": {"api_key": "E2E_MODEL_KEY"}}}})
        proc.signal(signal.SIGINT)
        v.check("the seeding run stops", proc.wait(30) is not None, proc.log())
    finally:
        proc.reap()


def point_at_plug(ctx, team):
    """The only change to what the app made: the model answers from the plug."""
    agents = team.agents()
    records.update(team, "lead", model=cli.MODEL, endpoint=ctx.model.endpoint,
                   credentials={"store": "credentials", "ref": f"model/{ctx.model.host}"})
    tiers = {}
    for name, t in agents["roles"]["tiers"].items():
        t = dict(t, model=cli.MODEL, endpoint=ctx.model.endpoint)
        t.pop("credentials", None)  # a spawned agent is not on the credential store's callable_by
        tiers[name] = t
    records.update(team, "roles", tiers=tiers, limits={"total": 6})


def run(ctx):
    v = Verdict("roles/app_team_spawn_trees")
    app.app_binary(ctx)  # a Skip when the app is not built next to --bin
    keys = run_flow.fake_keys(ctx)
    env = {"E2E_MODEL_KEY": keys["model_key"], "FANTASTIC_AGENTIC_RESOURCES": "1"}
    folder = os.path.join(ctx.run_dir, "team")
    os.makedirs(folder)
    ctx.work = folder
    probe = team_mod.Team(ctx.binary, folder, env)
    seed(ctx, folder, probe.env, v)
    port = app.free_port()
    ctx.team = probe  # the app gets the probe's environment (`app.env_of`)
    proc = app.run_app(ctx, ["--binary", ctx.binary, "--port", port, "--dir", folder], name="app")
    try:
        v.check("the app starts and prints the web UI URL", bool(proc.wait_line("web UI at", 90)), proc.log())
        made = probe.wait_for(lambda: {"lead", "roles", "worker_exec"} <= set(probe.agents()), 30)
        v.check("the app made the team", bool(made), str(sorted(probe.agents())))
        for name in sorted(os.listdir(REPO_ROLES)):
            with open(os.path.join(REPO_ROLES, name)) as f, open(os.path.join(folder, "roles", name)) as g:
                v.check(f"roles/{name} on disk is the repo's card", f.read() == g.read())
        point_at_plug(ctx, probe)
        before = cli.tree(probe)

        r = probe.send("lead", {"type": "send", "text": TASK})
        v.check("the lead takes its task", r.get("status") == "completed", str(r))
        v.check("the scripted model played every turn, none past the end",
                cli.wait_served(ctx, 150) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        lanes = set(ctx.model.request_lanes)
        v.check("every lane asked the model: lead and project_lead each spawned, each spawned a worker",
                {"main", "s1", "p1", "w1", "w2"} <= lanes, str(sorted(lanes)))
        time.sleep(1.0)
        after = cli.tree(probe)
        left = [i for i, a in probe.agents().items() if a.get("spawned_by")]
        v.check("no spawned agent is left", not left, str(left))
        orphans = [i for i, (_, p) in after.items() if p and p not in after]
        v.check("no orphans: every agent's parent exists", not orphans, str(orphans))
        v.check("the records are the ones the app made", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        count = probe.send("roles", {"type": "get_count"})["result"]
        v.check("the registry counts nothing running", count["running"] == 0, str(count))
    finally:
        proc.signal(signal.SIGINT)
        proc.wait(30)
        proc.reap()
    return v.finish()
