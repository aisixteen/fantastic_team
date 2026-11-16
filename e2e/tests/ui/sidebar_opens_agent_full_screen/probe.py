"""The page's sidebar and socket, driven end to end: the tree the page's left column draws (the base
sidebar agent's html view, over the root's full `reflect`) holds the workers and, under w2, its
subagents; `open` answers that agent's chat, and a message sent over the page's socket reaches that
agent alone.

Every call goes over the page's real leg, `/team_ws/ws`, with its real grants, the way the page's
shim does. The team serves the page with the `webui` word."""

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

        def children(node, id_):
            """The child ids of `id_` in the tree `node`, or None when it is not there."""
            if node.get("id") == id_:
                return [k.get("id") for k in node.get("children") or []]
            for k in node.get("children") or []:
                found = children(k, id_)
                if found is not None:
                    return found
            return None

        # b. the left column is the sidebar agent's html view, the page's first call
        side = leg("sidebar", {"type": "reflect"})
        v.check("the sidebar agent answers the page's leg with its html view",
                (side.get("view") or {}).get("kind") == "html", str(side)[:300])

        # c. the tree that view draws: the workers, and w2's subagents under w2 alone
        tree = leg("kernel", {"type": "reflect", "detail": "full"}).get("tree") or {}
        kids = {w: children(tree, w) for w in ["w1", "w2"]}
        v.check("the tree holds both workers; sub_a and sub_b are w2's children, w1 has none",
                kids["w1"] == [] and sorted(kids["w2"] or []) == ["sub_a", "sub_b"], str(kids))

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
