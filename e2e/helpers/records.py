"""Agent records made with fantastic's own verbs, for every helper and test (issue #221).

A test's team is record files (`e2e/fleets/`, named by `fleet:` in its `test.yaml`), laid before the
first start (`team.lay_fleet`). What a scenario does with agents while it runs is a `create_agent`
payload sent to the root with `send`, under an id the test chooses, and a grant is written on BOTH
ends of the edge (the caller's `can_call` and the callee's `callable_by`). `update_agent` replaces a
whole map, so `merge` restates the entries the agent has.
"""

from __future__ import annotations

ROOT = "kernel_state"  # fantastic's root agent


def create(team, payload: dict) -> dict:
    """`create_agent` on the root; the result, or a RuntimeError when it was refused."""
    r = team.send("kernel", dict(payload, type="create_agent"))
    if r.get("status") != "completed" or (r.get("result") or {}).get("error"):
        raise RuntimeError(f"create_agent {payload.get('id')}: {r}")
    return r["result"]


def update(team, agent: str, **fields) -> dict:
    r = team.send("kernel", dict(fields, type="update_agent", id=agent))
    if r.get("status") != "completed" or (r.get("result") or {}).get("error"):
        raise RuntimeError(f"update_agent {agent}: {r}")
    return r


def merge(team, agent: str, field: str, entries: dict, have: dict | None = None) -> None:
    """Add `entries` to `agent`'s `can_call`/`callable_by`, keeping the entries it has."""
    rec = (have if have is not None else team.agents())[agent]
    cur = dict(rec.get(field) or {})
    cur.update(entries)
    update(team, agent, **{field: cur})


def link(team, caller: str, callee: str, verbs="*") -> None:
    """`caller` may call `callee` (verbs): both ends of the edge, as the kernel wants them."""
    have = team.agents()
    if callee in have:
        merge(team, callee, "callable_by", {caller: verbs}, have)
    if caller in have:
        merge(team, caller, "can_call", {callee: verbs}, have)
