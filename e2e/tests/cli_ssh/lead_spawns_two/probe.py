"""cli_ssh/lead_spawns_two: the lead spawns w1 on the local roles registry and w2 on a remote one (a
kernel in a Podman box, the registry made there by the lead through the bridge). w1 fixes adder.py
and pushes, then w2 pulls, fixes muler.py and pushes; the lead reads w2's conversation back through
the bridge and deletes each worker where it lives. The wiring and the shared checks are
`helpers/ssh_twins.py`'s; the local checks are `cli/lead_spawns_two`'s, split across the kernels."""

import re

from helpers import cli, podman_remote, ssh_twins as T, team as team_mod
from helpers.verdict import Verdict

TASK = "Fix the failing test: spawn one worker here and one on the remote kernel, then delete them."


def run(ctx):
    v = Verdict("cli_ssh/lead_spawns_two")
    podman_remote.require()
    with T.Box(ctx, roles=True) as box:
        cli.start(ctx, roles=True)
        cli.add_base(ctx)
        base = set(cli.ids(ctx.team))
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]}, subtree=True)
        cli.make_registry(ctx, total=1, roles=cli.roles_dir("lead_spawns_two"))
        new = set(cli.ids(ctx.team)) - base
        v.check("create_agent made the lead and its roles registry, under fixed ids", new == {"lead", "roles"}, str(new))
        count = ctx.team.send("roles", {"type": "get_count"})["result"]
        v.check("the local registry is on and counts nothing running", count == {"running": 0, "max": 1}, str(count))
        before = cli.tree(ctx.team)
        box.publish()
        box.bind()
        v.check("the scripted model played every turn, none past the end",
                T.hand(ctx, cli.LEAD, TASK) and ctx.model.consumed,
                f"served {ctx.model.served} of {len(ctx.model.turns)}")
        ok, why = cli.solved(ctx.repo)
        v.check("solved in the local clone (the test passes on HEAD, the fix is committed)", ok, why)
        spawns = [c for c in team_mod.log_calls(ctx.team.log()) if c == ("roles", "spawn")]
        v.check("the lead spawned 1 worker here", len(spawns) == 1, str(spawns))
        after = cli.tree(ctx.team)
        v.check("the local records are the ones it started with (the worker and the bridge deleted)", after == before,
                f"before {sorted(before)} after {sorted(after)}")
        count = ctx.team.send("roles", {"type": "get_count"})["result"]
        v.check("the local registry counts nothing running", count["running"] == 0, str(count))
        v.check("each worker asked the model (w1 here, w2 on the remote kernel)",
                {"w1", "w2"} <= set(ctx.model.request_lanes), str(set(ctx.model.request_lanes)))
        minted = list(dict.fromkeys(re.findall(r"<id>(anthropic_backend_\w+)</id>", ctx.model.raw_bodies[-1])))
        v.check("the lead saw two minted worker ids, one per kernel", len(minted) >= 2, str(minted))
        v.check("w2's results came back through the bridge (the box clone's last commit is its fix)",
                T.came_back(ctx, None, "<line>Fix mul</line>"))
        box.end(v)
        remote = T.remote_spawns(box.remote_log_text)
        v.check("the remote registry spawned 1 worker there", len(remote) == 1, str(remote))
    box.removed(v)
    return v.finish()
