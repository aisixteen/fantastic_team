"""Attach to a shift on another box through the user's own `ssh -L`, with a fake ssh."""

import os
import re
import signal
import stat
import subprocess
import sys
import time

from helpers import app, run_flow, web
from helpers.verdict import Verdict

CANVAS = "/canvas_page/file/index.html"

# The user's ssh, replaced: it records its argv and forwards the loopback port it is
# asked for (`-L 127.0.0.1:LOCAL:127.0.0.1:REMOTE`) to the remote port, like ssh -N -L.
FAKE_SSH = r'''#!/usr/bin/env python3
import os, socket, sys, threading
with open("ssh_argv.txt", "w") as f:
    f.write("\n".join(sys.argv[1:]) + "\n")
with open("ssh_pid.txt", "w") as f:
    f.write(str(os.getpid()))
spec = sys.argv[sys.argv.index("-L") + 1].split(":")
bind, local, rport = spec[0], int(spec[1]), int(spec[3])
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind((bind, local))
srv.listen(16)
def pipe(a, b):
    try:
        while True:
            d = a.recv(65536)
            if not d:
                break
            b.sendall(d)
    except OSError:
        pass
    for s in (a, b):
        try:
            s.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
while True:
    c, _ = srv.accept()
    r = socket.create_connection(("127.0.0.1", rport))
    threading.Thread(target=pipe, args=(c, r), daemon=True).start()
    threading.Thread(target=pipe, args=(r, c), daemon=True).start()
'''

# The remote box's front: the launcher asks for a 200 at `/`, and the web host answers
# 404 there (the page is at the canvas path), so `/` is mapped to the page. See README.
FRONT = r'''
import http.server, sys, urllib.request
front, real, page = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass
    def do_GET(self):
        path = page if self.path == "/" else self.path
        try:
            r = urllib.request.urlopen(f"http://127.0.0.1:{real}{path}", timeout=10)
            code, body = r.status, r.read()
        except Exception as e:
            code, body = getattr(e, "code", 502), b"x"
        self.send_response(code); self.send_header("Content-Length", str(len(body))); self.end_headers()
        self.wfile.write(body)
http.server.ThreadingHTTPServer(("127.0.0.1", front), H).serve_forever()
'''


def run(ctx):
    v = Verdict("app/attach_remote")
    app.app_binary(ctx)
    procs = []
    try:
        keys = run_flow.fake_keys(ctx)
        ctx.secrets = keys
        real = app.free_port()
        ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                       "E2E_APP_PEM": keys["pem"]}, port=real, wire=["credentials"])
        v.check("the shift serves its web UI on its port", app.wait_http(f"http://127.0.0.1:{real}{CANVAS}", 30))
        remote = app.free_port()
        front = app.Proc([sys.executable, "-c", FRONT, str(remote), str(real), CANVAS], app.env_of(ctx.team),
                         ctx.run_dir, os.path.join(ctx.run_dir, "front.log"))
        procs.append(front)
        v.check("the remote box answers on a second port", app.wait_http(f"http://127.0.0.1:{remote}/", 20))
        ssh = os.path.join(ctx.run_dir, "fake_ssh")
        with open(ssh, "w") as f:
            f.write(FAKE_SSH)
        os.chmod(ssh, os.stat(ssh).st_mode | stat.S_IXUSR)

        t = app.run_app(ctx, ["--tunnel", f"fakehost:{remote}", "--ssh", ssh], name="tunnel")
        procs.append(t)
        line = t.wait_line("web UI at", 60) or ""
        m = re.search(r"web UI at (http://127\.0\.0\.1:(\d+)/)$", line.strip())
        v.check("the app prints a loopback URL for the tunnel", m is not None, t.log())
        local = int(m.group(2)) if m else 0
        status, body = web.http_get(m.group(1)) if m else (0, b"")
        v.check("the page answers through the tunnel", status == 200 and b"<html" in body.lower(), str(status))

        argv_path = os.path.join(ctx.run_dir, "ssh_argv.txt")
        argv = open(argv_path).read().split("\n")[:-1] if os.path.exists(argv_path) else []
        v.check("ssh got the forward bound to 127.0.0.1 only, and nothing else",
                argv == ["-N", "-o", "ExitOnForwardFailure=yes", "-L",
                         f"127.0.0.1:{local}:127.0.0.1:{remote}", "fakehost"], str(argv))
        v.check("no key, token, URL or user info in ssh's argv",
                not any(re.search(r"://|@|-i$|token|key|pem|password", a, re.I) for a in argv), str(argv))
        pid_path = os.path.join(ctx.run_dir, "ssh_pid.txt")
        ssh_pid = int(open(pid_path).read()) if os.path.exists(pid_path) else 0
        v.check("ssh is the app's child process", ssh_pid in app.child_pids(t.pid), f"{ssh_pid} {app.child_pids(t.pid)}")

        needles = {"model key": keys["model_key"], "app key text": keys["pem_chunk"]}
        out = t.log()
        for what, needle in needles.items():
            where = app.files_holding(ctx.run_dir, needle)
            v.check(f"the {what} is in no file of the run (outside secrets/)", where == [], str(where))
            v.check(f"the {what} is not in the app's output nor the page", needle not in out
                    and needle.encode() not in body)
        v.check("the app's output names no key and no user info",
                not re.search(r"@|api[_-]?key|BEGIN", out, re.I), out)

        t.signal(signal.SIGINT)
        v.check("the app exits 0 on SIGINT", t.wait(20) == 0, t.log())
        v.check("ssh is gone with it", ssh_pid > 0 and app.wait_gone(ssh_pid, 10))
        v.check("the tunnel's port is closed", app.wait_refused(local, 10))
        v.check("the shift on the other port is still running", web.up(real, CANVAS))

        # a URL with user info is refused, and its password is never echoed
        secret = "s3cr3tpw-9157"
        b = app.run_app(ctx, ["--attach", f"http://bob:{secret}@127.0.0.1:{remote}/"], name="userinfo")
        procs.append(b)
        v.check("--attach with user info exits 2, starts nothing, and does not echo the password",
                b.wait(20) == 2 and app.child_pids(b.pid) == [] and secret not in b.log(), b.log())
        # a host that ssh would read as an option is refused before ssh runs
        before = os.path.getmtime(argv_path) if os.path.exists(argv_path) else 0
        c = app.run_app(ctx, ["--tunnel", "-oProxyCommand=touch_x", "--ssh", ssh], name="optionhost")
        procs.append(c)
        v.check("--tunnel with a host that looks like an ssh option exits 2 and runs no ssh",
                c.wait(20) == 2 and (os.path.getmtime(argv_path) if os.path.exists(argv_path) else 0) == before, c.log())
    finally:
        for p in procs:
            p.reap()
    return v.finish()
