"""Shared steps of the packs tests: a plain team (no puller, no queue), a source worker,
and the `pack` agent's verbs through `send`."""

from __future__ import annotations

import json
import os
import secrets

from . import team as team_mod

WORKER_BLUEPRINT = """agents:
  - id: scribe
    handler_module: ram_store.tools
    can_call:
      '@connector:github': '*'
ports:
  '@connector:github':
    kind: connector
    why: reads issues
    used_by: [scribe]
"""


def boot_plain(ctx, name: str) -> team_mod.Team:
    """Boot a team in `<run_dir>/<name>` from the test's team.yaml. No `wire`: the packs
    tests need no credentials grant, puller or queue folder."""
    d = os.path.join(ctx.run_dir, name)
    os.makedirs(d, exist_ok=True)
    t = team_mod.Team(ctx.binary, d, {})
    team_mod.boot(t, os.path.join(ctx.dir, "team.yaml"), dict(ctx.vars, run_dir=d))
    # The harness boots twice (see helpers/team.py): the first start seeds the base tree, whose
    # `files` and `user` agents then take the ids the team plan's own were to have, and their
    # `callable_by` does not name the pack agent as the plan writes it. Grant it, keeping the rest.
    grant(t, "files", "pack", "*")
    grant(t, "files", "pack_listing", "*")
    grant(t, "user", "pack", ["import_subtree", "send"])
    return t


def grant(team: team_mod.Team, agent: str, caller: str, verbs) -> None:
    """Add `caller: verbs` to `agent`'s callable_by, keeping the other entries."""
    r = team.send("kernel", {"type": "list_agents", "limit": 200})
    rows = r.get("result", r).get("agents", [])
    cb = dict(next(a for a in rows if a.get("id") == agent).get("callable_by", {}))
    cb[caller] = verbs
    r = team.send("kernel", {"type": "update_agent", "id": agent, "callable_by": cb})
    if r.get("status") != "completed":
        raise RuntimeError(f"cannot grant {caller} on {agent}: {r}")


def write_source_worker(team_dir: str) -> str:
    """`w/` under the team dir: a worker record (with a fake key and a model binding), a
    directive and a blueprint with one agent that needs the github connector.
    Returns the fake key (built from pieces, so no source scan trips on it)."""
    key = "".join(["sk-", "ant-", secrets.token_hex(20)])
    w = os.path.join(team_dir, "w")
    for sub in ("directives", os.path.join("blueprint", "scribe")):
        os.makedirs(os.path.join(w, sub), exist_ok=True)
    rec = {"name": "scribe", "version": "1.0.0", "author": "e2e", "license": "MIT",
           "ports": ["@connector:github"], "model": "scripted-model", "api_key": key}
    with open(os.path.join(w, "worker.json"), "w") as f:
        json.dump(rec, f)
    with open(os.path.join(w, "directives", "main.md"), "w") as f:
        f.write("Summarise the issue in three lines.\n")
    with open(os.path.join(w, "blueprint", "blueprint.yaml"), "w") as f:
        f.write(WORKER_BLUEPRINT)
    with open(os.path.join(w, "blueprint", "scribe", "directive.md"), "w") as f:
        f.write("Read the issue and summarise it.\n")
    return key


def send_pack(team: team_mod.Team, payload: dict) -> dict:
    """The `pack` agent's reply (the operator leg, as `fantastic_team send`)."""
    r = team.send("pack", payload)
    inner = r.get("result", r)
    return inner if isinstance(inner, dict) else {"error": str(r)}


def agents(team: team_mod.Team) -> set:
    """The ids of the live agents."""
    out, after = set(), None
    for _ in range(50):
        ask = {"type": "list_agents", "limit": 200}
        if after:
            ask["after"] = after
        r = team.send("kernel", ask)
        r = r.get("result", r)
        out.update(a["id"] for a in r.get("agents", []) if isinstance(a, dict) and "id" in a)
        after = r.get("next")
        if not isinstance(after, str):
            break
    return out


def tree_text(root: str) -> str:
    """Every file under `root`, concatenated."""
    parts = []
    for base, _, names in os.walk(root):
        for n in names:
            try:
                parts.append(open(os.path.join(base, n), errors="replace").read())
            except OSError:
                pass
    return "\n".join(parts)
