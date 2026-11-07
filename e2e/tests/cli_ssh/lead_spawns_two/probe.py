"""cli_ssh/lead_spawns_two: the lead spawns w1 and w2 on the local roles registry (the master; the box
holds tools only). w1 fixes adder.py and pushes. The remote agent (the work program in the box) sends w2
its work and watches it, over the bridge (w2 is minted by the spawn: the operator opens it to the leg when
the script reaches that turn): w2's turn runs on the master, its stream comes back to the box, its tools are
the box's exec agent (pull, fix muler.py, push). The lead deletes both workers. The wiring and the shared
checks are `helpers/ssh_twins.py`'s; the local checks are `cli/lead_spawns_two`'s."""

import re

from helpers import cli, podman_remote, ssh_twins as T, team as team_mod
from helpers.verdict import Verdict

TASK = "Fix the failing test: spawn two workers here, one for the box's workspace (the remote asks it), then delete them."


def run(ctx):
    v = Verdict("cli_ssh/lead_spawns_two")
    podman_remote.require()
    with T.Box(ctx) as box:
        cli.start(ctx, roles=True)
        cli.add_base(ctx)
        base = set(cli.ids(ctx.team))
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]}, subtree=True)
        cli.make_registry(ctx, total=2, roles=cli.roles_dir("lead_spawns_two"))
        new = set(cli.ids(ctx.team)) - base
        v.check("create_agent made the lead and its roles registry, under fixed ids", new == {"lead", "roles"}, str(new))
        count = ctx.team.send("roles", {"type": "get_count"})["result"]
        v.check("the registry is on and counts nothing running", count == {"running": 0, "max": 2}, str(count))
        before = cli.tree(ctx.team)
        box.publish()
        box.bind()
        v.check("the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, TASK) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        ok, why = cli.solved(ctx.repo)
        v.check("solved in the local clone (the test passes on HEAD, the fix is committed)", ok, why)
        spawns = [c for c in team_mod.log_calls(ctx.team.log()) if c == ("roles", "spawn")]
        v.check("the lead spawned 2 workers, both here", len(spawns) == 2, str(spawns))
        after = cli.tree(ctx.team)
        v.check("the records are the ones it started with (both workers and the bridge deleted)", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        count = ctx.team.send("roles", {"type": "get_count"})["result"]
        v.check("the registry counts nothing running", count["running"] == 0, str(count))
        v.check("each worker asked the model, both on the master (w1, and w2 for the remote)",
                {"w1", "w2"} <= set(ctx.model.request_lanes), str(set(ctx.model.request_lanes)))
        minted = list(dict.fromkeys(re.findall(r"<id>(anthropic_backend_\w+)</id>", ctx.model.raw_bodies[-1])))
        v.check("the lead saw two minted worker ids", len(minted) >= 2, str(minted))
        v.check("the remote work ran through the bridge: the work program exited 0",
                T.came_back(ctx, None, "<exit_code>0</exit_code>"), str(T.forward_replies(ctx)[-2:])[:300])
        v.check("the box clone's last commit is w2's fix (read through the bridge)",
                T.came_back(ctx, None, "<line>Fix mul</line>"))
        box.end(v, streams=[("w2", "w2")])
    box.removed(v)
    return v.finish()
