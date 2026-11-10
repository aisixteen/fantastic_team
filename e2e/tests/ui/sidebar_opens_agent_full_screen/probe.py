"""The page's socket, driven end to end: workers and their subagents are siblings in one
list with different styles, subagents start collapsed and expand one worker at a time, `open`
answers that agent's chat, and a message sent over the page's socket reaches that agent alone.

The page itself is two empty columns for now (ui/layout_two_columns draws it in a browser);
this test speaks to the team over the page's real leg, `/team_ws/ws`, with its real grants,
the way the page's shim does. The team serves the page with the `webui` word."""

import json
import secrets
import time

from helpers import stock as S, team as team_mod
from helpers.verdict import Verdict
from helpers.web import WsClient

NAME = "ui/sidebar_opens_agent_full_screen"


def holders(ws, token: str, nodes) -> dict:
    """node -> whether its history (read over the page's socket) holds the token."""
    return {n: token in json.dumps(ws.call(n, {"type": "history"}, timeout=30)) for n in nodes}


def run(ctx):
    v = Verdict(NAME)
    # the fleet is laid before the start (`fleet:` of test.yaml): the dispatcher with its group
    # `fantastic`, the lead, the workers and their subagents, the page, its leg and its grants;
    # started headless, then stopped and started again as `webui --port N`
    port = team_mod.free_port()
    ctx.vars["web_port"] = port
    S.boot(ctx)
    team = ctx.team
    S.serve_webui(team, port)

    status, body = team_mod.http_get(port, "/team_page/file/index.html")
    _, shim = team_mod.http_get(port, "/team_page/file/shim.js")
    v.check("the page is served (HTTP 200, the socket shim)",
            status == 200 and "window.fantastic = {" in shim, f"HTTP {status} {body[:300]}")

    ws = WsClient(port, "/team_ws/ws")
    try:
        def leg(target, payload):
            return ws.call(target, payload, timeout=30)

        # b. same level, different styles; c. collapsed by default
        first = leg("dispatcher", {"type": "sidebar", "expanded": []})
        items = next((g["items"] for g in first.get("groups", []) if g["id"] == "fantastic"), [])
        v.check("the sidebar reply lists both workers and both expanders in one flat list of group fantastic",
                [(i["kind"], i.get("id") or i.get("worker")) for i in items]
                == [("worker", "w1"), ("subagents", "w1"), ("worker", "w2"), ("subagents", "w2")], str(items))
        v.check("workers have style worker; expanders have style subagent",
                [i["style"] for i in items] == ["worker", "subagent", "worker", "subagent"], str(items))
        v.check("collapsed by default: no subagent is listed", "sub_a" not in json.dumps(first), str(items))

        # d. expand one worker only
        second = leg("dispatcher", {"type": "sidebar", "expanded": ["w2"]})
        text = json.dumps(second)
        v.check("expanded: [w2] lists sub_a and sub_b",
                "sub_a" in text and "sub_b" in text, text[:400])

        # e. a worker opens as a chat bound to it
        opened = leg("dispatcher", {"type": "open", "node": "w1"})
        v.check("open w1 answers view chat, target w1, fullscreen",
                opened.get("view") == "chat" and opened.get("target") == "w1" and opened.get("fullscreen") is True,
                str(opened))

        # f. the message reaches that agent only
        tok1 = "TOK_" + secrets.token_hex(6)
        leg("w1", {"type": "send", "text": tok1})
        end = time.time() + 30
        while time.time() < end and ctx.model.served < 1:
            time.sleep(0.2)
        time.sleep(1.0)
        v.check("the scripted model got exactly one request, and it holds the token",
                ctx.model.served == 1 and len(ctx.model.raw_bodies) == 1 and tok1 in ctx.model.raw_bodies[0],
                f"served {ctx.model.served}")
        h = holders(ws, tok1, ["w1", "w2", "sub_a", "sub_b", ctx.lead])
        v.check("only w1's history holds the token (not w2, its subagents or the lead)",
                h == {"w1": True, "w2": False, "sub_a": False, "sub_b": False, ctx.lead: False}, str(h))

        # g. a subagent opens and is messaged alone
        opened = leg("dispatcher", {"type": "open", "node": "sub_a"})
        v.check("the subagent opens: view chat, target sub_a",
                opened.get("view") == "chat" and opened.get("target") == "sub_a" and opened.get("fullscreen") is True,
                str(opened))
        tok2 = "TOK_" + secrets.token_hex(6)
        leg("sub_a", {"type": "send", "text": tok2})
        end = time.time() + 30
        while time.time() < end and ctx.model.served < 2:
            time.sleep(0.2)
        time.sleep(1.0)
        v.check("the second message cost the model exactly one more request, and it holds the token",
                ctx.model.served == 2 and tok2 in ctx.model.raw_bodies[-1], f"served {ctx.model.served}")
        h = holders(ws, tok2, ["w1", "w2", "sub_a", "sub_b", ctx.lead])
        v.check("only sub_a's history holds the second token",
                h == {"w1": False, "w2": False, "sub_a": True, "sub_b": False, ctx.lead: False}, str(h))
    except (RuntimeError, OSError) as e:
        v.check("every step over the page's socket can be done", False, str(e))
    finally:
        ws.close()
    team.stop()
    return v.finish()
