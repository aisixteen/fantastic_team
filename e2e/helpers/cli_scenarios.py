"""The six scenarios of the cli group, defined once (issues #219, #237, #242).

A `cli/` test plays one on the scripted model; its `gpu_cli/` twin plays the same on a real model
(the vllm plug, no script). Both build the same team through the same `create_agent` calls, under
the same fixed ids and the same role cards, and run the same checks on the result. What differs is
only what the model path is: the scripted test also checks that every scripted turn was served and
that each lane asked the model (checks about a script); the gpu test hands the lead its task in
plain words and waits for the lead to answer `done`, and asserts nothing about the model's own path.

Each scenario is a `*_start(ctx, v)` that boots and builds the team (and checks it), and a
`*_end(ctx, v, ...)` that checks the outcome. Both return what the other half needs.
"""

from __future__ import annotations

import os
import subprocess
import time

from . import cli, stock, team as team_mod, web
from .verdict import Skip

# The trees the precreated scenarios make: id -> parent, parents first.
ONE_TWO = {"lead": "user", "w1": "lead", "w2": "lead"}
TREE = {"lead": "user", "s1": "lead", "w1": "s1", "s2": "lead", "w2": "s2"}


# ---- single_lead -----------------------------------------------------------------------

def single_by_hand(ctx, v, sub: str = "create_agent_run"):
    """A: `--headless`, then the lead made with `create_agent` under the id `lead`.
    Returns (lead id, the records before the task)."""
    cli.start(ctx, sub=sub)
    cli.add_base(ctx)
    base = set(cli.ids(ctx.team))
    lead = cli.make_lead(ctx, {cli.EXEC: "*"})
    made = set(cli.ids(ctx.team)) - base
    v.check("A: create_agent made the lead alone, under the id `lead`", made == {"lead"} and lead == "lead", str(made))
    before = cli.ids(ctx.team)
    handler = ctx.model.ai_record()["handler_module"]
    v.check(f"A: the lead is a {handler} record", before.get(lead) == handler, str(before))
    return lead, before


def single_by_recipe(ctx, v, sub: str = "recipe_run"):
    """B: fantastic's recipe makes the root AI `ai`, then `send` of `create_agent` /
    `update_agent` gives it the key, the exec grant and whatever the recipe leaves to the plug.
    Returns ("ai", the records before the task)."""
    cli.start(ctx, ctx.model.recipe_args(), sub=sub)
    cli.create(ctx.team, {"id": "credentials", "parent_id": "user",
                          "handler_module": "credential_store.tools", "backend": "env",
                          "refs": {f"model/{ctx.model.host}": {"host": ctx.model.host,
                                                                "env": {"api_key": "E2E_MODEL_KEY"}}}})
    cli.create(ctx.team, dict(cli.tool_record(ctx.repo, ctx.work), id=cli.EXEC, parent_id="user"))
    handler = ctx.model.ai_record()["handler_module"]
    ai = ctx.team.agents().get("ai")
    v.check(f"B: the recipe made the root AI `ai` on {handler}",
            ai is not None and ai.get("handler_module") == handler, str(ai)[:300])
    grants = dict(ai.get("can_call") or {})
    grants[cli.EXEC] = ["*"]
    update = {"type": "update_agent", "id": "ai", "can_call": grants, **ctx.model.recipe_fields()}
    if ctx.model.keyed:
        update["credentials"] = cli.model_credentials(ctx)
    r = ctx.team.send("kernel", update)
    v.check("B: send update_agent gave it the key and the exec grant", r.get("status") == "completed", str(r)[:300])
    return "ai", cli.ids(ctx.team)


def single_end(ctx, v, label: str, before: dict) -> bool:
    """The outcome of one single_lead run: solved, and nothing added to the records."""
    after = cli.ids(ctx.team)
    ok, why = cli.solved(ctx.repo)
    v.check(f"{label}: solved (the test passes on HEAD, the fix is committed)", ok, why)
    v.check(f"{label}: the records are the ones it started with", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    return ok


# ---- lead_spawns_two -------------------------------------------------------------------

def spawn_two_start(ctx, v, credentials: bool = False) -> dict:
    """A lead with the registry and the `worker` card (room for 2). Returns the tree before.
    `credentials`: the workers' tier carries the model's key ref (a plug that needs one)."""
    cli.start(ctx, roles=True)
    cli.add_base(ctx)
    base = set(cli.ids(ctx.team))
    cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["delete_agent"]}, subtree=True)
    cli.make_registry(ctx, total=2, roles=cli.roles_dir("lead_spawns_two"),
                      credentials=cli.model_credentials(ctx) if credentials else None)
    new = set(cli.ids(ctx.team)) - base
    v.check("create_agent made the lead and its roles registry, under fixed ids", new == {"lead", "roles"}, str(new))
    count = ctx.team.send("roles", {"type": "get_count"})["result"]
    v.check("the registry is on and counts nothing running", count == {"running": 0, "max": 2}, str(count))
    return cli.tree(ctx.team)


