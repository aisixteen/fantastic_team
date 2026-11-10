"""The five scenarios of the cli group, defined once (issues #219, #237, #242).

A `cli/` test plays one on the scripted model; its `gpu_cli/` twin plays the same on a real model
(the vllm plug, no script). Both start on the same fleet (record files laid before the first start, `e2e/fleets/`), under
the same fixed ids and the same role cards, and run the same checks on the result. What differs is
only what the model path is: the scripted test also checks that every scripted turn was served and
that each lane asked the model (checks about a script); the gpu test hands the lead its task in
plain words and waits for the lead to answer `done`, and asserts nothing about the model's own path.

Each scenario is a `*_start(ctx, v)` that boots the team on its fleet (and checks it), and a
`*_end(ctx, v, ...)` that checks the outcome. Both return what the other half needs.
"""

from __future__ import annotations

from collections import Counter

import json
import re

from backends.scripted import ScriptedModel

from . import cli, team as team_mod

# The trees the precreated scenarios make: id -> parent, parents first.
ONE_TWO = {"lead": "user", "w1": "lead", "w2": "lead"}
TREE = {"lead": "user", "s1": "lead", "w1": "s1", "s2": "lead", "w2": "s2"}


# ---- single_lead -----------------------------------------------------------------------

def fleet_there(v, ctx, label: str, expect: dict) -> None:
    """Every agent of `expect` (id -> (handler, parent)) is there, loaded from its record file."""
    tree = cli.tree(ctx.team)
    wrong = {i: tree.get(i) for i, hp in expect.items() if tree.get(i) != hp}
    v.check(label, not wrong, f"wanted {expect}, found {wrong}")


def provider_there(ctx, parent: str = cli.LEAD) -> dict:
    """The provider and its store, under `parent`, as `fleet_there` wants them."""
    return {cli.PROVIDER: (ctx.model.ai_record()["handler_module"], parent),
            cli.PROVIDER_STORE: ("ram_store.tools", parent)}


def check_in_leads(ctx, leads) -> None:
    """The leads whose records carry the check-ins paragraph (a model test), for `gpu_cli.check_ins_ran`."""
    if cli.check_ins_on(ctx):
        ctx.check_in_leads = list(leads)


def single_by_hand(ctx, v, sub: str = "create_agent_run"):
    """A: `--headless` on the fleet laid in the folder `sub`: the lead (id `lead`), its provider
    with its store, the cadencer. Returns (lead id, the records before the task)."""
    cli.start(ctx, sub=sub)
    lead = cli.LEAD
    fleet_there(v, ctx, "A: the lead (and its provider with its store) and the cadencer are loaded, under the ids laid",
                {lead: (cli.CHAT, "user"), "cadencer": ("team_cadencer.tools", "user"), **provider_there(ctx)})
    before = cli.ids(ctx.team)
    v.check(f"A: the lead is an AI chat ({cli.CHAT}) on the team's provider",
            before.get(lead) == cli.CHAT and before.get(cli.PROVIDER) == ctx.model.ai_record()["handler_module"], str(before))
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
    cli.add_cadencer(ctx)
    return "ai", cli.ids(ctx.team)


