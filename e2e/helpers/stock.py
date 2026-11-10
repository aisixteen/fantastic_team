"""The stock team of the older e2e tests, as fantastic record files (issues #219, #221).

The team binary is fantastic's command line (`--headless`, `webui --port N`) and has no config
file and no blueprint. A test names its fleet in its `test.yaml` (`fleet: <layer> ...`): the layers
of `e2e/fleets/` and the test's own `fleet/` folder are copied into the run folder's `.fantastic/`
before the first start (`cli.lay_test_fleet`), so the kernel loads the lead, the credential store,
the worker's exec and files, the connectors, the puller, the board, the dispatcher, the cadencer, the
roles registry and the team page from record files, under the fixed ids the product's code and the
role cards name. The tests that run on the product's own seed (`seeded=True`) name no fleet.

Nothing here makes an agent but what a scenario does as one of its own steps.
"""

from __future__ import annotations

import os
import time

from . import cli, records, run_flow, team as T
from .records import link, merge, update

LEAD = "lead"


# ---- the credential store of a seeded folder ---------------------------------------------

def make_credentials(ctx, team) -> None:
    """The credential store (`backend: env`) with the model's key ref: the one record a seeded
    folder does not have (a record file there would need a seed run before the real start)."""
    host = ctx.vars["model_host"]
    refs = {f"model/{host}": {"host": host, "env": {"api_key": "E2E_MODEL_KEY"}}}
    records.create(team, dict(id="credentials", parent_id="user", handler_module="credential_store.tools",
                              backend="env", refs=refs))


# ---- role bundles, the project lead, the page ---------------------------------------------

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


def operator_holds(team, targets: dict) -> None:
    """The operator (the `send` leg, `mcp`) may call `targets` (agent -> verbs): both ends. A
    spawner grants only what it holds, so the operator holds what a card grants before it spawns."""
    have = team.agents()
    for t, verbs in targets.items():
        if t in have:
            merge(team, t, "callable_by", {"mcp": verbs}, have)
    merge(team, "mcp", "can_call", dict(targets))


def spawn_project_lead(ctx, team) -> dict:
    """The registry's spawned tree is rooted at the `project_lead` (what the cadencer digests, in roles
    mode): the team gets its provider (the scripted model, under the lead) and the operator spawns
    one naming it. A spawner grants only what it holds, so the operator holds what the card grants
    first. A child takes its memory store from its spawner and the operator has none, so the
    spawned lead gets the team's store here, before its first message (its workers copy it).
    Returns the spawn's result (`id`, ...)."""
    operator_holds(team, {"github": "*", "roles": "*", "kernel_state": "*", "canvas_page": ["new_canvas"]})
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


def serve_webui(team, port: int, wait_s: float = 30.0) -> None:
    """Stop the headless team and start it again with the `webui --port N` word (#224), which puts
    the port on the `web` host its fleet laid; wait until the page is served."""
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


# ---- the whole stock team ---------------------------------------------------------------

def boot(ctx, env: dict | None = None, *, extra_args=None, seeded: bool = False,
         sub: str | None = None, cards: bool = False):
    """Start the team binary as fantastic's CLI in the run folder. A test whose `test.yaml` names a
    `fleet:` has its agents as record files, laid in `.fantastic/` before the first start
    (`cli.lay_test_fleet`; the folders `work/wt`, `work/tasks`, `sandbox/home` its tools use are made
    first, and with `cards` the repo's role bundles are copied to `.fantastic/bundles/`): the kernel
    loads them at boot and every agent laid must be there after it. A `seeded` test names no fleet: the
    product's seed (`webui --port` on a never-used folder) makes the team's fixed agents, and only the
    credential store is made with `send`. Sets `ctx.team`, `ctx.lead`, `ctx.work`; returns the Team."""
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    env = dict(env or {"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                       "E2E_APP_PEM": keys["pem"]})
    fleet = bool(ctx.test.get("fleet"))
    if fleet == seeded:
        raise RuntimeError("a test names a `fleet:` or is `seeded`, never both and never neither")
    if cards:
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
    else:
        make_credentials(ctx, team)
    ctx.lead = LEAD
    return team
