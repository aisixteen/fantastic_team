"""headless_and_canvas: `--headless` opens no port and needs no terminal; `canvas --port N` serves;
`webui --port N` serves the team page and its socket; an agent's `create_agent` of a canvas works
with no `canvas` word."""

import os
import subprocess
import time

from helpers import cli, team as team_mod, web
from helpers.verdict import Skip, Verdict


def run(ctx):
    v = Verdict("cli/headless_and_canvas")
    if cli.listeners(os.getpid()) is None:
        raise Skip("neither lsof nor /proc/net/tcp is available to list listening sockets")

    # 1. --headless: no port, no screen; the lead makes a canvas with create_agent
    team = cli.start(ctx)  # stdin is /dev/null and there is no tty: the UI could not open
    cli.add_base(ctx)
    base = set(cli.ids(team))
    cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent"]})
    new = set(cli.ids(team)) - base
    v.check("the lead was made with create_agent", new == {"lead"}, str(new))
    v.check("the team runs headless", team.proc.poll() is None)
    agents = team.agents()
    v.check("the canvas is registered but not started: no web host, no canvas page",
            not {"web", "canvas_page", "canvas_ws"} & set(agents), str(sorted(agents)))
    team.send("lead", {"type": "send", "text": "Make a canvas for your notes."})
    v.check("the scripted model played every turn", cli.wait_served(ctx, 60) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    agents = team.agents()
    sk = agents.get("sketch", {})
    v.check("the lead's create_agent made a canvas with no `canvas` word",
            sk.get("handler_module") == "canvas_page.tools" and sk.get("parent_id") == "lead", str(sk)[:300])
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
    v.check("the lead's canvas and the lead are still there", {"lead", "sketch"} <= set(team.agents()))
    team.stop()

    # 3. webui --port N, in the same folder: the team page and its socket, from the binary
    wport = team_mod.free_port()
    if os.path.exists(sock):
        os.remove(sock)
    team.start(extra_args=["webui", "--port", wport])
    page = "/team_page/file/index.html"
    end = time.time() + 30
    status = 0
    while time.time() < end and status != 200:
        status, _ = team_mod.http_get(wport, page)
        time.sleep(0.3)
    v.check("webui --port N serves the team page", status == 200, str(status))
    ports = cli.listeners(team.proc.pid)
    v.check("the listener check sees the webui port", str(wport) in ports, f"{ports}, want {wport}")
    v.check("the page agent and its socket leg are there",
            {"team_page", "team_ws"} <= set(team.agents()), str(sorted(team.agents())))
    try:
        web.WsClient(int(wport)).sock.close()
        socket_ok = True
    except OSError as e:
        socket_ok = False
        print(f"socket: {e}")
    v.check("the team page's socket (/team_ws/ws) accepts a websocket", socket_ok)
    v.check("it announces the page", f"the team page: http://127.0.0.1:{wport}{page}" in team.log(),
            team.log()[-300:])
    v.check("the lead's canvas and the lead are still there", {"lead", "sketch"} <= set(team.agents()))
    team.stop()

    # 4. webui and canvas together: refused with a message, no process left, no port
    if os.path.exists(sock):
        os.remove(sock)
    both = subprocess.run(team.argv(["webui", "canvas", "--port", team_mod.free_port()]), env=team.env,
                          cwd=team.dir, capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
    v.check("webui with canvas is refused (exit 2) and says why",
            both.returncode == 2 and "`webui` and `canvas` cannot run together" in both.stderr,
            f"{both.returncode} {both.stderr[-300:]}")

    # 5. --headless over the folder webui served: still no port
    team.start()
    time.sleep(2)
    ports = cli.listeners(team.proc.pid)
    v.check("--headless opens no port in a folder webui has run in", ports == set(), str(ports))
    v.check("the old webui port refuses connections", web.refuses(int(wport)))
    team.stop()
    return v.finish()
