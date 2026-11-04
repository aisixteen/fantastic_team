"""The app launcher: start a headless child, attach to a server that is up, stop."""

import os
import signal
import subprocess

from helpers import app, records, run_flow, team as team_mod, web
from helpers.verdict import Verdict


def seed_dir(ctx, folder, name, procs, v, setup):
    """Run the binary once in `folder` so it is seeded, let `setup(team)` make its agents with
    `send`, and stop it."""
    port = app.free_port()
    seed = app.Proc([ctx.binary, "canvas", "--port", str(port)], app.env_of(ctx.team),
                    folder, os.path.join(ctx.run_dir, f"{name}.log"))
    procs.append(seed)
    v.check(f"{name}: a first run seeds the directory",
            app.wait_http(f"http://127.0.0.1:{port}/canvas_page/file/index.html", 30), seed.log())
    team = team_mod.Team(ctx.binary, folder, app.env_of(ctx.team))
    setup(team)
    seed.signal(signal.SIGINT)
    v.check(f"{name}: the seeding run stops", seed.wait(30) is not None, seed.log())
    return team


def key_mode(team):
    """The key mode fantastic's set-up agent reports for the running kernel (None: not chosen)."""
    records.create(team, {"id": "mode_probe", "parent_id": "user", "handler_module": "setup.tools"})
    m = team.send("mode_probe", {"type": "key_mode"})["result"].get("mode")
    team.send("kernel", {"type": "delete_agent", "id": "mode_probe"})
    return m


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
        # The launcher makes a team (`create_agent` under fixed ids) in a folder whose kernel has
        # no `lead`; a folder that has its `lead` is started as it is, so this one brings its own.
        # A never-used directory is seeded by the first `canvas --port` start and serves the
        # page from the next, so seed it once before the launcher starts it.
        seed_dir(ctx, start_dir, "seed", procs, v, lambda t: records.create(
            t, {"id": "lead", "parent_id": "user", "handler_module": "ram_store.tools"}))
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

        # --- start in an empty folder on a fresh home: the app chooses fantastic's plain key mode
        # (so the credential store exists), then makes the team with create_agent under fixed ids
        # (no blueprint, no import)
        new_dir = os.path.join(ctx.run_dir, "new_team_dir")
        os.makedirs(new_dir)
        before = {}

        def look(t):
            before["agents"], before["mode"] = t.agents(), key_mode(t)

        seed_dir(ctx, new_dir, "seed_new", procs, v, look)
        v.check("the fresh home has no credential store and no key mode before the app starts",
                "credentials" not in before.get("agents", {"credentials": 1}) and before.get("mode", 1) is None,
                str(before.get("mode")))
        n = app.free_port()
        s2 = app.run_app(ctx, ["--binary", ctx.binary, "--port", n, "--dir", new_dir], name="start_new")
        procs.append(s2)
        line = s2.wait_line("web UI at", 60)
        v.check("start in a folder with no team prints the web UI URL", bool(line), s2.log())
        t2 = team_mod.Team(ctx.binary, new_dir, app.env_of(ctx.team))
        agents = t2.agents() if line else {}
        v.check("the app chose fantastic's plain key mode on the fresh home",
                bool(line) and key_mode(t2) == "plain")
        want_ids = {"credentials", "lead", "queue", "github", "board", "roles", "tasks", "worker_exec", "worker_files"}
        v.check("the app made the team under the fixed ids the role cards name",
                want_ids <= set(agents), str(sorted(set(agents))))
        v.check("no id carries an import's suffix", not [i for i in agents if i.endswith("_1")],
                str(sorted(agents)))
        v.check("the roles registry, the queue and the board sit under the lead",
                all(agents.get(i, {}).get("parent_id") == "lead" for i in ("roles", "queue", "board", "tasks")),
                str({i: agents.get(i, {}).get("parent_id") for i in want_ids}))
        mem = agents.get("memory", {}).get("callable_by", {})
        v.check("the other end of the lead's edge is on the home's memory, and what it had stays",
                mem.get("lead") == "*" and mem.get("user") in ("*", ["*"]), str(mem))
        v.check("the role cards were written next to the kernel",
                os.path.isfile(os.path.join(new_dir, "roles", "project_lead.md")))
        s2.signal(signal.SIGINT)
        v.check("start exits 0 on SIGINT", s2.wait(20) == 0, s2.log())
    finally:
        for pr in procs:
            pr.reap()
    return v.finish()
