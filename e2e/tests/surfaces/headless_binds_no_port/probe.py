"""Headless (`--headless`): the team binds no TCP port, draws no terminal UI, and a verb still works
over the socket. `canvas --port N` is the positive control: the same listener check then sees port N."""

import os
import time

from helpers import cli, stock, team as team_mod
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("surfaces/headless_binds_no_port")
    if cli.listeners(os.getpid()) is None:
        print("SKIP: neither lsof nor /proc/net/tcp is available to list listening sockets")
        return 0
    other = None
    try:
        stock.boot(ctx, forge=False)
        team = ctx.team
        v.check("the headless team is running", team.proc.poll() is None)
        v.check("it serves the operator socket", os.path.exists(os.path.join(team.dir, team_mod.SOCK)))
        v.check("the blueprint's lead is in it", ctx.lead in team.agents(), ctx.lead)
        ports = cli.listeners(team.proc.pid)
        v.check("it binds no TCP port", ports == set(), str(ports))
        log = team.log()
        v.check("it draws no terminal UI", not any(s in log for s in cli.ALT_SCREEN))
        v.check("it serves no canvas page", "the canvas: http" not in log)
        r = team.send("kernel", {"type": "list_agents"})
        v.check("a verb works over the socket with no surface on", r.get("status") == "completed", str(r)[:300])
        code = team.stop()
        v.check("it stops cleanly on SIGTERM", code in (0, -15), str(code))

        # control: with `canvas --port N` the same check must see the listener
        d = os.path.join(ctx.run_dir, "ported")
        os.makedirs(d)
        port = team_mod.free_port()
        other = team_mod.Team(ctx.binary, d, team.env)
        other.start(extra_args=["canvas", "--port", port])
        end = time.time() + 60
        while time.time() < end and "the canvas: http://" not in other.log():
            time.sleep(0.3)
        got = cli.listeners(other.proc.pid)
        v.check("control: with `canvas --port N` the listener check sees port N", str(port) in got, f"{got}, want {port}")
    finally:
        if ctx.team:
            ctx.team.stop()
        if other:
            other.stop()
    return v.finish()
