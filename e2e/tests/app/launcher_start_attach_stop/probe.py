"""The app launcher: start a headless child, attach to a server that is up, stop."""

import json
import os
import signal
import stat

from helpers import app, run_flow, web
from helpers.verdict import Verdict

# A stand-in for the headless binary, for the START leg only. The real binary's
# `--port` serves the canvas page (the web host answers 404 at `/`), while the
# launcher waits for a 200 at `/`; so the real binary can never be "ready" for it
# (see README). The stand-in answers 200 at `/` and records its argv.
STANDIN = r'''
import http.server, json, os, sys
args = sys.argv[1:]
port = int(args[args.index("--port") + 1])
where = args[args.index("--dir") + 1]
with open(os.path.join(where, "standin_argv.json"), "w") as f:
    json.dump(args, f)
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def do_GET(self):
        self.send_response(200); self.send_header("Content-Length", "2"); self.end_headers(); self.wfile.write(b"ok")
http.server.ThreadingHTTPServer(("127.0.0.1", port), H).serve_forever()
'''


def run(ctx):
    v = Verdict("app/launcher_start_attach_stop")
    app.app_binary(ctx)  # a Skip when the app is not built next to --bin
    procs = []
    try:
        keys = run_flow.fake_keys(ctx)
        ctx.secrets = keys
        q = app.free_port()
        ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"]}, port=q, wire=["credentials"])
        canvas = f"http://127.0.0.1:{q}/canvas_page/file/index.html"
        v.check("the headless binary's web UI answers on its port", app.wait_http(canvas, 30), canvas)

        # --- attach to the server that is up: starts nothing, stops nothing
        a = app.run_app(ctx, ["--attach", canvas], name="attach")
        procs.append(a)
        line = a.wait_line("web UI at", 40)
        v.check("attach prints the web UI URL", bool(line) and line.strip().endswith(canvas), a.log())
        v.check("attach starts no child process", app.child_pids(a.pid) == [], str(app.child_pids(a.pid)))
        a.signal(signal.SIGINT)
        v.check("attach exits 0 on SIGINT", a.wait(20) == 0, a.log())
        v.check("the server it attached to is still up", web.up(q, "/canvas_page/file/index.html"))

        # --- attach to nothing: fails, starts nothing
        dead = app.free_port()
        b = app.run_app(ctx, ["--attach", f"http://127.0.0.1:{dead}/"], name="attach_dead")
        procs.append(b)
        v.check("attach to an unused port exits non-zero, with no child process",
                b.wait(90) not in (None, 0) and app.child_pids(b.pid) == [] and "no 200" in b.log(), b.log())

        # --- start: one child, the web UI answers, stop leaves no child
        wrapper = os.path.join(ctx.run_dir, "headless")
        standin = os.path.join(ctx.run_dir, "standin.py")
        with open(standin, "w") as f:
            f.write(STANDIN)
        with open(wrapper, "w") as f:
            f.write(f'#!/bin/sh\nexec python3 -u "{standin}" "$@"\n')
        os.chmod(wrapper, os.stat(wrapper).st_mode | stat.S_IXUSR)
        p = app.free_port()
        s = app.run_app(ctx, ["--binary", wrapper, "--port", p, "--dir", ctx.run_dir], name="start")
        procs.append(s)
        want = f"http://127.0.0.1:{p}/"
        line = s.wait_line("web UI at", 40)
        v.check("start prints the web UI URL", bool(line) and line.strip().endswith(want), s.log())
        kids = app.child_pids(s.pid)
        v.check("start has exactly one child process", len(kids) == 1, str(kids))
        v.check("the child answers 200 on the port", app.wait_http(want, 10))
        try:
            argv = json.load(open(os.path.join(ctx.run_dir, "standin_argv.json")))
        except (OSError, ValueError):
            argv = None
        v.check("the child got the port and the dir and nothing else",
                argv == ["--port", str(p), "--dir", ctx.run_dir], str(argv))
        s.signal(signal.SIGINT)
        v.check("start exits 0 on SIGINT", s.wait(20) == 0, s.log())
        v.check("stop leaves no child process", bool(kids) and app.wait_gone(kids[0], 10), str(kids))
        v.check("the port refuses connections after stop", app.wait_refused(p, 10))
        v.check("stop did not touch the attached server", web.up(q, "/canvas_page/file/index.html"))
    finally:
        for pr in procs:
            pr.reap()
    return v.finish()