def single_end(ctx, v, label: str, before: dict) -> bool:
    """The outcome of one single_lead run: solved, and nothing added to the records."""
    after = cli.ids(ctx.team)
    ok, why = cli.solved(ctx.repo)
    v.check(f"{label}: solved (the test passes on HEAD, the fix is committed)", ok, why)
    after = cli.without_new_schedulers(ctx.team, after, before)
    v.check(f"{label}: the records are the ones it started with (a lead's scheduler may stay)", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    return ok


# ---- lead_spawns_two -------------------------------------------------------------------

def spawn_two_start(ctx, v, credentials: bool = False) -> dict:
    """A lead with the registry and the `worker` card (room for 2). Returns the tree before.
    `credentials`: kept for the callers; a spawned agent takes its spawner's backend, the key ref
    included."""
    cli.start(ctx, roles=True, bundles=cli.spawn_roles("lead_spawns_two"))
    check_in_leads(ctx, [cli.LEAD])
    fleet_there(v, ctx, "the lead, its provider (and store) and its roles registry are loaded, under the ids laid",
                {cli.LEAD: (cli.CHAT, "user"), "roles": ("agentic_resources.tools", cli.LEAD), **provider_there(ctx)})
    count = ctx.team.send("roles", {"type": "get_count"})["result"]
    v.check("the registry is on and counts nothing running", count == {"running": 0, "max": 2}, str(count))
    return cli.tree(ctx.team)


def spawns_of(ctx) -> list:
    """The lead's `spawn` calls on the registry, from the team's log (a refused one included)."""
    return [c for c in team_mod.log_calls(ctx.team.log()) if c == ("roles", "spawn")]


def spawn_two_end(ctx, v, before: dict) -> None:
    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    after = cli.tree(ctx.team)
    after = cli.without_new_schedulers(ctx.team, after, before)
    v.check("the records are the ones it started with (a lead's scheduler may stay) (both workers deleted)", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    count = ctx.team.send("roles", {"type": "get_count"})["result"]
    v.check("the registry counts nothing running", count["running"] == 0, str(count))
    rows = cli.history_rows(ctx.team, "lead")
    told = [r for r in rows if "[team] The team of `lead` changed" in json.dumps(r)]
    v.check("the cadencer told the lead its team when the team changed", bool(told), f"{len(rows)} rows")


# ---- precreated_1_2 and precreated_tree ------------------------------------------------

def precreate(ctx, v, expect: dict) -> dict:
    """Start on the fleet of `expect` (id -> parent, parents first), laid as record files: each agent
    able to call its children, a leaf the exec agent. Returns the tree before the task."""
    cli.start(ctx)
    check_in_leads(ctx, [i for i in expect if i in expect.values()])
    fleet_there(v, ctx, "the tree is the one laid: each agent an AI chat under the parent it was laid under",
                {**{i: (cli.CHAT, p) for i, p in expect.items()}, "cadencer": ("team_cadencer.tools", "user"),
                 **provider_there(ctx)})
    return cli.tree(ctx.team)


def precreated_end(ctx, v, before: dict) -> None:
    ok, why = cli.solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    after = cli.tree(ctx.team)
    # a lead's scheduler agent may stay (its schedules are gone: `check_ins_ran`), one per lead
    scheds = {i: p for i, (h, p) in after.items() if h == cli.SCHEDULER and i not in before}
    rest = {i: s for i, s in after.items() if i not in scheds}
    v.check("the same agents under the same parents, nothing extra (a lead's scheduler may stay)",
            rest == before, f"before {sorted(before)} after {sorted(rest)}")
    per = Counter(scheds.values())
    v.check("no more than one scheduler per lead", all(n <= 1 for n in per.values()), str(dict(per)))


# ---- leads_spawn_leads -----------------------------------------------------------------

def spawn_leads_start(ctx, v, credentials: bool = False) -> dict:
    """A lead with the registry (room for 4) and the `sublead` and `worker` cards."""
    cli.start(ctx, roles=True, bundles=cli.spawn_roles("leads_spawn_leads"))
    check_in_leads(ctx, [cli.LEAD])
    fleet_there(v, ctx, "the lead, its provider (and store) and its roles registry are loaded, under the ids laid",
                {cli.LEAD: (cli.CHAT, "user"), "roles": ("agentic_resources.tools", cli.LEAD), **provider_there(ctx)})
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
    after = cli.without_new_schedulers(team, after, before)
    v.check("the records are the ones it started with (a lead's scheduler may stay)", after == before,
            f"before {sorted(before)} after {sorted(after)}")
    count = team.send("roles", {"type": "get_count"})["result"]
    v.check("the registry counts nothing running", count["running"] == 0, str(count))


# ---- checkins_busy_stuck (#284) ---------------------------------------------------------

def busy_stuck_start(ctx, v) -> dict:
    """A lead with the registry (room for 2) and the `sublead` card. Returns the tree before."""
    cli.start(ctx, roles=True, bundles=cli.roles_dir("checkins_busy_stuck"))
    check_in_leads(ctx, [cli.LEAD])
    fleet_there(v, ctx, "the lead, its provider (and store) and its roles registry are loaded, under the ids laid",
                {cli.LEAD: (cli.CHAT, "user"), "roles": ("agentic_resources.tools", cli.LEAD), **provider_there(ctx)})
    return cli.tree(ctx.team)


S1_REPORT = "s1 done: adder.py is fixed and committed"


def busy_stuck_end(ctx, v, before: dict, report: str = S1_REPORT) -> None:
    """The outcome of the busy/stuck run: the lead read the slow sub-lead `generating` on both check-ins
    and did not stop it (it finished and reported); it read the idle one NOT generating with no new row
    on both, stopped it and said `task failed: …`; both are deleted and the registry counts none."""
    team = ctx.team
    raw = [b for b, l in zip(ctx.model.raw_bodies, ctx.model.request_lanes) if l == "main"][-1]
    lead = "\n\n".join(b for m in json.loads(raw)["messages"] for b in ScriptedModel.blocks(m))
    ids = list(dict.fromkeys(re.findall(r"<id>(sublead_[0-9a-f]+)</id>", lead)))[:2]
    v.check("the lead saw the two minted sub-lead ids", len(ids) == 2, str(ids))
    busy, stuck = ids[:1] + [""], ids[1:] + [""]
    seen = re.findall(r"<source>(\w+)</source>\s*<client_id>[^<]*</client_id>\s*<generating>(true|false)</generating>", lead)
    got = lambda who: [g for s, g in seen if s == who[0]]  # noqa: E731
    v.check("the slow sub-lead read `generating` on both check-ins", got(busy).count("true") >= 2 and "false" not in got(busy),
            str(got(busy)))
    v.check("the idle sub-lead read NOT generating on both check-ins", got(stuck).count("false") >= 2 and "true" not in got(stuck),
            str(got(stuck)))
    v.check("the idle sub-lead's history had no new row on the second check-in (the reply is the first's)",
            "[identical to the reply in row" in lead, "no repeated history reply")
    stops = [a for a, verb in team_mod.log_calls(team.log()) if verb == "stop"]
    v.check("the lead stopped the idle sub-lead", stuck[0] in stops, str(stops))
    v.check("the lead did not stop the generating one", busy[0] not in stops, str(stops))
    cli.reports_reached(ctx, v, [report])
    rows = cli.history_rows(team, "lead")
    v.check("the lead said `task failed: …` for the stopped one", any("task failed:" in json.dumps(r) for r in rows),
            f"{len(rows)} rows")
    tree = cli.without_new_schedulers(team, cli.tree(team), before)
    v.check("the records are the ones it started with (a lead's scheduler may stay) (both sub-leads deleted)", tree == before,
            f"before {sorted(before)} after {sorted(tree)}")
    count = team.send("roles", {"type": "get_count"})["result"]
    v.check("the registry counts nothing running", count["running"] == 0, str(count))