def spawn_two_end(ctx, v, before: dict) -> None:
    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    spawns = [c for c in team_mod.log_calls(ctx.team.log()) if c == ("roles", "spawn")]
    v.check("the lead spawned 2 workers", len(spawns) == 2, str(spawns))
    after = cli.tree(ctx.team)
    v.check("the records are the ones it started with (both workers deleted)", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    count = ctx.team.send("roles", {"type": "get_count"})["result"]
    v.check("the registry counts nothing running", count["running"] == 0, str(count))


# ---- precreated_1_2 and precreated_tree ------------------------------------------------

def precreate(ctx, v, expect: dict, all_keyed: bool = False) -> dict:
    """Make the agents of `expect` (id -> parent, parents first) with `create_agent`, each
    able to call its children, a leaf the exec agent. Returns the tree before the task. Only the
    lead reads the model's key (the scripted server wants none); `all_keyed`: every agent does,
    for a server behind a door that asks every request for it."""
    cli.start(ctx)
    cli.add_base(ctx)
    base = set(cli.ids(ctx.team))
    for id, parent in expect.items():
        kids = {c: "*" for c, p in expect.items() if p == id}
        cli.make_agent(ctx, id, parent, kids or {cli.EXEC: "*"}, key=(id == cli.LEAD or all_keyed))
    before = cli.tree(ctx.team)
    v.check("the tree is the one made and the base, nothing else",
            set(before) == base | set(expect), str(sorted(set(before) - base)))
    v.check("each agent sits under the parent it was made under",
            all(before[i][1] == p for i, p in expect.items()),
            str({i: before[i][1] for i in expect}))
    return before


def precreated_end(ctx, v, before: dict) -> None:
    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    after = cli.tree(ctx.team)
    v.check("the same agents under the same parents, nothing extra", after == before,
            f"before {sorted(before)} after {sorted(after)}")


# ---- leads_spawn_leads -----------------------------------------------------------------

def spawn_leads_start(ctx, v, credentials: bool = False) -> dict:
    """A lead with the registry (room for 4) and the `sublead` and `worker` cards."""
    cli.start(ctx, roles=True)
    cli.add_base(ctx)
    base = set(cli.ids(ctx.team))
    cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["delete_agent"]}, subtree=True)
    cli.make_registry(ctx, total=4, roles=cli.roles_dir("leads_spawn_leads"),
                      credentials=cli.model_credentials(ctx) if credentials else None)
    new = set(cli.ids(ctx.team)) - base
    v.check("create_agent made the lead and its roles registry, under fixed ids", new == {"lead", "roles"}, str(new))
    return cli.tree(ctx.team)


