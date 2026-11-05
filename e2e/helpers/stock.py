"""The stock team of the older e2e tests, as fantastic records (issues #219, #221).

The team binary is fantastic's command line (`--headless`, `canvas --port N`) and has no
config file and no blueprint, so a test builds its team the way anyone does: every agent is a
`create_agent` payload sent with `fantastic_team send`, under an id the test chooses, and every
edge is written on both ends (`link`):

- the lead is `lead` (`ctx.lead`), made by `make_lead`;
- the credential store, the worker's exec and files, the connectors, the puller, the board, the
  dispatcher, the cadencer, the roles registry and the pack agents are made the same way, under
  the fixed ids the product's code and the role cards name.

Nothing here is a team config: every step is a fantastic verb a person can type.
"""

from __future__ import annotations

import os
import re
import time

from . import forge as forge_mod, run_flow, team as T
from .records import ROOT, create as _create, link, merge, update

PAGE_VERBS = ["sidebar", "tree", "open", "control"]
CHAT = T.CHAT_VERBS
DISPATCHER_KERNEL_VERBS = ["create_agent", "update_agent", "delete_agent"]
LEAD_ROLES_VERBS = ["roles", "role", "spawn", "get_count", "fleet"]
LEAD_BOARD_VERBS = ["board", "rows"]
AWAIT_VERBS = ["await_set", "await_cancel"]
REPO = "acme/widgets"
LEAD = "lead"

# the tools the lead may call, when they exist
LEAD_TOOLS = ("worker_exec", "worker_files", "queue", "git", "github")


# ---- records ---------------------------------------------------------------------------

def create(_team, **rec) -> dict:
    return _create(_team, rec)


def operator_holds(team, targets: dict) -> None:
    """The operator (the `send` leg, `mcp`) may call `targets` (agent -> verbs): both ends. A
    spawner grants only what it holds, so the operator holds what a card grants before it spawns."""
    have = team.agents()
    for t, verbs in targets.items():
        if t in have:
            merge(team, t, "callable_by", {"mcp": verbs}, have)
    merge(team, "mcp", "can_call", dict(targets))


# ---- the base: credentials, the worker's tools, the forge's connectors -----------------

def credential_refs(ctx, github: bool = True) -> dict:
    refs = {f"model/{ctx.vars['model_host']}": {"host": ctx.vars["model_host"],
                                                "env": {"api_key": "E2E_MODEL_KEY"}}}
    if github:
        refs["github/app"] = {"host": ctx.vars["forge_host"],
                              "env": {"app_id": "E2E_APP_ID", "private_key": "E2E_APP_PEM"}}
    return refs


def github_allowed() -> str:
    """The `allowed` pattern of the GitHub build: the one `patches/github-exec.patch` gives
    `worker_exec::WORKER_ALLOWED` (read from the patch, so the test follows the build)."""
    patch = os.path.join(os.path.dirname(__file__), "..", "..", "patches", "github-exec.patch")
    with open(patch) as f:
        m = re.search(r'^\+pub const WORKER_ALLOWED: &str = r#"(.*)"#;$', f.read(), re.M)
    if not m:
        raise RuntimeError("patches/github-exec.patch no longer sets WORKER_ALLOWED")
    return m.group(1)


def worker_exec_record(build: str = "general") -> dict:
    """The restricted exec of a worker (`worker_exec::worker_exec_record`), relative to the
    folder the team runs in: its HOME is `./sandbox/home` and no credential is inherited. The
    general build allows any command line; the GitHub build refuses forge calls."""
    ssh = ("ssh -F /dev/null -o IdentityAgent=none -o IdentitiesOnly=yes "
           "-o IdentityFile=/dev/null -o BatchMode=yes")
    return {"handler_module": "exec_bridge.tools", "ingress_rule": "allow_all_ingress",
            "cwd": "./work", "allowed": [github_allowed() if build == "github" else ".*"],
            "inherit_env": ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR", "TZ", "USER",
                            "LOGNAME", "SHELL"],
            "env": {"HOME": "./sandbox/home", "XDG_CONFIG_HOME": "./sandbox/home/.config",
                    "GIT_CONFIG_NOSYSTEM": "1", "GIT_TERMINAL_PROMPT": "0",
                    "GCM_INTERACTIVE": "never", "GIT_SSH_COMMAND": ssh}}


