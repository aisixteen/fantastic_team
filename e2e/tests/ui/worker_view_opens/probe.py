"""The worker view in the real binary: the verb leg and the web UI leg."""

import os
import re
import time

from helpers import app, run_flow, team as T
from helpers.verdict import Verdict
from helpers.web import WsClient, http_get

PAGE = "/canvas_page/file/index.html"
WS = "/canvas_ws/ws"
TOP = ["task", "files", "output", "preview"]


def run(ctx):
    v = Verdict("ui/worker_view_opens")
    # boot without helpers.team.wire: that wires the github flow, and this team has none
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    team = ctx.team = T.Team(ctx.binary, ctx.run_dir, {"E2E_MODEL_KEY": keys["model_key"]})
    port = app.free_port()
    T.boot(team, os.path.join(ctx.dir, "team.yaml"), ctx.vars, port=port)

    # two workers on the scripted model, and the operator's grants a team.yaml cannot say
    for w in ("w1", "w2"):
        r = team.send("kernel", {"type": "create_agent", "id": w, "parent_id": "kernel",
                                 "handler_module": "anthropic_backend.tools",
                                 "model": "scripted-model", "endpoint": ctx.model.endpoint,
                                 "memory_store_id": "lead_memory",
                                 "credentials": {"store": "credentials",
                                                 "ref": f"model/{ctx.vars['model_host']}"},
                                 "can_call": {"$self": "*", "queue": "*"}})
        v.check(f"{w} was created", r.get("status") == "completed", str(r))
    r = team.send("kernel", {"type": "update_agent", "id": "credentials",
                             "callable_by": {"$self": "*", "lead": "*", "w1": "*", "w2": "*"}})
    v.check("the credential store admits the workers", r.get("status") == "completed", str(r))
    for agent in ("worker_view", "dispatcher"):
        r = team.send("kernel", {"type": "update_agent", "id": agent,
                                 "callable_by": {"$self": "*", "$parent": "*", "mcp": "*"}})
        v.check(f"the operator may call {agent}", r.get("status") == "completed", str(r))

    v.check("the page is served on --port", app.wait_http(f"http://127.0.0.1:{port}{PAGE}", 30))

    def wv(payload):
        return team.send("worker_view", payload).get("result", {})

    # 1. verb leg: the view of each worker
    w1 = wv({"type": "open", "worker": "w1", "group": "workers"})
    w2 = wv({"type": "open", "worker": "w2", "group": "workers"})
    kinds = [c.get("kind") for c in w1.get("cells", [])]
    v.check("w1's view is bound to w1, in the mobile layout",
            w1.get("worker") == "w1" and w1.get("layout") == "mobile", str(w1))
    v.check("the default top cells are task, files, output, preview, then chat",
            kinds == TOP + ["chat"], str(kinds))
    chat = (w1.get("cells") or [{}])[-1]
    v.check("the chat cell is the last cell, locked, and targets w1 only",
            chat.get("kind") == "chat" and chat.get("locked") is True and chat.get("target") == "w1",
            str(chat))
    v.check("no other cell is locked or has a target",
            all(not c.get("locked") and not c.get("target") for c in w1["cells"][:-1]), str(w1))
    v.check("w2's chat cell targets w2, not w1",
            w2.get("cells", [{}])[-1].get("target") == "w2" and w2.get("worker") == "w2", str(w2))
    v.check("opening w1 again answers the same view", wv({"type": "open", "worker": "w1"}) == w1)

    # 2. next and prev move within the group, and wrap
    v.check("next of w1 is w2", wv({"type": "next", "worker": "w1"}).get("worker") == "w2")
    v.check("next of w2 wraps to w1", wv({"type": "next", "worker": "w2"}).get("worker") == "w1")
    v.check("prev of w1 wraps to w2", wv({"type": "prev", "worker": "w1"}).get("worker") == "w2")
    v.check("prev of w2 is w1", wv({"type": "prev", "worker": "w2"}).get("worker") == "w1")

    # 3. place goes into the top part; the chat stays last and locked
    placed = wv({"type": "place", "worker": "w1", "cell": {"kind": "note", "label": "Note",
                                                            "target": None, "locked": False}})
    pk = [c.get("kind") for c in placed.get("cells", [])]
    v.check("a placed cell goes before the chat cell",
            pk == TOP + ["note", "chat"] and placed["cells"][-1].get("locked") is True, str(placed))
    refused = wv({"type": "place", "worker": "w1", "cell": {"kind": "chat", "label": "x",
                                                             "target": "w2", "locked": False}})
    v.check("placing a chat cell is refused", "error" in refused, str(refused))

    # 4. web leg: what the binary's --port serves, and what its socket may do
    status, page = http_get(f"http://127.0.0.1:{port}{PAGE}")
    page = page.decode(errors="replace")
    v.check("the web UI page is served on --port", status == 200 and "<html" in page.lower(),
            f"{status}")
    # the dispatcher page and its socket are the in-process app's (TeamHandle); the binary's
    # --port must not serve them: shown here, so the skip below is a fact, not a claim
    status, _ = http_get(f"http://127.0.0.1:{port}/team_page/file/index.html")
    v.check("the binary serves no dispatcher page (/team_page)", status == 404, f"{status}")
    # the host may accept the websocket upgrade on any path; what matters is that no
    # dispatcher answers behind /team_ws
    leg = None
    try:
        leg = WsClient(port, "/team_ws/ws")
        got, err = leg.call("dispatcher", {"type": "open", "node": "w1"}, timeout=10), None
    except (RuntimeError, OSError) as e:
        got, err = None, str(e)
    finally:
        if leg:
            leg.close()
    print(f"  (team_ws -> dispatcher open w1: reply={got!r} error={err!r})")
    v.check("nothing behind /team_ws opens a worker", not (isinstance(got, dict) and got.get("view")),
            f"{got} {err}")

    ws = WsClient(port, WS)
    try:
        for target, payload in (("worker_view", {"type": "open", "worker": "w1"}),
                                ("dispatcher", {"type": "open", "node": "w1"}),
                                ("lead", {"type": "send", "text": "hi"}),
                                ("w1", {"type": "send", "text": "hi"})):
            got, err = None, None
            try:
                got = ws.call(target, payload)
            except (RuntimeError, OSError) as e:
                err = str(e)
            print(f"  (canvas_ws -> {target}: reply={got!r} error={err!r})")
            refused = err is not None and "closed" not in err or (
                isinstance(got, dict) and (got.get("reason") == "unauthorized" or got.get("refused")))
            v.check(f"the page's socket may not call {target}", refused, f"{got} {err}")
    finally:
        ws.close()
    v.check("nothing reached the model through the page's socket", ctx.model.served == 0,
            f"served {ctx.model.served}")

    # chat reaches only the bound worker: the verb leg (the operator's `send`), since the
    # binary's socket is not granted any worker
    mark = os.path.getsize(team.log_path)
    r = team.send("w1", {"type": "send", "text": "hi"})
    v.check("a message to w1 is answered", r.get("status") == "completed", str(r))
    end, last, since = time.time() + 30, -1, time.time()
    while time.time() < end:  # until the model count has been still for 3 s
        if ctx.model.served != last:
            last, since = ctx.model.served, time.time()
        elif last >= 1 and time.time() - since > 3:
            break
        time.sleep(0.2)
    v.check("w1's message reached the model", ctx.model.served >= 1, f"served {ctx.model.served}")
    with open(team.log_path, "rb") as f:
        f.seek(mark)
        after = f.read().decode(errors="replace")
    mine = [ln for ln in after.splitlines() if re.search(r"\] w1 (call|task) ", ln)]
    other = [ln for ln in after.splitlines() if re.search(r"\] (w2|lead) (call|task) ", ln)]
    v.check("w1 handled the message", bool(mine), "no w1 line in the team log")
    v.check("w2 and the lead handled nothing", not other, str(other[:3]))

    # 5. not built: see README.md
    print("  [skipped] the page opens a worker as a full-screen chat bound to it (dispatcher "
          "`open`): shown above that the binary's --port has no /team_page, and that a socket "
          "opened on /team_ws/ws is the canvas_ws leg, which may not call the dispatcher; the "
          "dispatcher page and its leg (team_page, team_ws, with grant_chat_to for the "
          "workers) are made only by the in-process app (TeamHandle), which a probe cannot start")
    print("  [skipped] no horizontal scroll at 360x740 and desktop: no page serves the worker "
          "view (the web UI opens the chat only); needs a headless browser, and the stdlib-only "
          "harness has no CDP client")

    code = team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
    return v.finish()