def spawn_leads_end(ctx, v, before: dict) -> None:
    team = ctx.team
    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    spawns = [c for c in team_mod.log_calls(team.log()) if c == ("roles", "spawn")]
    v.check("4 spawns: 2 sub-leads by the lead, 1 worker by each", len(spawns) == 4, str(spawns))
    after = cli.tree(team)
    left = [i for i, a in team.agents().items() if a.get("spawned_by")]
    v.check("no spawned agent is left", not left, str(left))
    orphans = [i for i, (_, p) in after.items() if p and p not in after]
    v.check("no orphans: every agent's parent exists", not orphans, str(orphans))
    v.check("the records are the ones it started with", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    count = team.send("roles", {"type": "get_count"})["result"]
    v.check("the registry counts nothing running", count["running"] == 0, str(count))


# ---- headless_and_canvas ---------------------------------------------------------------

CANVAS = {"id": "sketch", "parent_id": "lead", "handler_module": "canvas_page.tools"}


def canvas_start(ctx, v):
    """`--headless`: no port, no screen; a lead that may `create_agent` on the root. Returns the team.
    Raises Skip when this host cannot list listening sockets."""
    if cli.listeners(os.getpid()) is None:
        raise Skip("neither lsof nor /proc/net/tcp is available to list listening sockets")
    team = cli.start(ctx)  # stdin is /dev/null and there is no tty: the UI could not open
    cli.add_base(ctx)
    base = set(cli.ids(team))
    cli.make_lead(ctx, {cli.EXEC: "*", "kernel_state": ["create_agent"]})
    new = set(cli.ids(team)) - base
    v.check("the lead was made with create_agent", new == {"lead"}, str(new))
    v.check("the team runs headless", team.proc.poll() is None)
    agents = team.agents()
    v.check("the canvas is registered but not started: no web host, no canvas page",
            not {"web", "canvas_page", "canvas_ws"} & set(agents), str(sorted(agents)))
    return team


def wait_page(port, path: str, wait_s: float = 30.0) -> int:
    end = time.time() + wait_s
    status = 0
    while time.time() < end and status != 200:
        status, _ = team_mod.http_get(port, path)
        time.sleep(0.3)
    return status


def canvas_end(ctx, v) -> None:
    """After the lead's turn: its canvas, still headless; then `canvas --port N`, `webui --port N`,
    the two refused together, and `--headless` again with no port."""
    team = ctx.team
    agents = team.agents()
    sk = agents.get(CANVAS["id"], {})
    v.check("the lead's create_agent made a canvas with no `canvas` word",
            sk.get("handler_module") == CANVAS["handler_module"] and sk.get("parent_id") == CANVAS["parent_id"],
            str(sk)[:300])
    v.check("still no web host", "web" not in agents, str(sorted(agents)))
    ports = cli.listeners(team.proc.pid)
    v.check("--headless binds no TCP port", ports == set(), str(ports))
    log = team.log()
    v.check("it draws no terminal UI", not any(s in log for s in cli.ALT_SCREEN))
    v.check("it serves no canvas page", "the canvas: http" not in log)
    code = team.stop()
    v.check("it stops cleanly on SIGTERM", code in (0, -15), str(code))

    # canvas --port N, in the same folder: the same records, and now a web host
    port = team_mod.free_port()
    sock = os.path.join(ctx.work, team_mod.SOCK)
    if os.path.exists(sock):  # the stopped team's socket file must not pass for the next start
        os.remove(sock)
    team.start(extra_args=["canvas", "--port", port])
    status = wait_page(port, "/canvas_page/file/index.html")
    v.check("canvas --port N serves the canvas page", status == 200, str(status))
    ports = cli.listeners(team.proc.pid)
    v.check("the listener check sees port N", str(port) in ports, f"{ports}, want {port}")
    v.check("the web host and the canvas page are there now",
            {"web", "canvas_page"} <= set(team.agents()), str(sorted(team.agents())))
    v.check("the lead's canvas and the lead are still there", {"lead", "sketch"} <= set(team.agents()))
    team.stop()

    # webui --port N, in the same folder: the team page and its socket, from the binary. A folder
    # fantastic's own `canvas` seeded has none of the team's records (the team's first run seeds them,
    # #261, and `webui` makes none): they are made here, as a person with a team of their own would.
    wport = team_mod.free_port()
    if os.path.exists(sock):
        os.remove(sock)
    team.start()
    stock.make_web(team)
    team.stop()
    if os.path.exists(sock):
        os.remove(sock)
    team.start(extra_args=["webui", "--port", wport])
    page = "/team_page/file/index.html"
    status = wait_page(wport, page)
    v.check("webui --port N serves the team page", status == 200, str(status))
    ports = cli.listeners(team.proc.pid)
    v.check("the listener check sees the webui port", str(wport) in ports, f"{ports}, want {wport}")
    v.check("the page agent and its socket leg are there",
            {"team_page", "team_ws"} <= set(team.agents()), str(sorted(team.agents())))
    try:
        web.WsClient(int(wport)).sock.close()
        socket_ok = True
    except OSError as e:
        socket_ok = False
        print(f"socket: {e}")
    v.check("the team page's socket (/team_ws/ws) accepts a websocket", socket_ok)
    v.check("it announces the page", f"the team page: http://127.0.0.1:{wport}{page}" in team.log(),
            team.log()[-300:])
    v.check("the lead's canvas and the lead are still there", {"lead", "sketch"} <= set(team.agents()))
    team.stop()

    # webui and canvas together: refused with a message, no process left, no port
    if os.path.exists(sock):
        os.remove(sock)
    both = subprocess.run(team.argv(["webui", "canvas", "--port", team_mod.free_port()]), env=team.env,
                          cwd=team.dir, capture_output=True, text=True, timeout=60, stdin=subprocess.DEVNULL)
    v.check("webui with canvas is refused (exit 2) and says why",
            both.returncode == 2 and "`webui` and `canvas` cannot run together" in both.stderr,
            f"{both.returncode} {both.stderr[-300:]}")

    # --headless over the folder webui served: still no port
    team.start()
    time.sleep(2)
    ports = cli.listeners(team.proc.pid)
    v.check("--headless opens no port in a folder webui has run in", ports == set(), str(ports))
    v.check("the old webui port refuses connections", web.refuses(int(wport)))
    team.stop()