def make_tools(ctx, team, queue: bool = True) -> None:
    """The worker's exec and files (under `user`, as the product's own are), the RAM queue."""
    for d in ("work/wt", "work/tasks", "sandbox/home"):
        os.makedirs(os.path.join(ctx.work, d), exist_ok=True)
    create(team, id="worker_exec", parent_id="user", **worker_exec_record(getattr(ctx, "build", "general")))
    create(team, id="worker_files", parent_id="user", handler_module="file_bridge.tools",
           root="./work", allow=["."], ingress_rule="allow_all_ingress")
    if queue:
        create(team, id="queue", parent_id="user", handler_module="ram_store.tools")


def make_credentials(ctx, team, github: bool = True) -> None:
    create(team, id="credentials", parent_id="user", handler_module="credential_store.tools",
           backend="env", refs=credential_refs(ctx, github))


def make_forge(ctx, team, puller: bool = True, github_only: bool = False) -> None:
    """The git and github connectors against the fake forge, the queue folder and the puller."""
    run = ctx.work
    if not github_only:
        create(team, id="git", parent_id="user", handler_module="team_git.tools",
               repo_dir=os.path.join(ctx.run_dir, "clone"), workspace_root=os.path.join(run, "work", "wt"),
               remote=f"{ctx.forge.url}/{REPO}.git", default_branch="main",
               author_name="Team Bot", author_email="team-bot@example.invalid",
               directive="Workers may make a worktree and commit. Never the default branch, never a force.")
    extra = {} if github_only else {"git": "git"}
    create(team, id="github", parent_id="user", handler_module="team_github.tools",
           app={"credentials": {"store": "credentials", "ref": "github/app"}},
           repos=[REPO], api_url=ctx.forge.url, **extra,
           directive="Push a work branch and open a draft pull request.",
           callable_by={"$self": "*", "mcp": "*"})
    link(team, "github", "credentials")
    if puller:
        create(team, id="queue_files", parent_id="user", handler_module="file_bridge.tools",
               root=os.path.join(run, "work", "tasks"), allow=["."],
               ingress_rule="allow_all_ingress", callable_by={"$self": "*", "mcp": "*"})
        create(team, id="puller", parent_id="user", handler_module="team_puller.tools",
               repos=[REPO], maintainer=forge_mod.MAINTAINER, app="team-bot[bot]",
               github="github", store="queue_files")
        link(team, "puller", "github")
        link(team, "puller", "queue_files")


# ---- the lead: `create_agent` ------------------------------------------------------------

def make_lead(ctx, team, tools=None, subtree: bool = True) -> str:
    """The lead, `create_agent` under `user` with the fixed id `lead`: the scripted (or real)
    plug's model, the credential ref of the model host, the shared memory store. Its `can_call`
    names the tools that exist (`tools`, default `LEAD_TOOLS`), each edge written on both ends.
    Returns the lead's id (`ctx.lead`)."""
    have = team.agents()
    wanted = LEAD_TOOLS if tools is None else tools
    create(team, id=LEAD, parent_id="user", handler_module="anthropic_backend.tools",
           model=ctx.vars["model_name"], endpoint=ctx.vars["model_endpoint"],
           credentials={"store": "credentials", "ref": f"model/{ctx.vars['model_host']}"},
           memory_store_id="memory", context_window=100000, max_tokens=4000,
           can_call={"$self": "*", **({"$subtree": "*"} if subtree else {})},
           callable_by={"$self": "*", "$parent": "*"})
    for callee in [c for c in ("memory", "credentials", *wanted) if c in have]:
        link(team, LEAD, callee)
    ctx.lead = LEAD
    return LEAD


# ---- the machinery: roles, board, dispatcher, cadencer, packs, the page -----------------

