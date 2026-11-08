"""gpu_cli_ssh/headless_and_canvas: `cli_ssh/headless_and_canvas` on a real model. The local kernel
serves `canvas --port P`, the remote runs `--headless`; told the box and the wiring in plain words, the
lead has the remote ask `rlead` (a backend on the master) to make a canvas on the remote; both add a note. The local canvas's socket
lists the remote agents through the bridge; the box listens on sshd and the tunnel's port only. Then
the lead deletes the bridge and stops the remote; the local kernel restarts as `webui --port P2`; the
lead starts the remote and the bridge again, and the team page's socket lists them too. Then the lead
cleans up, and the local test's last checks: `webui` with `canvas` is refused, `--headless` opens no
port. The box and the checks are `helpers/ssh_twins.py`'s; the task is `helpers/gpu_ssh.py`'s; no script."""

import os
import subprocess
import time

from helpers import cli, cli_scenarios as S, gpu_cli as gpu, gpu_ssh as G, ssh_twins as T, team as team_mod
from helpers.verdict import Skip, Verdict


def fresh(ctx) -> None:
    sock = os.path.join(ctx.work, team_mod.SOCK)
    if os.path.exists(sock):  # the stopped team's socket file must not pass for the next start
        os.remove(sock)


def run(ctx):
    v = Verdict("gpu_cli_ssh/headless_and_canvas")
    G.require(ctx)
    if cli.listeners(os.getpid()) is None:
        raise Skip("neither lsof nor /proc/net/tcp is available to list listening sockets")
    with T.Box(ctx) as box:
        port = team_mod.free_port()
        team = cli.start(ctx, extra_args=["canvas", "--port", port])
        cli.add_base(ctx)
        cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent", "delete_agent"]})
        cli.make_agent(ctx, "rlead", cli.LEAD, {cli.EXEC: "*"}, key=ctx.model.keyed)
        box.open_backends(["rlead"])
        cli.add_cadencer(ctx)
        before = cli.tree(team)
        box.publish()
        v.check("canvas --port P serves the canvas page", S.wait_page(port, "/canvas_page/file/index.html") == 200)
        watch = G.play(ctx, v, "lead", G.wiring(ctx, box, ["rlead"]) + G.TASKS["headless_and_canvas"], label="1: ")
        gpu.lead_on_default(ctx, v, label="1: ")
        v.check("the box listens on sshd and the tunnel's port only (the remote is --headless)",
                box.listening() <= {22, box.tunnel_port}, str(box.listening()))
        T.shows_canvas(v, "the local canvas's socket (canvas_ws -> user.forward -> the bridge)",
                       T.ask_through(port, "/canvas_ws/ws"))
        G.play(ctx, v, "lead", G.PAUSE, label="1b: ")
        gpu.report(ctx, watch)  # before the restart: the plug reads the lead's usage from this team
        team.stop()

        wport = team_mod.free_port()
        fresh(ctx)
        # a folder `canvas` seeded has no team page records: `webui` does not make them (#261)
        team.start()
        S.stock.make_web(team)
        team.stop()
        fresh(ctx)
        team.start(extra_args=["webui", "--port", wport])
        v.check("webui --port P2 serves the team page", S.wait_page(wport, "/team_page/file/index.html") == 200)
        G.play(ctx, v, "lead", G.RESTART, label="2: ")
        T.shows_canvas(v, "the team page's socket (/team_ws/ws -> user.forward -> the bridge)", T.ask_through(wport))
        watch = G.play(ctx, v, "lead", G.CLEANUP, label="3: ")
        after = cli.tree(team)
        extra, missing = set(after) - set(before), set(before) - set(after)
        v.check("the local records are the ones it started with, plus the team page's (the bridge deleted)",
                extra <= {"team_page", "team_ws"} and not missing, f"extra {sorted(extra)} missing {sorted(missing)}")
        box.end(v, solved=False)
        notes = box.server_files.get("NOTES.md", "")
        v.check("both notes landed on the git server: the lead's and rlead's",
                "notes of the lead" in notes and "notes of rlead" in notes, notes)
        gpu.report(ctx, watch)
        team.stop()

        fresh(ctx)
        both = subprocess.run(team.argv(["webui", "canvas", "--port", team_mod.free_port()]), env=team.env,
                              cwd=team.dir, capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
        v.check("webui with canvas is refused (exit 2) and says why",
                both.returncode == 2 and "`webui` and `canvas` cannot run together" in both.stderr,
                f"{both.returncode} {both.stderr[-300:]}")
        team.start()
        time.sleep(2)
        ports = cli.listeners(team.proc.pid)
        v.check("--headless opens no port here either", ports == set(), str(ports))
        team.stop()
    box.removed(v)
    return v.finish()
