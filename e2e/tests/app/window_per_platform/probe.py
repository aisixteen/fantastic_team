"""The packaged window: opens on the web UI, quit stops the headless child.

Needs what only the maintainer's machines have: the app built with `--features tauri`
(E2E_APP_TAURI_BIN names it, absolute), a Linux display (DISPLAY, Xvfb is fine) with
WebKitGTK, and xdotool. Without them it is a Skip, never a failure. The real window run
on macOS, Windows and Linux, by hand, is the maintainer's; this is the scripted Linux leg.
"""

import os
import shutil
import subprocess
import sys
import time

from helpers import app, run_flow, web
from helpers.verdict import Skip, Verdict


def missing():
    out = []
    tauri = os.environ.get("E2E_APP_TAURI_BIN", "")
    if not (tauri and os.path.isabs(tauri) and os.path.isfile(tauri)):
        out.append("no tauri build (E2E_APP_TAURI_BIN)")
    if not sys.platform.startswith("linux"):
        out.append(f"not Linux ({sys.platform})")
    elif not os.environ.get("DISPLAY"):
        out.append("no DISPLAY (Xvfb)")
    if not shutil.which("xdotool"):
        out.append("no xdotool")
    return out


def run(ctx):
    gone = missing()
    if gone:
        raise Skip("no Linux display or tauri build here: " + ", ".join(gone))
    v = Verdict("app/window_per_platform")
    tauri = os.environ["E2E_APP_TAURI_BIN"]
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"]}, wire=["credentials"])
    ctx.team.stop()
    port = app.free_port()
    env = dict(app.env_of(ctx.team))
    for k in ("DISPLAY", "XAUTHORITY", "XDG_RUNTIME_DIR"):
        if os.environ.get(k):
            env[k] = os.environ[k]
    w = app.Proc([tauri, "--binary", ctx.binary, "--port", str(port), "--dir", ctx.run_dir], env,
                 ctx.run_dir, os.path.join(ctx.run_dir, "window.log"))
    try:
        line = w.wait_line("web UI at", 90)
        v.check("the app prints the web UI URL", line is not None, w.log())
        url = line.split("web UI at", 1)[1].strip() if line else ""
        v.check("the web UI answers", bool(url) and app.wait_http(url, 20), url)
        v.check("one headless child", len(app.child_pids(w.pid)) == 1, str(app.child_pids(w.pid)))
        kid = (app.child_pids(w.pid) or [0])[0]
        wid = ""
        end = time.time() + 30
        while time.time() < end and not wid:
            r = subprocess.run(["xdotool", "search", "--pid", str(w.pid), "--name", "Fantastic Team"],
                               capture_output=True, text=True)
            wid = (r.stdout.split() or [""])[0]
            time.sleep(0.5)
        v.check("a window titled `Fantastic Team` is open", bool(wid))
        subprocess.run(["xdotool", "windowquit", wid] if wid else ["true"], capture_output=True)
        v.check("closing the window ends the app", w.wait(30) is not None, w.log()[-400:])
        v.check("the headless child is gone", kid > 0 and app.wait_gone(kid, 15))
        v.check("the port refuses connections", app.wait_refused(port, 10))
    finally:
        w.reap()
    return v.finish()
