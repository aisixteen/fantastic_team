"""bundles/app_team_spawn_trees: the team the app makes runs lead -> sub-lead -> worker and
lead -> project_lead -> worker, and each parent deletes what it spawned (#227)."""

import os
import signal
import time

from helpers import app, cli, records, run_flow, stock as S, team as team_mod
from helpers.verdict import Verdict

TASK = "Run a sub-lead and a project lead, each with a worker; delete what you spawned."


def seed(ctx, folder, env, v, team):
    """Run the binary's `webui` once in `folder`, so the team's first run seeds the team's agents
    from the blueprint (#261). The key mode is the person's choice (the team and the app choose
    none): the probe makes it as the set-up page does, `set_key_mode` plain on fantastic's
    `setup.tools` agent, so fantastic makes its credential store; then stop the run."""
    port = app.free_port()
    proc = app.Proc([ctx.binary, "webui", "--port", str(port)], env, folder,
                    os.path.join(ctx.run_dir, "seed.log"))
    try:
        v.check("a first run seeds the directory",
                app.wait_http(f"http://127.0.0.1:{port}/team_page/file/index.html", 30), proc.log())
        records.create(team, {"id": "person_setup", "parent_id": "user", "handler_module": "setup.tools"})
        r = team.send("person_setup", {"type": "set_key_mode", "mode": "plain"})
        v.check("the person chose the plain key mode", (r.get("result") or {}).get("mode") == "plain", str(r))
        team.send("kernel", {"type": "delete_agent", "id": "person_setup"})
        proc.signal(signal.SIGINT)
        v.check("the seeding run stops", proc.wait(30) is not None, proc.log())
    finally:
        proc.reap()


def store_model_key(ctx, team, key):
    """Keep the model's key the way the set-up page does: declare the ref on the store fantastic
    made for the person's choice (its backend is the `plain` file), then `set` the value."""
    ref = f"model/{ctx.model.host}"
    refs = dict(team.agents()["credentials"].get("refs") or {})
    refs[ref] = {"host": ctx.model.host, "fields": ["api_key"]}
    records.update(team, "credentials", refs=refs)
    r = team.send("credentials", {"type": "set", "ref": ref, "fields": {"api_key": key}})
    if r.get("status") != "completed" or (r.get("result") or {}).get("error"):
        raise RuntimeError(f"set {ref}: {r}")


def point_at_plug(ctx, team):
    """The only change to what the app made: the person adds a provider on the plug (under the lead,
    its key the one in the credential store) and the lead names it, as the team's yaml says the user
    does; a spawned chat takes the lead's provider."""
    records.update(team, "lead", provider_id=cli.PROVIDER, model=cli.MODEL)
    cli.add_provider(ctx, "lead")
    # a spawned agent runs on its spawner's provider: pointing the lead at the plug points its tree there


def ids_once_up(team):
    """The agent ids, or none while the app's kernel does not answer yet (it is still starting:
    its socket is not there, or `list_agents` fails); the wait goes on until its deadline."""
    r = team.send("kernel", {"type": "list_agents"})
    if r.get("status") != "completed":
        return set()
    return {a["id"] for a in r["result"]["agents"]}


def run(ctx):
    v = Verdict("bundles/app_team_spawn_trees")
    app.app_binary(ctx)  # a Skip when the app is not built next to --bin
    keys = run_flow.fake_keys(ctx)
    env = {"E2E_MODEL_KEY": keys["model_key"], "FANTASTIC_AGENTIC_RESOURCES": "1"}
    folder = os.path.join(ctx.run_dir, "team")
    os.makedirs(folder)
    ctx.work = folder
    probe = team_mod.Team(ctx.binary, folder, env)
    seed(ctx, folder, probe.env, v, probe)
    port = app.free_port()
    ctx.team = probe  # the app gets the probe's environment (`app.env_of`)
    proc = app.run_app(ctx, ["--binary", ctx.binary, "--port", port, "--dir", folder], name="app")
    try:
        v.check("the app starts and prints the web UI URL", bool(proc.wait_line("web UI at", 90)), proc.log())
        made = probe.wait_for(lambda: {"lead", "bundles", "worker_exec"} <= ids_once_up(probe), 30)
        v.check("the app made the team", bool(made), str(sorted(ids_once_up(probe))))
        if not made:
            return v.finish()  # no team to drive: the checks below would only read a missing kernel
        v.check("the bundles registry serves the repo's bundles (compiled in; no card is written to the folder)",
                S.bundles_served(probe))
        store_model_key(ctx, probe, keys["model_key"])
        point_at_plug(ctx, probe)
        before = cli.tree(probe)
        # the app's own agents on spawnable bundles (the lead is one): what runs before the task
        base = probe.send("bundles", {"type": "get_count"})["result"]

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
        left = [i for i in probe.agents() if i not in before]
        v.check("no spawned agent is left", not left, str(left))
        orphans = [i for i, (_, p) in after.items() if p and p not in after]
        v.check("no orphans: every agent's parent exists", not orphans, str(orphans))
        v.check("the records are the ones the app made", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        count = probe.send("bundles", {"type": "get_count"})["result"]
        v.check("the registry counts only the app's own agents, as before the task", count == base,
                f"before {base} after {count}")
    finally:
        proc.signal(signal.SIGINT)
        proc.wait(30)
        proc.reap()
    return v.finish()
