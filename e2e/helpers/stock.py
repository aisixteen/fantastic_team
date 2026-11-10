"""The stock team of the older e2e tests, as fantastic records (issues #219, #221).

The team binary is fantastic's command line (`--headless`, `webui --port N`) and has no
config file and no blueprint, so a test builds its team the way anyone does: every agent is a
`create_agent` payload sent with `fantastic_team send`, under an id the test chooses, and every
edge is written on both ends (`link`):

- the lead is `lead` (`ctx.lead`), made by `make_lead`;
- the credential store, the worker's exec and files, the connectors, the puller, the board, the
  dispatcher, the cadencer and the roles registry are made the same way, under
  the fixed ids the product's code and the role cards name.

Nothing here is a team config: every step is a fantastic verb a person can type.
"""

from __future__ import annotations

import os
import time

from . import cli, forge as forge_mod, run_flow, team as T
from .records import ROOT, create as _create, link, merge, update

PAGE_VERBS = ["sidebar", "tree", "open", "control"]
CHAT = T.CHAT_VERBS
DISPATCHER_KERNEL_VERBS = ["create_agent", "update_agent", "delete_agent"]
LEAD_ROLES_VERBS = ["roles", "role", "spawn", "get_count", "fleet"]
LEAD_BOARD_VERBS = ["board"]
AWAIT_VERBS = ["await_set", "await_cancel"]
REPO = "acme/widgets"
LEAD = "lead"

# the tools the lead may call, when they exist
LEAD_TOOLS = ("worker_exec", "worker_files", "queue", "git", "github")


# ---- records ---------------------------------------------------------------------------

def create(_team, **rec) -> dict:
    return _create(_team, rec)


# ---- the base: credentials, the worker's tools, the forge's connectors -----------------

def credential_refs(ctx, github: bool = True, github_mode: str = "app") -> dict:
    refs = {f"model/{ctx.vars['model_host']}": {"host": ctx.vars["model_host"],
                                                "env": {"api_key": "E2E_MODEL_KEY"}}}
    if github:
        env = ({"token": "E2E_GH_TOKEN"} if github_mode == "personal"
               else {"app_id": "E2E_APP_ID", "private_key": "E2E_APP_PEM"})
        refs["github/app"] = {"host": ctx.vars["forge_host"], "env": env}
    return refs


def worker_exec_record() -> dict:
    """The restricted exec of a worker (`worker_exec::worker_exec_record`), relative to the
    folder the team runs in: its HOME is `./sandbox/home` and no credential is inherited. It
    allows any command line."""
    ssh = ("ssh -F /dev/null -o IdentityAgent=none -o IdentitiesOnly=yes "
           "-o IdentityFile=/dev/null -o BatchMode=yes")
    return {"handler_module": "exec_bridge.tools", "ingress_rule": "allow_all_ingress",
            "cwd": "./work", "allowed": [".*"],
            "inherit_env": ["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR", "TZ", "USER",
                            "LOGNAME", "SHELL"],
            "env": {"HOME": "./sandbox/home", "XDG_CONFIG_HOME": "./sandbox/home/.config",
                    "GIT_CONFIG_NOSYSTEM": "1", "GIT_TERMINAL_PROMPT": "0",
                    "GCM_INTERACTIVE": "never", "GIT_SSH_COMMAND": ssh}}


def make_tools(ctx, team, queue: bool = True) -> None:
    """The worker's exec and files (under `user`, as the product's own are), the RAM queue."""
    for d in ("work/wt", "work/tasks", "sandbox/home"):
        os.makedirs(os.path.join(ctx.work, d), exist_ok=True)
    create(team, id="worker_exec", parent_id="user", **worker_exec_record())
    create(team, id="worker_files", parent_id="user", handler_module="file_bridge.tools",
           root="./work", allow=["."], ingress_rule="allow_all_ingress")
    if queue:
        create(team, id="queue", parent_id="user", handler_module="ram_store.tools")


def make_credentials(ctx, team, github: bool = True, github_mode: str = "app") -> None:
    create(team, id="credentials", parent_id="user", handler_module="credential_store.tools",
           backend="env", refs=credential_refs(ctx, github, github_mode))


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


# ---- the machinery: roles, board, dispatcher, cadencer, the page -----------------

REPO_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def lay_role_bundles(work: str, src: str | None = None) -> list[str]:
    """Copy the repo's role bundles (`crates/team/bundles/<role>/bundle.yaml`), or those of `src`,
    into `<work>/.fantastic/bundles/` before the first start: the kernel reads them at boot, as a
    first run writes them. Returns the names."""
    import shutil
    src = src or os.path.join(REPO_DIR, "crates", "team", "bundles")
    names = sorted(os.listdir(src))
    for name in names:
        shutil.copytree(os.path.join(src, name), os.path.join(work, ".fantastic", "bundles", name),
                        dirs_exist_ok=True)
    return names