def make_registry(ctx, team, tiers: dict, limits: dict | None = None, cards: bool = False,
                  role_dirs=("roles",)) -> None:
    """The roles registry (`agentic_resources`) with the given `tiers` and limits.
    Without `cards` it has no `role_dirs`: the dispatcher makes workers from its tiers only."""
    lead = ctx.lead
    create(team, id="roles", parent_id=ROOT, handler_module="agentic_resources.tools",
           role_dirs=list(role_dirs) if cards else [], tiers=tiers,
           can_call={"$self": "*"}, callable_by={"$self": "*", "$parent": "*", "mcp": "*"})
    link(team, "roles", "memory")
    for role, n in [(None, (limits or {}).get("total"))] + list(((limits or {}).get("per_role") or {}).items()):
        if n is None:
            continue
        ask = {"type": "set_limit", "max": n}
        if role:
            ask["role"] = role
        r = team.send("roles", ask)
        if r.get("status") != "completed" or r["result"].get("error"):
            raise RuntimeError(f"set_limit {ask}: {r}")
    link(team, lead, "roles", LEAD_ROLES_VERBS)


def make_roles(ctx, team, tiers: dict, limits: dict | None = None) -> None:
    """Roles mode: the registry with the repo's role cards, the task registry under the lead,
    and the lead's grants on them and on the root (a spawner grants only what it holds)."""
    lead = ctx.lead
    make_registry(ctx, team, tiers, limits, cards=True)
    create(team, id="tasks", parent_id=lead, handler_module="task_registry.tools",
           task_store_id="memory", can_call={"$self": "*"},
           callable_by={"$self": "*", "$parent": "*", "mcp": "*"})
    link(team, "tasks", "memory")
    link(team, lead, ROOT, AWAIT_VERBS + ["create_agent", "delete_agent"])
    merge(team, lead, "can_call", {"canvas_page": ["new_canvas"]})


def lead_tier(ctx, **extra) -> dict:
    """A tier the scripted model answers: the lead's own backend fields."""
    t = {"handler_module": "anthropic_backend.tools", "model": "scripted-model",
         "endpoint": ctx.vars["model_endpoint"],
         "credentials": {"store": "credentials", "ref": f"model/{ctx.vars['model_host']}"},
         "memory_store_id": "memory", "context_window": 100000, "max_tokens": 4000}
    t.update(extra)
    return t


def inert_tier(ctx) -> dict:
    return {"handler_module": "ram_store.tools", "model": "inert",
            "endpoint": ctx.vars["model_endpoint"], "memory_store_id": "memory"}


def make_board(ctx, team, groups: dict, store: str = "queue_files", roles: bool = False) -> None:
    team_groups = {n: {"driver": g.get("driver", ""), "members": list(g.get("members", []))}
                   for n, g in groups.items()}
    create(team, id="board", parent_id=ROOT, handler_module="team_board.tools", repos=[REPO],
           github="github", store=store, team_groups=team_groups,
           can_call={"$self": "*", "github": "*", store: "*"},
           callable_by={"$self": "*", "$parent": "*", "dispatcher": "*", "pack": ["board"],
                        "cadencer": ["rows"], "mcp": "*"})
    link(team, "board", "github")
    link(team, "board", store)
    link(team, ctx.lead, "board", LEAD_BOARD_VERBS)
    if "pack" in team.agents():  # `eval {from_shifts}` reads the board's tasks for the measured record
        link(team, "pack", "board", ["board"])
    if roles:
        update(team, "board", roles="roles")
        link(team, "board", "roles", ["fleet", "get_count"])
        link(team, "board", "tasks", ["list"])
        link(team, "board", ROOT, ["reflect"])


