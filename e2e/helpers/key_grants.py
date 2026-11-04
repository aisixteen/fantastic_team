"""The credential-store grants of #223, for the tests of `tests/roles/*key*`.

The team binary runs as fantastic's headless CLI (no screen, no port, none of the canvas host's
"reach", which would give every AI agent every verb on every agent). The lead is made the way
`new_team.steps` makes the app's lead on the grants that matter here: `update_agent` and
`list_agents` on the root, `get` on the credential store, the roles registry, and the tools
the cards grant. The registry holds the repo's own role cards and a tier per card whose
`credentials` names the model's key, so a spawned agent's first turn reads it from the store.
"""

from __future__ import annotations

import os
import shutil
import time

from . import cli, records

REPO_ROLES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "roles")
ROOT = "kernel_state"
STORE = "credentials"
# the verbs the app's lead holds on the root (`new_team::LEAD_ROOT_VERBS`)
LEAD_ROOT_VERBS = ["await_set", "await_cancel", "create_agent", "delete_agent", "list_agents",
                   "update_agent"]


def key_ref(ctx) -> str:
    return f"model/{ctx.model.host}"


def boot(ctx, total: int = 6):
    """Start the binary, make the credential store, the main lead and the registry. Returns the
    team. The agents the cards name as tools but the test does not need are not made: a spawn
    writes a card's grant on them only when they exist."""
    cli.start(ctx, roles=True)
    cli.add_base(ctx)
    cli.make_lead(ctx, {
        ROOT: LEAD_ROOT_VERBS, STORE: ["get"],
        "worker_exec": "*", "worker_files": "*", "queue": "*", "canvas_page": ["new_canvas"],
    }, subtree=True)
    team = ctx.team
    shutil.copytree(REPO_ROLES, os.path.join(ctx.work, "roles"))
    cred = {"store": STORE, "ref": key_ref(ctx)}
    tier = {"handler_module": "anthropic_backend.tools", "model": cli.MODEL,
            "endpoint": ctx.model.endpoint, "memory_store_id": "memory", "credentials": cred}
    records.create(team, {"id": "roles", "parent_id": cli.LEAD, "handler_module": "agentic_resources.tools",
                          "role_dirs": ["roles"], "limits": {"total": total},
                          "tiers": {"small": dict(tier), "strong": dict(tier)},
                          "can_call": {"$self": "*"}, "callable_by": {"$self": "*", "$parent": "*"}})
    records.link(team, "roles", "memory")
    records.link(team, cli.LEAD, "roles", ["roles", "role", "spawn", "get_count", "fleet"])
    return team


def store_map(team) -> dict:
    return dict(team.agents()[STORE].get("callable_by") or {})


def poll(fn, wait_s: float = 120.0, every: float = 0.05):
    """Call `fn` until it returns something truthy (returned) or the time is up (None)."""
    end = time.time() + wait_s
    while time.time() < end:
        got = fn()
        if got:
            return got
        time.sleep(every)
    return None


def worker_with_get(team):
    """The store's `callable_by` as soon as it names a spawned agent with `get` alone (what the
    lead writes; fantastic's reach writes `*` for it a moment after the agent is made), else None."""
    m = store_map(team)
    return m if any(k.startswith("anthropic_backend_") and v == ["get"] for k, v in m.items()) else None


def restated(grants: dict, owner: str) -> dict:
    """`grants` as the lead sends it: a call cannot write a `$` tag, so `$self` is the owner's id."""
    return {(owner if k == "$self" else k): v for k, v in grants.items()}
