"""The app launcher: start a headless child, attach to a server that is up, stop."""

import os
import shutil
import signal
import subprocess

from helpers import app, run_flow, web
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("app/launcher_start_attach_stop")
    app.app_binary(ctx)  # a Skip when the app is not built next to --bin
    procs = []
    try:
        q = app.free_port()
        run_flow.boot(ctx, forge=False, port=q)
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
        # The real binary, fantastic's `canvas --port N` run in the team folder. The launcher waits
        # for the page that command serves; the web host answers 404 at `/`.
        p = app.free_port()
        start_dir = os.path.join(ctx.run_dir, "start_dir")
        os.makedirs(start_dir)
        # The launcher writes the app's preset into a folder with no `blueprint/` and imports it; a
        # folder that has one is started as it is, so this one brings its own (the test's blueprint).
        shutil.copytree(os.path.join(ctx.dir, "blueprint"), os.path.join(start_dir, "blueprint"))
        # A never-used directory is seeded by the first `canvas --port` start and serves the
        # page from the next, so seed it once before the launcher starts it.
        sp = app.free_port()
        seed = app.Proc([ctx.binary, "canvas", "--port", str(sp)], app.env_of(ctx.team),
                        start_dir, os.path.join(ctx.run_dir, "seed.log"))
        procs.append(seed)
        v.check("a first run seeds the directory", app.wait_http(f"http://127.0.0.1:{sp}/canvas_page/file/index.html", 30), seed.log())
        seed.signal(signal.SIGINT)
        v.check("the seeding run stops", seed.wait(30) is not None, seed.log())
        s = app.run_app(ctx, ["--binary", ctx.binary, "--port", p, "--dir", start_dir], name="start")
        procs.append(s)
        want = f"http://127.0.0.1:{p}/"
        page = want + "canvas_page/file/index.html"
        line = s.wait_line("web UI at", 40)
        v.check("start prints the web UI URL (the page `canvas --port` serves)",
                bool(line) and line.strip().endswith(page), s.log())
        kids = app.child_pids(s.pid)
        v.check("start has exactly one child process", len(kids) == 1, str(kids))
        v.check("the child answers 200 on the page path", app.wait_http(page, 10))
        v.check("the child answers 404 at / (the launcher waits on the page path)",
                web.http_get(want)[0] == 404)
        args = ""
        if kids:
            args = subprocess.run(["ps", "-o", "args=", "-p", str(kids[0])],
                                  capture_output=True, text=True).stdout.strip()
        cwd = ""
        if kids:  # fantastic runs in the folder it is started in: that is how it gets the dir
            cwd = subprocess.run(["lsof", "-a", "-d", "cwd", "-p", str(kids[0]), "-Fn"],
                                 capture_output=True, text=True).stdout
        v.check("the child got fantastic's `canvas --port N` and nothing else",
                args.endswith(f"canvas --port {p}"), args)
        v.check("it runs in the team directory", os.path.realpath(start_dir) in
                [os.path.realpath(l[1:]) for l in cwd.splitlines() if l.startswith("n")] or not cwd, cwd)
        s.signal(signal.SIGINT)
        v.check("start exits 0 on SIGINT", s.wait(20) == 0, s.log())
        v.check("stop leaves no child process", bool(kids) and app.wait_gone(kids[0], 10), str(kids))
        v.check("the port refuses connections after stop", app.wait_refused(p, 10))
        v.check("stop did not touch the attached server", web.up(q, "/canvas_page/file/index.html"))
    finally:
        for pr in procs:
            pr.reap()
    return v.finish()