def make_dispatcher(ctx, team, groups: dict, board: bool = True, roles: bool = False,
                    registry: bool = False) -> None:
    """The dispatcher with its worker view, and the cadencer. `groups`: name -> {driver?,
    members, cadence?, stall_after?, quiet?, enabled?}; `driver` may be `"$lead"`."""
    lead = ctx.lead

    def who(x):
        return lead if x == "$lead" else x

    gr = {n: {**g, **({"driver": who(g["driver"])} if g.get("driver") else {})}
          for n, g in groups.items()}
    settings = {n: {k: g[k] for k in ("cadence", "stall_after", "quiet", "enabled") if k in g}
                for n, g in gr.items()}
    settings = {n: s for n, s in settings.items() if s}
    d = dict(id="dispatcher", parent_id=ROOT, handler_module="team_dispatcher.tools",
             groups={n: list(g.get("members", [])) for n, g in gr.items()}, team=lead,
             can_call={"$self": "*"}, callable_by={"$self": "*", "$parent": "*", "mcp": "*"})
    if board:
        d["board"] = "board"
    if roles or registry:
        d["roles"] = "roles"
    create(team, **d)
    create(team, id="worker_view", parent_id="dispatcher", handler_module="team_worker_view.tools",
           callable_by={"$self": "*", "$parent": "*", "mcp": "*"})
    link(team, "dispatcher", "worker_view")
    link(team, "dispatcher", ROOT, DISPATCHER_KERNEL_VERBS)
    c = dict(id="cadencer", parent_id=ROOT, handler_module="team_cadencer.tools",
             group_settings=settings, can_call={"$self": "*"},
             callable_by={"$self": "*", "$parent": "*", "mcp": "*"})
    if board:
        c["board"] = "board"
    if roles:
        c["roles"] = "roles"
    create(team, **c)
    link(team, "dispatcher", "cadencer", ["status"])
    if board:
        link(team, "cadencer", "board", ["rows"])
    link(team, "cadencer", lead, ["send"])
    for g in gr.values():
        if g.get("driver"):
            link(team, "cadencer", g["driver"], ["send"])
    if roles or registry:
        link(team, "dispatcher", "roles", ["get_count"])
    if roles:
        link(team, "cadencer", "roles", ["fleet", "get_count"])
        link(team, "cadencer", "tasks", ["list"])
        link(team, "cadencer", ROOT, ["reflect"])
    if "github" in team.agents():
        link(team, "dispatcher", "github", ["reflect"])
    if board:
        merge(team, "board", "callable_by", {"dispatcher": "*"})


def make_packs(ctx, team) -> None:
    """The pack agents, the `files` peer they read through, and the `user` verb they build with."""
    lead = ctx.lead
    create(team, id="pack", parent_id="user", handler_module="team_pack.tools",
           can_call={"$self": "*"},
           callable_by={"$self": "*", "$parent": "*", "dispatcher": "*", "mcp": "*"})
    create(team, id="pack_listing", parent_id="user", handler_module="team_packs.tools",
           index_path="packs/index.json", templates_dir="templates", store="files",
           can_call={"$self": "*"},
           callable_by={"$self": "*", "$parent": "*", "dispatcher": "*", "mcp": "*"})
    link(team, "pack", "files")
    link(team, "pack_listing", "files")
    link(team, "pack", "user", ["import_subtree", "send"])
    link(team, "pack", ROOT, ["list_agents"])
    link(team, "pack", lead)
    # the operator's leg writes and reads pack folders through `files`
    merge(team, "files", "callable_by", {"mcp": "*"})


def serve_team_page(team, port: int, workers=()) -> None:
    """Serve the team page, the way the app does (`app_control::prepare_web`): a `web` host
    on `port`, the page agent and the socket leg under it, their edges, the leg's grants on the
    dispatcher and the other tools the page sends to, and the chat verbs of `workers`; then `boot`
    on the host, which mounts its children and listens. Needs a dispatcher. A team started with
    fantastic's `canvas` word has a second socket leg on the same route, so start it `--headless`."""
    doc = ("args: path:str (req), offset:int?, length:int?. The team page (index.html, page.css, "
           "shim.js, page.js, card.js) as a BINARY reply: the bytes as the body, "
           "{path, offset, next_offset, eof, size} as the head.")
    create(team, id="web", parent_id=ROOT, handler_module="web.tools", port=port)
    create(team, id="team_page", parent_id="web", handler_module="team_page.tools",
           verbs={"read_stream": doc})
    create(team, id="team_ws", parent_id="web", handler_module="web_ws.tools",
           ingress_rule="allow_all_ingress")
    link(team, "web", "team_ws", ["get_routes"])
    link(team, "web", "team_page", ["read_stream"])
    link(team, "team_ws", "dispatcher", PAGE_VERBS)
    have = team.agents()
    for target, verbs in (("cadencer", ["status", "set"]), ("user", ["inquiries", "answer_inquiry"]),
                          ("pack", ["install", "eval"]),
                          ("pack_listing", ["browser", "show", "fetch"])):
        if target in have:
            link(team, "team_ws", target, verbs)
    grant_chat(team, "team_ws", list(workers))
    r = team.send("web", {"type": "boot"})  # it mounts its children, then listens
    if r.get("status") != "completed" or (r.get("result") or {}).get("error"):
        raise RuntimeError(f"cannot boot the web host: {r}")
    team.port = port


