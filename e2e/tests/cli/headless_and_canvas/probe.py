"""headless_and_canvas: `--headless` opens no port and needs no terminal; `canvas --port N` serves;
an agent's `create_agent` of a canvas works with no `canvas` word."""

import os
import time

from helpers import cli, team as team_mod
from helpers.verdict import Skip, Verdict


def run(ctx):
    v = Verdict("cli/headless_and_canvas")
    if cli.listeners(os.getpid()) is None:
        raise Skip("neither lsof nor /proc/net/tcp is available to list listening sockets")

    # 1. --headless: no port, no screen; the lead makes a canvas with create_agent
    team = cli.start(ctx)  # stdin is /dev/null and there is no tty: the UI could not open
    cli.add_base(ctx)
    new = cli.import_blueprint(ctx)
    v.check("the lead was imported", new == ["lead_1"], str(new))
    v.check("the team runs headless", team.proc.poll() is None)
    agents = team.agents()
    v.check("the canvas is registered but not started: no web host, no canvas page",
            not {"web", "canvas_page", "canvas_ws"} & set(agents), str(sorted(agents)))
    team.send("lead_1", {"type": "send", "text": "Make a canvas for your notes."})
    v.check("the scripted model played every turn", cli.wait_served(ctx, 60) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    agents = team.agents()
    sk = agents.get("sketch", {})
    v.check("the lead's create_agent made a canvas with no `canvas` word",
            sk.get("handler_module") == "canvas_page.tools" and sk.get("parent_id") == "lead_1", str(sk)[:300])
    v.check("still no web host", "web" not in agents, str(sorted(agents)))
    ports = cli.listeners(team.proc.pid)
    v.check("--headless binds no TCP port", ports == set(), str(ports))
    log = team.log()
    v.check("it draws no terminal UI", not any(s in log for s in cli.ALT_SCREEN))
    v.check("it serves no canvas page", "the canvas: http" not in log)
    code = team.stop()
    v.check("it stops cleanly on SIGTERM", code in (0, -15), str(code))

    # 2. canvas --port N, in the same folder: the same records, and now a web host
    port = team_mod.free_port()
    sock = os.path.join(ctx.work, team_mod.SOCK)
    if os.path.exists(sock):  # the stopped team's socket file must not pass for the next start
        os.remove(sock)
    team.start(extra_args=["canvas", "--port", port])
    end = time.time() + 30
    status = 0
    while time.time() < end and status != 200:
        status, _ = team_mod.http_get(port, "/canvas_page/file/index.html")
        time.sleep(0.3)
    v.check("canvas --port N serves the canvas page", status == 200, str(status))
    ports = cli.listeners(team.proc.pid)
    v.check("the listener check sees port N", str(port) in ports, f"{ports}, want {port}")
    v.check("the web host and the canvas page are there now",
            {"web", "canvas_page"} <= set(team.agents()), str(sorted(team.agents())))
    v.check("the lead's canvas and the lead are still there", {"lead_1", "sketch"} <= set(team.agents()))
    team.stop()
    return v.finish()
