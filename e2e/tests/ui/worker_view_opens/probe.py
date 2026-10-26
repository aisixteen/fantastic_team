"""The worker view in the real binary: the verb leg and the web UI leg."""

import os
import re
import time

from helpers import app, run_flow, team as T
from helpers.verdict import Verdict
from helpers.web import WsClient, http_get

PAGE = "/canvas_page/file/index.html"
TEAM_PAGE = "/team_page/file/index.html"
WS = "/team_ws/ws"
CHAT_VERBS = ["history", "send", "stop", "status", "watch"]  # tree::CHAT_VERBS
PAGE_VERBS = ["sidebar", "tree", "open", "control"]  # app_control::PAGE_VERBS
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
    # the binary grants the page's leg the workers a team.yaml names, at start; these two
    # are made after boot, so the operator grants them as `tree::grant_chat_to` would
    for w in ("w1", "w2"):
        r = team.send("kernel", {"type": "update_agent", "id": w,
                                 "callable_by": {"$self": "*", "$parent": "*", "team_ws": CHAT_VERBS}})
        v.check(f"the page's leg may chat with {w}", r.get("status") == "completed", str(r))
    r = team.send("kernel", {"type": "update_agent", "id": "team_ws",
                             "can_call": {"$self": "*", "dispatcher": PAGE_VERBS,
                                          "w1": CHAT_VERBS, "w2": CHAT_VERBS}})
    v.check("the page's leg may call the dispatcher and the workers", r.get("status") == "completed", str(r))
    for agent, leg_verbs in (("worker_view", None), ("dispatcher", PAGE_VERBS)):
        # update_agent rewrites callable_by whole: keep the page's leg on the dispatcher
        by = {"$self": "*", "$parent": "*", "mcp": "*"}
        if leg_verbs:
            by["team_ws"] = leg_verbs
        r = team.send("kernel", {"type": "update_agent", "id": agent, "callable_by": by})
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
    # the dispatcher page and its socket are the same ones the app serves (`serve_web`)
    status, tpage = http_get(f"http://127.0.0.1:{port}{TEAM_PAGE}")
    tpage = tpage.decode(errors="replace")
    v.check("the binary serves the dispatcher page (/team_page)",
            status == 200 and "function renderSidebar(" in tpage, f"{status}")

    ws = WsClient(port, WS)
    try:
        def leg(target, payload):
            try:
                return ws.call(target, payload, timeout=30), None
            except (RuntimeError, OSError) as e:
                return None, str(e)

        got, err = leg("dispatcher", {"type": "open", "node": "w1"})
        v.check("the page's socket opens w1 as a full-screen chat bound to it",
                isinstance(got, dict) and got.get("view") == "chat" and got.get("target") == "w1"
                and got.get("fullscreen") is True, f"{got} {err}")
        for target, payload in (("worker_view", {"type": "open", "worker": "w1"}),
                                ("lead", {"type": "send", "text": "hi"})):
            got, err = leg(target, payload)
            print(f"  (team_ws -> {target}: reply={got!r} error={err!r})")
            refused = err is not None and "closed" not in err or (
                isinstance(got, dict) and (got.get("reason") == "unauthorized" or got.get("refused")))
            v.check(f"the page's socket may not call {target}", refused, f"{got} {err}")
        v.check("nothing reached the model through the refused calls", ctx.model.served == 0,
                f"served {ctx.model.served}")

        # chat reaches only the bound worker, over the page's own socket
        mark = os.path.getsize(team.log_path)
        got, err = leg("w1", {"type": "send", "text": "hi"})
        v.check("a message to w1 over the page's socket is answered",
                isinstance(got, dict) and "error" not in got and got.get("final") == "hello from w1",
                f"{got} {err}")
    finally:
        ws.close()
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

    # not built: see README.md
    print("  [skipped] no horizontal scroll at 360x740 and desktop: needs a headless browser, "
          "and the stdlib-only harness has no CDP client")

    code = team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
    return v.finish()
