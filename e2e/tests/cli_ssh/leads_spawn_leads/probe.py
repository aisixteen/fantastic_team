"""cli_ssh/leads_spawn_leads: the lead spawns s1 on the local roles registry and s2 on a remote one
(a kernel in a Podman box, the registry made there by the lead through the bridge). Each sub-lead
spawns its worker on its own kernel's registry: s1's worker fixes adder.py and pushes, then s2's
pulls, fixes muler.py and pushes; each sub-lead deletes its worker, the lead deletes s1 here and s2
through the bridge. The wiring and the shared checks are `helpers/ssh_twins.py`'s; the local checks
are `cli/leads_spawn_leads`'s, split across the kernels."""

from helpers import cli, podman_remote, ssh_twins as T, team as team_mod
from helpers.verdict import Verdict

TASK = "Fix the failing test with sub-leads, one here and one on the remote kernel; delete what you spawned."


def run(ctx):
    v = Verdict("cli_ssh/leads_spawn_leads")
    podman_remote.require()
    with T.Box(ctx, roles=True) as box:
        cli.start(ctx, roles=True)
        cli.add_base(ctx)
        base = set(cli.ids(ctx.team))
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]}, subtree=True)
        cli.make_registry(ctx, total=2, roles=cli.roles_dir("leads_spawn_leads"))
        new = set(cli.ids(ctx.team)) - base
        v.check("create_agent made the lead and its roles registry, under fixed ids", new == {"lead", "roles"}, str(new))
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
        v.check("2 spawns here: s1 by the lead, its worker by s1", len(spawns) == 2, str(spawns))
        after = cli.tree(team)
        left = [i for i, a in team.agents().items() if a.get("spawned_by")]
        v.check("no spawned agent is left here", not left, str(left))
        orphans = [i for i, (_, p) in after.items() if p and p not in after]
        v.check("no orphans here: every agent's parent exists", not orphans, str(orphans))
        v.check("the local records are the ones it started with (the bridge deleted)", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        count = team.send("roles", {"type": "get_count"})["result"]
        v.check("the local registry counts nothing running", count["running"] == 0, str(count))
        v.check("every lane asked the model (s1, w1 here; s2, w2 remote)",
                {"s1", "s2", "w1", "w2"} <= set(ctx.model.request_lanes), str(sorted(set(ctx.model.request_lanes))))
        v.check("the remote side's results came back through the bridge (the box clone's last commit is its fix)",
                T.came_back(ctx, None, "<line>Fix mul</line>"))
        box.end(v)
        remote = T.remote_spawns(box.remote_log_text)
        v.check("2 spawns on the remote kernel: s2 (asked through the bridge), its worker by s2",
                len(remote) == 2, str(remote))
    box.removed(v)
    return v.finish()
