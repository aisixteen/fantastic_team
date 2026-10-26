"""The page of the sidebar, driven end to end: workers and their subagents are siblings
in one list with different styles, subagents start collapsed and expand one worker at a
time, a click opens that agent's chat full screen and a message reaches that agent alone,
and Back keeps the expansion.

There is no browser here: `helpers/page_driver.js` runs the served page's own script on a
stub DOM, bridged to the live /team_ws/ws socket of the team (the page's real leg, with
its real grants). It needs node >= 22 (a global WebSocket); without it the test SKIPs."""

import json
import os
import re
import secrets
import shutil
import subprocess
import sys
import time

from helpers import team as team_mod
from helpers.verdict import Verdict

NAME = "ui/sidebar_opens_agent_full_screen"
DRIVER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "helpers", "page_driver.js")
ROOT = "kernel_state"  # where the team's agents live
ROOT_DIR = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", ".."))


def node_ok() -> bool:
    node = shutil.which("node")
    if not node:
        return False
    out = subprocess.run([node, "--version"], capture_output=True, text=True).stdout.strip()
    m = re.match(r"v(\d+)\.", out)
    return bool(m) and int(m.group(1)) >= 22


class PageError(Exception):
    """A step of the page that cannot be done (for example a row that is not drawn)."""


class Page:
    """The node driver: JSON lines in, JSON lines out."""

    def __init__(self, port: int, log_path: str) -> None:
        self.log = open(log_path, "wb")
        self.proc = subprocess.Popen(["node", DRIVER, str(port)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=self.log, text=True)

    def cmd(self, **c) -> dict:
        self.proc.stdin.write(json.dumps(c) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            raise RuntimeError("the page driver exited")
        out = json.loads(line)
        if out.get("error") or out.get("fatal"):
            raise PageError(f"{c.get('cmd')} {c.get('id') or c.get('worker') or ''}: {out.get('error') or out.get('fatal')}")
        return out

    def close(self) -> None:
        try:
            self.proc.stdin.write(json.dumps({"cmd": "quit"}) + "\n")
            self.proc.stdin.flush()
        except OSError:
            pass
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


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


CHAT_VERBS = ["history", "send", "stop", "status", "watch"]  # tree::CHAT_VERBS
PAGE_VERBS = ["sidebar", "tree", "open", "control"]          # app_control::PAGE_VERBS


def operator(team, **fields) -> None:
    r = team.send("kernel", fields)
    if r.get("status") != "completed" or "error" in (r.get("result") or {}):
        raise RuntimeError(f"cannot write {fields.get('id')}: {r}")


def serve_team_page(ctx, team, port: int, chat_agents) -> None:
    """Serve the team page from outside, the way `app_control::serve_web` does inside.

    The headless binary's `--port` serves fantastic's canvas (a browser app, with no
    dispatcher cell placed by this flow), and it does not register `team_page.tools`: only
    the in-process `TeamHandle` can serve /team_page and /team_ws. So the operator makes
    the same agents with the same grants: the web host; the page (a file agent holding
    the repo's own dispatcher.html with the page shim in its head, which is what
    `page::index_html` builds); the socket leg (`web_ws.tools`) that may call the
    dispatcher's page verbs and the chat verbs of the workers and their subagents, as
    `tree::grant_chat_to` writes them. The dispatcher, the workers and their replies are
    the binary's own."""
    web = os.path.join(ROOT_DIR, "crates", "team", "web")
    html = open(os.path.join(web, "dispatcher.html")).read()
    shim = open(os.path.join(web, "page_shim.js")).read()
    page_dir = os.path.join(ctx.run_dir, "page")
    os.makedirs(page_dir, exist_ok=True)
    with open(os.path.join(page_dir, "index.html"), "w") as f:
        f.write(html.replace("</head>", f"<script>\n{shim}</script>\n</head>", 1))
    operator(team, type="create_agent", id="web", parent_id="kernel_state", handler_module="web.tools", port=port)
    operator(team, type="create_agent", id="team_page", parent_id="web", handler_module="file_bridge.tools",
             root=page_dir, allow=["."], ingress_rule="allow_all_ingress")
    operator(team, type="create_agent", id="team_ws", parent_id="web", handler_module="web_ws.tools",
             ingress_rule="allow_all_ingress")
    for served, verb in (("team_ws", "get_routes"), ("team_page", "read_stream")):
        operator(team, type="update_agent", id=served, callable_by={"$self": "*", "$parent": "*", "web": [verb]})
    operator(team, type="update_agent", id="web",
             can_call={"$self": "*", "team_ws": ["get_routes"], "team_page": ["read_stream"]})
    for aid in chat_agents:
        operator(team, type="update_agent", id=aid,
                 callable_by={"$self": "*", "$parent": "*", "team_ws": CHAT_VERBS})
    operator(team, type="update_agent", id="team_ws",
             can_call={"$self": "*", "dispatcher": PAGE_VERBS, **{a: CHAT_VERBS for a in chat_agents}})
    operator(team, type="update_agent", id="dispatcher",
             callable_by={"$self": "*", "$parent": "*", "team_ws": PAGE_VERBS})
    r = team.send("web", {"type": "boot"})
    if r.get("status") != "completed":
        raise RuntimeError(f"cannot serve the page: {r}")


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
    key = "".join(["sk-", "ant-", secrets.token_hex(20)])
    team = team_mod.Team(ctx.binary, ctx.run_dir, {"E2E_MODEL_KEY": key})
    ctx.team = team
    team_mod.boot(team, os.path.join(ctx.dir, "team.yaml"), ctx.vars)
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
    r = team.send("kernel", {"type": "update_agent", "id": "credentials",
                             "callable_by": {"$self": "*", "lead": "*", **{a: "*" for a, _ in agents}}})
    if r.get("status") != "completed":
        raise RuntimeError(f"cannot admit the agents to the credential store: {r}")
    port = team_mod.free_port()
    serve_team_page(ctx, team, port, [a for a, _ in agents])

    status, body = team_mod.http_get(port, "/team_page/file/index.html")
    v.check("the page is served (HTTP 200, the sidebar script and the socket shim)",
            status == 200 and "function renderSidebar(" in body and "window.fantastic = {" in body, f"HTTP {status} {body[:300]}")

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

        # e. a worker opens full screen
        mark = page.cmd(cmd="frames")["total"]
        s = page.cmd(cmd="click_node", id="w1")
        fr = page.cmd(cmd="frames", since=mark)["frames"]
        opened = next((f["reply"] for f in fr if (f.get("payload") or {}).get("type") == "open"), {})
        v.check("open w1 answers view chat, target w1, fullscreen",
                opened.get("view") == "chat" and opened.get("target") == "w1" and opened.get("fullscreen") is True, str(opened))
        v.check("the pane is shown, the sidebar hidden, the title is w1 and the composer says `Message w1`",
                not s["paneHidden"] and s["sidebarHidden"] and s["title"] == "w1" and s["chat"]
                and s["chat"]["node"] == "w1" and s["chat"]["placeholder"] == "Message w1", str(s)[:400])

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
        h = holders(page, tok1, ["w1", "w2", "sub_a", "sub_b", "lead"])
        v.check("only w1's history holds the token (not w2, its subagents or the lead)",
                h == {"w1": True, "w2": False, "sub_a": False, "sub_b": False, "lead": False}, str(h))

        # g. a subagent's chat
        s = page.cmd(cmd="click_back")
        v.check("Back from w1's chat shows the sidebar with w2 still expanded and w1 collapsed",
                not s["sidebarHidden"] and [r["node"] for r in s["rows"] if "sub" in row_cls(r)] == ["sub_a", "sub_b"]
                and s["expanded"] == ["w2"], str(s["rows"]))
        mark = page.cmd(cmd="frames")["total"]
        s = page.cmd(cmd="click_node", id="sub_a")
        fr = [f for f in page.cmd(cmd="frames", since=mark)["frames"]]
        opened = next((f["reply"] for f in fr if (f.get("payload") or {}).get("type") == "open"), {})
        v.check("the subagent row opened (the expansion survived Back): view chat, target sub_a",
                opened.get("view") == "chat" and opened.get("target") == "sub_a" and opened.get("fullscreen") is True, str(opened))
        v.check("the pane targets sub_a alone: title sub_a, composer `Message sub_a`",
                not s["paneHidden"] and s["title"] == "sub_a" and s["chat"] and s["chat"]["node"] == "sub_a"
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
        h = holders(page, tok2, ["w1", "w2", "sub_a", "sub_b", "lead"])
        v.check("only sub_a's history holds the second token",
                h == {"w1": False, "w2": False, "sub_a": True, "sub_b": False, "lead": False}, str(h))

        # h. Back keeps the expansion
        mark = page.cmd(cmd="frames")["total"]
        s = page.cmd(cmd="click_back")
        asks = [f for f in page.cmd(cmd="frames", since=mark)["frames"] if (f.get("payload") or {}).get("type") == "sidebar"]
        rows = s["rows"]
        exp = {r["expander"]: r for r in rows if r["expander"]}
        v.check("Back hides the pane and shows the sidebar and Back is hidden again (level 0)",
                s["paneHidden"] and not s["sidebarHidden"] and s["backHidden"] and s["chat"] is None, str(s)[:400])
        v.check("the reload on Back asked for expanded: [w2]",
                bool(asks) and asks[-1]["payload"]["expanded"] == ["w2"], str(asks))
        v.check("w2's subagent rows are still drawn and w1 is still collapsed",
                [r["node"] for r in rows if "sub" in row_cls(r)] == ["sub_a", "sub_b"]
                and exp["w2"]["aria"] == "true" and exp["w1"]["aria"] == "false", str(rows))
        v.check("the expansion is remembered per worker (fantastic.team.subagents.w2 = 1, none for w1) "
                "and the scroll was restored", s["remembered"].get("fantastic.team.subagents.w2") == "1"
                and "fantastic.team.subagents.w1" not in s["remembered"] and s["scrolls"][-1] == 0, str(s["remembered"]))
    except PageError as e:
        v.check("every step of the page can be done", False, str(e))
    finally:
        page.close()
    team.stop()
    return v.finish()
