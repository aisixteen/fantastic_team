"""The page of the sidebar, driven end to end: workers and their subagents are siblings
in one list with different styles, subagents start collapsed and expand one worker at a
time, a click opens that agent's chat in the pane beside the list (the list stays) and a
message reaches that agent alone, and the expansion survives opening agents and Back.

There is no browser here: `helpers/page_driver.js` runs the served page's own script
(`page.js`) on a stub DOM, bridged to the live /team_ws/ws socket of the team (the page's
real leg, with its real grants). The team serves the page with the `webui` word. It needs
node >= 22 (a global WebSocket); without it the test SKIPs."""

import json
import os
import secrets
import time

from helpers import stock as S, team as team_mod
from helpers.page import Page, PageError, node_ok
from helpers.verdict import Verdict

NAME = "ui/sidebar_opens_agent_full_screen"
ROOT = "kernel_state"  # where the team's agents live


def worker_record(ctx, agent_id: str, parent: str, key_ref: str) -> dict:
    """A chat-capable agent on the scripted endpoint (what `agent_record` makes from worker_t)."""
    return {"type": "create_agent", "id": agent_id, "parent_id": parent,
            "handler_module": "anthropic_backend.tools", "model": "scripted-model",
            "endpoint": ctx.vars["model_endpoint"],
            "credentials": {"store": "credentials", "ref": key_ref},
            "memory_store_id": agent_id + "_memory",
            "can_call": {"$self": "*", "credentials": "*", agent_id + "_memory": "*"},
            "callable_by": {"$self": "*", "$parent": "*"},
            "context_window": 100000, "max_tokens": 4000}


def operator(team, **fields) -> None:
    r = team.send("kernel", fields)
    if r.get("status") != "completed" or "error" in (r.get("result") or {}):
        raise RuntimeError(f"cannot write {fields.get('id')}: {r}")


def grant_chat(team, chat_agents) -> None:
    """The app grants the page's leg the chat verbs of a group's workers, at
    start (`tree::grant_chat_to`). These agents are made after boot, so the operator writes
    the same grants. `update_agent` rewrites a field whole, so `team_ws` gets the union."""
    for aid in chat_agents:
        operator(team, type="update_agent", id=aid,
                 callable_by={"$self": "*", "$parent": "*", "team_ws": team_mod.CHAT_VERBS})
    operator(team, type="update_agent", id="team_ws",
             can_call={"$self": "*", "dispatcher": S.PAGE_VERBS, **{a: team_mod.CHAT_VERBS for a in chat_agents}})


def holders(page: Page, token: str, nodes) -> dict:
    """node -> whether its history (read over the page's socket) holds the token."""
    out = {}
    for n in nodes:
        r = page.cmd(cmd="call", target=n, payload={"type": "history"})["reply"]
        out[n] = token in json.dumps(r)
    return out


def row_cls(row) -> set:
    return set(row["cls"].split())


