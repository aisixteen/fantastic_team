"""The connector setup flow of the app: the manifest page, a wrong callback, no key on disk."""

import html
import json
import os
import re
import signal

from helpers import app, run_flow, web
from helpers.verdict import Verdict

# setup_github.rs PERMISSIONS: the six the app asks for, nothing else.
WANT = {"contents": "write", "issues": "write", "pull_requests": "write",
        "metadata": "read", "checks": "read", "actions": "read"}
NAME = "e2e-app"


def run(ctx):
    v = Verdict("app/connector_setup")
    app.app_binary(ctx)
    procs = []
    try:
        keys = run_flow.fake_keys(ctx)
        ctx.secrets = keys
        ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                       "E2E_APP_PEM": keys["pem"]}, wire=["credentials"])
        ctx.team.stop()  # the directory's lock is free for the setup flow

        s = app.run_app(ctx, ["--setup-github", NAME, "--binary", ctx.binary], name="setup")
        procs.append(s)
        line = s.wait_line("web UI at", 60) or ""
        m = re.search(r"web UI at (http://127\.0\.0\.1:(\d+)/)\s*$", line)
        v.check("the app prints the setup page URL on loopback", m is not None, s.log())
        if not m:
            return v.finish()
        url, port = m.group(1), int(m.group(2))
        v.check("the app has the setup child (one process)", len(app.child_pids(s.pid)) == 1, s.log())

        status, body = web.http_get(url)
        page = body.decode()
        form = re.search(r'<form id="f" method="post" action="([^"]*)">', page)
        man = re.search(r'name="manifest" value="([^"]*)"', page)
        v.check("the setup page is a form that posts a manifest", status == 200 and form and man, page[:300])
        action = html.unescape(form.group(1)) if form else ""
        state = (re.search(r"state=([0-9a-f]+)$", action) or [None, ""])[1]
        v.check("it posts to github.com with a random state (128 bits)",
                action.startswith("https://github.com/settings/apps/new?state=") and len(state) == 32, action)
        manifest = json.loads(html.unescape(man.group(1))) if man else {}
        v.check("the manifest carries the name the user chose", manifest.get("name") == NAME, str(manifest.get("name")))
        v.check("it asks for exactly the six permissions, no more",
                manifest.get("default_permissions") == WANT, str(manifest.get("default_permissions")))
        v.check("it has no webhook and no event", manifest.get("hook_attributes") == {"active": False}
                and "url" not in manifest["hook_attributes"] and manifest.get("default_events") == [],
                str(manifest.get("hook_attributes")))
        v.check("its callback is this machine's loopback", manifest.get("redirect_url") == f"http://127.0.0.1:{port}/callback"
                and manifest.get("public") is False, str(manifest.get("redirect_url")))
        hosts = set(re.findall(r"https?://([^/\"'\s?:]+)", html.unescape(page)))
        v.check("the page names no host but github.com and this machine", hosts <= {"github.com", "127.0.0.1"}, str(hosts))

        v.check("another path is not served", web.http_get(f"http://127.0.0.1:{port}/other")[1] == b"not found")
        bad = web.http_get(f"http://127.0.0.1:{port}/callback?code=x&state=WRONG")
        v.check("a callback with a wrong state is refused", b"State did not match" in bad[1], str(bad))
        code = s.wait(30)
        v.check("the flow ends without registering anything: the app exits non-zero, no child left",
                code not in (None, 0) and app.child_pids(s.pid) == [], f"{code}\n{s.log()[-400:]}")
        v.check("the setup page is closed", app.wait_refused(port, 10))

        cfg = open(os.path.join(ctx.run_dir, "team.yaml")).read()
        v.check("team.yaml has no github_app_id", "github_app_id" not in cfg)
        for what, needle in (("app key text", keys["pem_chunk"]), ("private key marker", "PRIVATE KEY"),
                             ("model key", keys["model_key"])):
            where = app.files_holding(ctx.run_dir, needle)
            v.check(f"no file under the run holds the {what} (outside secrets/)", where == [], str(where))
        v.check("the app's output holds no key", keys["pem_chunk"] not in s.log() and "PRIVATE KEY" not in s.log())

        # cancelling leaves nothing running
        c = app.run_app(ctx, ["--setup-github", NAME, "--binary", ctx.binary], name="setup_cancel")
        procs.append(c)
        v.check("a second setup starts", c.wait_line("web UI at", 60) is not None, c.log())
        kid = (app.child_pids(c.pid) or [0])[0]
        c.signal(signal.SIGINT)
        v.check("SIGINT cancels it: exit 0, the setup child is gone", c.wait(20) == 0 and kid > 0
                and app.wait_gone(kid, 10), c.log()[-300:])
    finally:
        for p in procs:
            p.reap()
    return v.finish()
