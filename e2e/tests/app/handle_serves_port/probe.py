"""TeamHandle serves the web UI on a port inside one process: no child process."""

import os
import signal
import subprocess

from helpers import app, run_flow, team as team_mod, web
from helpers.verdict import Verdict


def others(run_dir, me):
    """Pids (not `me`) whose command line holds `--dir RUN`: a second team process."""
    r = subprocess.run(["pgrep", "-f", "--", f"--dir {run_dir}"], capture_output=True, text=True)
    return [int(x) for x in r.stdout.split() if int(x) not in (me, os.getpid())]


def run(ctx):
    v = Verdict("app/handle_serves_port")
    example = app.handle_example(ctx)  # a Skip when the example is not built
    h = None
    try:
        keys = run_flow.fake_keys(ctx)
        ctx.secrets = keys
        ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"]}, wire=["credentials"])
        ctx.team.stop()  # the tree is on disk, the lock is free
        port = app.free_port()
        h = app.Proc([example, "--dir", ctx.run_dir, "--port", str(port)], app.env_of(ctx.team),
                     ctx.run_dir, os.path.join(ctx.run_dir, "handle.log"), stdin=subprocess.PIPE)
        line = h.wait_line("url:", 60)
        page = f"http://127.0.0.1:{port}/team_page/file/index.html"
        v.check("the handle reports the page URL", line is not None and line.split("url:")[1].strip() == page,
                f"{line!r}\n{h.log()[-600:]}")
        ok = app.wait_http(page, 20)
        v.check("the page answers 200", ok)
        status, body = web.http_get(page) if ok else (0, b"")
        v.check("it is the dispatcher page with the socket shim",
                b"<title>Dispatcher</title>" in body and b"window.fantastic = {" in body, str(status))
        v.check("another file of the page is a 404",
                web.http_get(f"http://127.0.0.1:{port}/team_page/file/other.txt")[0] == 404)

        ws = web.WsClient(port, "/team_ws/ws")
        side = ws.call("dispatcher", {"type": "sidebar"})
        v.check("the page's socket reaches the dispatcher (sidebar answers the groups)",
                isinstance(side.get("groups"), list) and not side.get("error"), str(side)[:300])
        out = ws.call("kernel_state", {"type": "list_agents"})
        v.check("the socket reaches nothing outside its grant",
                out.get("reason") == "unauthorized" or out.get("refused"), str(out)[:200])
        ws.close()

        v.check("no child process", app.child_pids(h.pid) == [], str(app.child_pids(h.pid)))
        v.check("no second team process on this directory", others(ctx.run_dir, h.pid) == [],
                str(others(ctx.run_dir, h.pid)))
        second = team_mod.Team(ctx.binary, ctx.run_dir, ctx.team.env)
        try:
            second.start(wait_s=20)
            held = False
        except RuntimeError:
            held = True
        finally:
            second.stop()
        v.check("the directory lock is held while it serves (a second team cannot start)", held)

        h.p.stdin.close()  # end of stdin: stop
        v.check("on end of stdin the process stops and exits 0", h.wait(30) == 0, h.log()[-400:])
        v.check("the port refuses connections after stop", app.wait_refused(port, 10))
        try:
            ctx.team.start(wait_s=60)
            r = ctx.team.send("kernel", {"type": "list_agents"})
            freed = r.get("status") == "completed"
        except RuntimeError as e:
            freed, r = False, str(e)[:300]
        v.check("the lock is released: a normal team starts on the directory", freed, str(r)[:300])
    finally:
        if h:
            h.reap()
    return v.finish()