def load_role_bundles(team, src: str | None = None) -> list[str]:
    """[`lay_role_bundles`] into a RUNNING team's folder, then `reload_bundle` for each."""
    names = lay_role_bundles(team.dir, src)
    for name in names:
        r = team.send("kernel", {"type": "reload_bundle", "name": name})
        if (r.get("result") or {}).get("reloaded") != name:
            raise RuntimeError(f"bundle {name}: {r}")
    return names


def make_registry(ctx, team, tiers: dict, limits: dict | None = None, cards: bool = False) -> None:
    """The roles registry (`agentic_resources`) with its limits; `tiers` are the dispatcher's
    worker templates (`add_worker`, a field only the dispatcher reads). With `cards` the repo's
    role bundles are loaded: the roles a spawn makes, on its spawner's backend."""
    lead = ctx.lead
    if cards:
        load_role_bundles(team)
    create(team, id="roles", parent_id=ROOT, handler_module="agentic_resources.tools",
           tiers=tiers,
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
    """Roles mode: the registry with the repo's role bundles, the task registry under the lead,
    and the lead's grants on them and on the root (a spawner grants only what it holds)."""
    lead = ctx.lead
    make_registry(ctx, team, tiers, limits, cards=True)
    create(team, id="tasks", parent_id=lead, handler_module="task_registry.tools",
           task_store_id="memory", can_call={"$self": "*"},
           callable_by={"$self": "*", "$parent": "*", "mcp": "*"})
    link(team, "tasks", "memory")
    link(team, lead, ROOT, AWAIT_VERBS + ["create_agent", "delete_agent"])
    merge(team, lead, "can_call", {"canvas_page": ["new_canvas"]})


def spawn_project_lead(ctx, team) -> dict:
    """The registry's spawned tree is rooted at the `project_lead` (what the cadencer digests, in roles
    mode): the team gets its provider (the scripted model, under the lead) and the operator spawns
    one naming it. A child takes its memory store from its spawner and the operator has none, so the
    spawned lead gets the team's store here, before its first message (its workers copy it).
    Returns the spawn's result (`id`, ...)."""
    cli.add_provider(ctx, ctx.lead)
    made = team.send("roles", {"type": "spawn", "role": "project_lead", "provider": cli.PROVIDER,
                               "model": ctx.vars["model_name"]})["result"]
    if made.get("id"):
        update(team, made["id"], memory_store_id="memory")
        link(team, made["id"], "memory")
    return made


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


def make_board(ctx, team, store: str = "queue_files", roles: bool = False) -> None:
    create(team, id="board", parent_id=ROOT, handler_module="team_board.tools", repos=[REPO],
           github="github", store=store,
           can_call={"$self": "*", "github": "*", store: "*"},
           callable_by={"$self": "*", "$parent": "*", "dispatcher": "*",
                        "mcp": "*"})
    link(team, "board", "github")
    link(team, "board", store)
    link(team, ctx.lead, "board", LEAD_BOARD_VERBS)
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
    if roles:
        c["roles"] = "roles"
    create(team, **c)
    link(team, "dispatcher", "cadencer", ["status"])
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
    # a folder `webui` seeded has the page's leg: the blueprint holds its end of each edge, the
    # agents made here hold the other
    if "team_ws" in team.agents():
        link(team, "team_ws", "dispatcher", PAGE_VERBS)
        link(team, "team_ws", "cadencer", ["status", "set"])


READ_STREAM_DOC = ("args: path:str (req), offset:int?, length:int?. The team page (index.html, page.css, "
                   "shim.js, page.js, card.js) as a BINARY reply: the bytes as the body, "
                   "{path, offset, next_offset, eof, size} as the head.")


def make_web(team, workers=()) -> None:
    """The team page's records for a folder the script built (a folder the team's first run seeded
    already has them, from `crates/team/blueprint.yaml`): a `web` host with no port, the page agent
    and the socket leg under it, their edges, the leg's grants on the dispatcher and the other tools
    the page sends to, and the chat verbs of `workers`. Needs a dispatcher. The port is the run's:
    `webui --port N` puts it on the host."""
    have = team.agents()
    if "web" not in have:
        create(team, id="web", parent_id=ROOT, handler_module="web.tools")
    create(team, id="team_page", parent_id="web", handler_module="team_page.tools",
           verbs={"read_stream": READ_STREAM_DOC})
    create(team, id="team_ws", parent_id="web", handler_module="web_ws.tools",
           ingress_rule="allow_all_ingress")
    link(team, "web", "team_ws", ["get_routes"])
    link(team, "web", "team_page", ["read_stream"])
    for target, verbs in (("dispatcher", PAGE_VERBS), ("cadencer", ["status", "set"]),
                          ("user", ["inquiries", "answer_inquiry"])):
        if target in have:
            link(team, "team_ws", target, verbs)
    grant_chat(team, "team_ws", list(workers))


def serve_team_page(team, port: int, workers=()) -> None:
    """Serve the team page from a team started `--headless`: [`make_web`] with the port, then `boot`
    on the host, which mounts its children and listens. A team started with fantastic's `canvas` word
    has a second socket leg on the same route, so start it `--headless`."""
    make_web(team, workers)
    update(team, "web", port=port)
    r = team.send("web", {"type": "boot"})  # it mounts its children, then listens
    if r.get("status") != "completed" or (r.get("result") or {}).get("error"):
        raise RuntimeError(f"cannot boot the web host: {r}")
    team.port = port


def serve_webui(team, port: int, wait_s: float = 30.0) -> None:
    """Serve the team page from the binary: make the page's records ([`make_web`]; a folder that
    `webui` seeded has them), stop the headless team and start it again with the `webui --port N`
    word (#224), which puts the port on the host. Make the agents the page reaches first (the
    dispatcher, the cadencer, the user); the folder keeps them across the restart."""
    if "web" not in team.agents():
        make_web(team)
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

def boot(ctx, env: dict | None = None, *, forge: bool = True,
         puller: bool = True, git: bool = True, groups: dict | None = None,
         board: bool = False, board_store: str = "queue_files", roles_tiers: dict | None = None,
         registry_tiers: dict | None = None, limits: dict | None = None, extra_args=None, lead_tools=None,
         credentials: bool = True, queue: bool = True, github_cred: bool | None = None,
         sub: str | None = None, dispatcher: bool | None = None, seeded: bool = False,
         cards: bool = False):
    """Start the team binary as fantastic's CLI in the run folder. A test whose `test.yaml` names a
    `fleet:` has its agents as record files, laid in `.fantastic/` before the first start
    (`cli.lay_test_fleet`; the folders `work/wt`, `work/tasks`, `sandbox/home` its tools use are made
    first, and with `cards` the repo's role bundles are copied to `.fantastic/bundles/`): the kernel
    loads them at boot and every agent laid must be there after it. A test with no `fleet:` still gets
    its agents made with `send` + `create_agent` by the keywords (`make_lead`; `lead_tools` names the
    tools the lead may call, default `LEAD_TOOLS`). Sets `ctx.team`, `ctx.lead`, `ctx.work`; returns the Team."""
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    env = dict(env or {"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                       "E2E_APP_PEM": keys["pem"]})
    fleet = bool(ctx.test.get("fleet"))
    if cards or roles_tiers is not None or registry_tiers is not None:
        env["FANTASTIC_AGENTIC_RESOURCES"] = "1"
    ctx.work = os.path.join(ctx.run_dir, sub) if sub else ctx.run_dir
    os.makedirs(ctx.work, exist_ok=True)
    laid = []
    if fleet:
        for d in ("work/wt", "work/tasks", "sandbox/home"):
            os.makedirs(os.path.join(ctx.work, d), exist_ok=True)
        if cards:
            lay_role_bundles(ctx.work)
        laid = cli.lay_test_fleet(ctx)
    ctx.team = team = T.Team(ctx.binary, ctx.work, env)
    team.start(extra_args=T.HEADLESS if extra_args is None else extra_args)
    if fleet:
        missing = [i for i in laid if i not in team.agents()]
        if missing:
            raise RuntimeError(f"the kernel did not load these laid records: {missing}")
        ctx.lead = LEAD
        return team
    if credentials:
        make_credentials(ctx, team, github=forge if github_cred is None else github_cred)
    if seeded:
        # `webui --port` on a never-used folder seeded the team's fixed agents from
        # `crates/team/blueprint.yaml` (#261): the lead and what is under it are there.
        ctx.lead = LEAD
    else:
        make_tools(ctx, team, queue=queue)
        if forge:
            make_forge(ctx, team, puller=puller, github_only=not git)
        make_lead(ctx, team, lead_tools)
    if groups is not None:  # `$lead` is the lead's id
        groups = {n: {**g, **({"driver": ctx.lead} if g.get("driver") == "$lead" else {})}
                  for n, g in groups.items()}
    ctx.groups = groups
    if roles_tiers is not None:
        make_roles(ctx, team, roles_tiers, limits)
    elif registry_tiers is not None:
        make_registry(ctx, team, registry_tiers, limits)
    if board:
        make_board(ctx, team, board_store, roles=roles_tiers is not None)
    if (groups is not None) if dispatcher is None else dispatcher:
        make_dispatcher(ctx, team, groups or {}, board=board, roles=roles_tiers is not None,
                        registry=registry_tiers is not None)
    return team
