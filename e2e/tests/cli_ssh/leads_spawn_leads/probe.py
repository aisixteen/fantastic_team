"""cli_ssh/leads_spawn_leads: the lead spawns s1 and s2 on the local roles registry (the master; the box
holds tools only). s1 spawns its worker, which fixes adder.py and pushes. The remote agent (the work program
in the box) sends s2 its work and watches it, over the bridge (s2 is minted by the spawn: the operator opens
it to the leg when the script reaches that turn): s2's turn runs on the master, its stream comes back to the
box, and s2 spawns its worker (on the master, its tools the box's exec agent), which pulls, fixes muler.py
and pushes. Each sub-lead deletes its worker, the lead deletes both sub-leads. The wiring and the shared checks
are `helpers/ssh_twins.py`'s; the local checks are `cli/leads_spawn_leads`'s."""

from helpers import cli, podman_remote, ssh_twins as T, team as team_mod
from helpers.verdict import Verdict

TASK = "Fix the failing test with two sub-leads here, one for the box's workspace (the remote asks it); delete what you spawned."


def run(ctx):
    v = Verdict("cli_ssh/leads_spawn_leads")
    podman_remote.require()
    with T.Box(ctx) as box:
        cli.start(ctx, roles=True)
        cli.add_base(ctx)
        base = set(cli.ids(ctx.team))
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]}, subtree=True)
        cli.make_registry(ctx, total=4, roles=cli.spawn_roles("leads_spawn_leads"))
        new = set(cli.ids(ctx.team)) - base
        v.check("create_agent made the lead, its provider (and store) and its roles registry, under fixed ids",
                new == {"lead", "roles"} | {cli.PROVIDER, cli.PROVIDER_STORE}, str(new))
        cli.add_cadencer(ctx)
        before = cli.tree(ctx.team)
        box.publish()
        box.bind()
        v.check("the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, TASK) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        team = ctx.team
        ok, why = cli.solved(ctx.repo)
        v.check("solved in the local clone (the test passes on HEAD, the fix is committed)", ok, why)
        spawns = [c for c in team_mod.log_calls(team.log()) if c == ("roles", "spawn")]
        v.check("4 spawns, all here: 2 sub-leads by the lead, 1 worker by each", len(spawns) == 4, str(spawns))
        after = cli.tree(team)
        left = [i for i, a in team.agents().items() if a.get("spawned_by")]
        v.check("no spawned agent is left", not left, str(left))
        orphans = [i for i, (_, p) in after.items() if p and p not in after]
        v.check("no orphans: every agent's parent exists", not orphans, str(orphans))
        after = cli.without_new_schedulers(ctx.team, after, before)
        v.check("the records are the ones it started with (a lead's scheduler may stay) (the bridge deleted)", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        count = team.send("roles", {"type": "get_count"})["result"]
        v.check("the registry counts nothing running", count["running"] == 0, str(count))
        v.check("every lane asked the model, all on the master (s1, w1; s2 for the remote, w2)",
                {"s1", "s2", "w1", "w2"} <= set(ctx.model.request_lanes), str(sorted(set(ctx.model.request_lanes))))
        v.check("the remote work ran through the bridge: the work program exited 0",
                T.came_back(ctx, None, "<exit_code>0</exit_code>"), str(T.forward_replies(ctx)[-2:])[:300])
        v.check("the box clone's last commit is w2's fix (read through the bridge)",
                T.came_back(ctx, None, "<line>Fix mul</line>"))
        cli.check_ins_played(ctx, v)
        box.end(v, streams=[("s2", "s2")])
    box.removed(v)
    return v.finish()