def run(ctx):
    v = Verdict(NAME)
    if not node_ok():
        print(f"SKIP {NAME}: needs node >= 22 (global WebSocket) to drive the page's script against the live socket")
        return 0
    # the stock records (the dispatcher with its group `fantastic`) and the lead, started
    # headless; the agents below are made next, then the binary serves the team page
    port = team_mod.free_port()
    S.boot(ctx, forge=False, groups={"fantastic": {"members": ["w1", "w2"]}})
    team = ctx.team
    ref = "model/" + ctx.vars["model_host"]
    agents = [("w1", ROOT), ("w2", ROOT), ("sub_a", "w2"), ("sub_b", "w2")]
    for aid, parent in agents:
        mem = {"type": "create_agent", "id": aid + "_memory", "parent_id": ROOT, "handler_module": "ram_store.tools",
               "callable_by": {"$self": "*", aid: "*"}}
        r = team.send("kernel", mem)
        if r.get("status") != "completed":
            raise RuntimeError(f"cannot make the memory of {aid}: {r}")
        r = team.send("kernel", worker_record(ctx, aid, parent, ref))
        if r.get("status") != "completed":
            raise RuntimeError(f"cannot make {aid}: {r}")
    S.merge(team, "credentials", "callable_by", {a: "*" for a, _ in agents})
    S.serve_webui(team, port)  # `webui --port N`: the binary makes the page, its leg and their grants
    grant_chat(team, [a for a, _ in agents])

    status, body = team_mod.http_get(port, "/team_page/file/index.html")
    _, script = team_mod.http_get(port, "/team_page/file/page.js")  # the page's script is its own file
    _, shim = team_mod.http_get(port, "/team_page/file/shim.js")
    v.check("the page is served (HTTP 200, the sidebar script and the socket shim)",
            status == 200 and "function renderSidebar(" in script and "window.fantastic = {" in shim,
            f"HTTP {status} {body[:300]}")

    page = Page(port, os.path.join(ctx.run_dir, "page_driver.err"))
    try:
        info = page.cmd(cmd="load")
        s = page.cmd(cmd="state")
        rows = s["rows"]
        by_node = {r["node"]: r for r in rows if r["node"]}
        exp = {r["expander"]: r for r in rows if r["expander"]}
        first = next((f for f in page.cmd(cmd="frames")["frames"] if (f.get("payload") or {}).get("type") == "sidebar"), {})
        items = next((g["items"] for g in (first.get("reply") or {}).get("groups", []) if g["id"] == "fantastic"), [])

        # b. same level, different styles
        v.check("the first sidebar reply lists both workers and both expanders in one flat list of group fantastic",
                [(i["kind"], i.get("id") or i.get("worker")) for i in items]
                == [("worker", "w1"), ("subagents", "w1"), ("worker", "w2"), ("subagents", "w2")], str(items))
        v.check("workers have style worker; expanders have style subagent",
                [i["style"] for i in items] == ["worker", "subagent", "worker", "subagent"], str(items))
        v.check("drawn: worker rows are `worker`, expander rows are `expander`, all siblings in the sidebar nav",
                {"w1", "w2"} <= set(by_node) and "worker" in row_cls(by_node["w1"]) and "worker" in row_cls(by_node["w2"])
                and set(exp) == {"w1", "w2"} and all("expander" in row_cls(r) for r in exp.values())
                and len({r["parent"] for r in rows}) == 1 and rows[0]["parent"] == "sidebar", str(rows))

        # c. collapsed by default
        v.check("the first sidebar request carried expanded: []", first.get("payload", {}).get("expanded") == [])
        v.check("both expanders are collapsed, with counts 0 and 2, and no subagent row is drawn",
                exp["w1"]["aria"] == "false" and exp["w2"]["aria"] == "false"
                and "subagents (0)" in exp["w1"]["text"] and "subagents (2)" in exp["w2"]["text"]
                and not any("sub" in row_cls(r) for r in rows), str(rows))

        # d. expand one worker only
        mark = page.cmd(cmd="frames")["total"]
        s = page.cmd(cmd="click_expander", worker="w2")
        asks = [f for f in page.cmd(cmd="frames", since=mark)["frames"] if (f.get("payload") or {}).get("type") == "sidebar"]
        rows = s["rows"]
        subs = [r for r in rows if "sub" in row_cls(r)]
        w2_at = next(i for i, r in enumerate(rows) if r["node"] == "w2")
        exp = {r["expander"]: r for r in rows if r["expander"]}
        v.check("the click on w2's expander asked for the sidebar with expanded: [w2]",
                bool(asks) and asks[-1]["payload"]["expanded"] == ["w2"], str(asks))
        v.check("sub_a and sub_b are drawn as `sub` rows after w2, siblings of the worker rows",
                [r["node"] for r in subs] == ["sub_a", "sub_b"] and all(r["parent"] == "sidebar" for r in subs)
                and all(rows.index(r) > w2_at for r in subs) and "worker" not in row_cls(subs[0]), str(rows))
        v.check("w1's expander is still collapsed and w1 has no subagent row",
                exp["w1"]["aria"] == "false" and exp["w2"]["aria"] == "true", str(rows))

        # e. a worker opens in the pane beside the list
        mark = page.cmd(cmd="frames")["total"]
        s = page.cmd(cmd="click_node", id="w1")
        fr = page.cmd(cmd="frames", since=mark)["frames"]
        opened = next((f["reply"] for f in fr if (f.get("payload") or {}).get("type") == "open"), {})
        v.check("open w1 answers view chat, target w1, fullscreen",
                opened.get("view") == "chat" and opened.get("target") == "w1" and opened.get("fullscreen") is True, str(opened))
        selected = [r["node"] for r in s["rows"] if "selected" in row_cls(r)]
        v.check("the pane is shown beside the list (the list stays), w1 is the selected row and the composer says `Message w1`",
                not s["paneHidden"] and not s["sidebarHidden"] and selected == ["w1"] and s["chat"]
                and s["chat"]["node"] == "w1" and s["chat"]["placeholder"] == "Message w1", str(s)[:400])
        v.check("the pane head has a Back button", s["hasBack"], str(s)[:300])

        # f. the message reaches that agent only
        tok1 = "TOK_" + secrets.token_hex(6)
        s = page.cmd(cmd="submit", text=tok1)
        end = time.time() + 30
        while time.time() < end and ctx.model.served < 1:
            time.sleep(0.2)
        time.sleep(1.0)
        v.check("the scripted model got exactly one request, and it holds the token",
                ctx.model.served == 1 and len(ctx.model.raw_bodies) == 1 and tok1 in ctx.model.raw_bodies[0],
                f"served {ctx.model.served}")
        h = holders(page, tok1, ["w1", "w2", "sub_a", "sub_b", ctx.lead])
        v.check("only w1's history holds the token (not w2, its subagents or the lead)",
                h == {"w1": True, "w2": False, "sub_a": False, "sub_b": False, ctx.lead: False}, str(h))

        # g. Back scrolls the list into view (the phone layout); the chat and the expansion stay
        s = page.cmd(cmd="click_back")
        v.check("Back brings the list into view, keeps w1's chat open and w2 expanded and w1 collapsed",
                "sidebar" in s["scrolls"] and not s["paneHidden"] and s["chat"] and s["chat"]["node"] == "w1"
                and [r["node"] for r in s["rows"] if "sub" in row_cls(r)] == ["sub_a", "sub_b"]
                and s["expanded"] == ["w2"], str(s["rows"]))
        mark = page.cmd(cmd="frames")["total"]
        s = page.cmd(cmd="click_node", id="sub_a")
        fr = [f for f in page.cmd(cmd="frames", since=mark)["frames"]]
        opened = next((f["reply"] for f in fr if (f.get("payload") or {}).get("type") == "open"), {})
        v.check("the subagent row opened (the expansion survived): view chat, target sub_a",
                opened.get("view") == "chat" and opened.get("target") == "sub_a" and opened.get("fullscreen") is True, str(opened))
        selected = [r["node"] for r in s["rows"] if "selected" in row_cls(r)]
        v.check("the pane targets sub_a alone: sub_a is the selected row, composer `Message sub_a`",
                not s["paneHidden"] and selected == ["sub_a"] and s["chat"] and s["chat"]["node"] == "sub_a"
                and s["chat"]["placeholder"] == "Message sub_a", str(s)[:400])
        tok2 = "TOK_" + secrets.token_hex(6)
        page.cmd(cmd="submit", text=tok2)
        end = time.time() + 30
        while time.time() < end and ctx.model.served < 2:
            time.sleep(0.2)
        time.sleep(1.0)
        fr = page.cmd(cmd="frames", since=mark)["frames"]
        chat_frames = [f for f in fr if (f.get("payload") or {}).get("type") in ("history", "send", "stop", "status") or f["frame"] == "watch"]
        v.check("every history, send and watch frame sent while sub_a was open names sub_a",
                bool(chat_frames) and all(f["target"] == "sub_a" for f in chat_frames), str(chat_frames))
        v.check("the second message cost the model exactly one more request, and it holds the token",
                ctx.model.served == 2 and tok2 in ctx.model.raw_bodies[-1], f"served {ctx.model.served}")
        h = holders(page, tok2, ["w1", "w2", "sub_a", "sub_b", ctx.lead])
        v.check("only sub_a's history holds the second token",
                h == {"w1": False, "w2": False, "sub_a": True, "sub_b": False, ctx.lead: False}, str(h))

        # h. the expansion is kept while agents are opened
        s = page.cmd(cmd="state")
        rows = s["rows"]
        exp = {r["expander"]: r for r in rows if r["expander"]}
        v.check("the list is still there with w2's subagent rows drawn and w1 collapsed",
                not s["sidebarHidden"] and [r["node"] for r in rows if "sub" in row_cls(r)] == ["sub_a", "sub_b"]
                and exp["w2"]["aria"] == "true" and exp["w1"]["aria"] == "false", str(rows))
        v.check("the expansion is remembered per worker (fantastic.team.subagents.w2 = 1, none for w1)",
                s["remembered"].get("fantastic.team.subagents.w2") == "1"
                and "fantastic.team.subagents.w1" not in s["remembered"], str(s["remembered"]))
    except PageError as e:
        v.check("every step of the page can be done", False, str(e))
    finally:
        page.close()
    team.stop()
    return v.finish()