def serve_webui(team, port: int, wait_s: float = 30.0) -> None:
    """Serve the team page from the binary: stop the headless team and start it again with the
    `webui --port N` word (#224). The binary makes the `web` host, the page and the socket leg and
    grants the leg what the page sends to, for the agents that exist when it starts (the dispatcher,
    the cadencer, the user, the pack agents, the chat verbs of a group's workers). Make those agents
    first; the folder keeps them across the restart."""
    team.stop()
    sock = os.path.join(team.dir, T.SOCK)
    if os.path.exists(sock):
        os.remove(sock)
    team.start(extra_args=["webui", "--port", port])
    end = time.time() + wait_s
    status = 0
    while time.time() < end and status != 200:
        status, _ = T.http_get(port, "/team_page/file/index.html")
        time.sleep(0.2)
    if status != 200:
        raise RuntimeError(f"the team page is not served on {port}: HTTP {status}\n{team.log()[-500:]}")


def grant_chat(team, leg: str, workers, verbs=CHAT) -> None:
    for w in workers:
        link(team, leg, w, list(verbs))


# ---- the whole stock team ---------------------------------------------------------------

def start_args(port: int | None, extra_args=None) -> list[str]:
    if extra_args is not None:
        return list(extra_args)
    return ["canvas", "--port", str(port)] if port else list(T.HEADLESS)


def boot(ctx, env: dict | None = None, *, port: int | None = None, forge: bool = True,
         puller: bool = True, git: bool = True, packs: bool = False, groups: dict | None = None,
         board: bool = False, board_store: str = "queue_files", roles_tiers: dict | None = None,
         registry_tiers: dict | None = None, limits: dict | None = None, extra_args=None, lead_tools=None,
         credentials: bool = True, queue: bool = True, github_cred: bool | None = None,
         sub: str | None = None, dispatcher: bool | None = None):
    """Start the team binary as fantastic's CLI in the run folder and make the stock records with
    `send` + `create_agent`, the lead among them (`make_lead`; `lead_tools` names the tools it
    may call, default `LEAD_TOOLS`). Sets `ctx.team`, `ctx.lead`, `ctx.work`; returns the Team."""
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    env = dict(env or {"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                       "E2E_APP_PEM": keys["pem"]})
    if roles_tiers is not None or registry_tiers is not None:
        env["FANTASTIC_AGENTIC_RESOURCES"] = "1"
    ctx.work = os.path.join(ctx.run_dir, sub) if sub else ctx.run_dir
    os.makedirs(ctx.work, exist_ok=True)
    ctx.team = team = T.Team(ctx.binary, ctx.work, env)
    team.start(extra_args=start_args(port, extra_args))
    if credentials:
        make_credentials(ctx, team, github=forge if github_cred is None else github_cred)
    make_tools(ctx, team, queue=queue)
    if forge:
        make_forge(ctx, team, puller=puller, github_only=not git)
    make_lead(ctx, team, lead_tools)
    if groups is not None:  # `$lead` is the lead's id
        groups = {n: {**g, **({"driver": ctx.lead} if g.get("driver") == "$lead" else {})}
                  for n, g in groups.items()}
    ctx.groups = groups
    if packs:
        make_packs(ctx, team)
    if roles_tiers is not None:
        make_roles(ctx, team, roles_tiers, limits)
    elif registry_tiers is not None:
        make_registry(ctx, team, registry_tiers, limits)
    if board:
        make_board(ctx, team, groups or {}, board_store, roles=roles_tiers is not None)
    if (groups is not None) if dispatcher is None else dispatcher:
        make_dispatcher(ctx, team, groups or {}, board=board, roles=roles_tiers is not None,
                        registry=registry_tiers is not None)
    return team
