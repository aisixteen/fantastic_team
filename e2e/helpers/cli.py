"""The cli group's shared steps (issues #219, #221): one fixture, fantastic's own start, agents.

Every test in `tests/cli/` starts the team binary the way fantastic is started
(`--headless`, or `canvas --port N`) in the run folder, makes its agents with `send` +
`create_agent` under fixed ids (no blueprint, no import), hands the lead its task, and checks
the result and the record set.

THE FIXTURE is a local git repo with one failing test: `test_calc.py` checks `adder.add`
and `muler.mul`, and both are wrong. SOLVED means the test passes AND the fix is committed
(the test is run on a checkout of HEAD, and the work tree is clean). The two bugs live in
two files, so two workers can each fix one and the test passes only when both have.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import tempfile
import time

from . import records, run_flow, team as team_mod

FILES = {
    "adder.py": "def add(a, b):\n    return a - b\n",
    "muler.py": "def mul(a, b):\n    return a + b\n",
    "test_calc.py": ("from adder import add\nfrom muler import mul\n\n"
                     "assert add(2, 3) == 5, 'add is wrong'\n"
                     "assert mul(2, 3) == 6, 'mul is wrong'\n"
                     "print('ok')\n"),
}
IDENT = ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid"]


def git(repo: str, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_TERMINAL_PROMPT="0")
    return subprocess.run(["git", *IDENT, *args], cwd=repo, env=env, capture_output=True,
                          text=True, check=check)


def make_fixture(run_dir: str) -> str:
    """The failing repo at `run_dir/repo`, one commit on `main`."""
    repo = os.path.join(run_dir, "repo")
    os.makedirs(repo)
    git(repo, "init", "-q", "-b", "main")
    for name, text in FILES.items():
        with open(os.path.join(repo, name), "w") as f:
            f.write(text)
    git(repo, "add", "-A")
    git(repo, "commit", "-q", "-m", "calc with two bugs")
    return repo


def test_passes(path: str) -> bool:
    r = subprocess.run(["python3", "test_calc.py"], cwd=path, capture_output=True, text=True,
                       env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"))
    return r.returncode == 0 and "ok" in r.stdout


def solved(repo: str) -> tuple[bool, str]:
    """(solved, why not): the test fails on the start commit, passes on HEAD, the fix is a
    commit (not an edit in the work tree), and the work tree is clean."""
    log = git(repo, "log", "--format=%H").stdout.split()
    if len(log) < 2:
        return False, f"no fix commit: {len(log)} commit(s)"
    status = git(repo, "status", "--porcelain").stdout.strip()
    if status:
        return False, f"the work tree is not clean: {status!r}"
    with tempfile.TemporaryDirectory() as tmp:
        for rev, want in ((log[-1], False), (log[0], True)):
            out = os.path.join(tmp, rev[:8])
            os.makedirs(out)
            tar = subprocess.run(["git", "archive", rev], cwd=repo, capture_output=True, check=True)
            subprocess.run(["tar", "-x", "-C", out], input=tar.stdout, check=True)
            if test_passes(out) != want:
                return False, f"the test {'fails' if want else 'passes'} on {rev[:8]}"
    return True, ""


# ---- the scripted turns every test shares ----------------------------------------------

EXEC = "worker_exec"


def run_call(command: str, *args: str, target: str = EXEC, wait_s: int = 30, verb: str = "run") -> dict:
    """A scripted call to an exec agent: `run` waits `wait_s`, `start` (a job) does not."""
    call_args = {"command": command, "args": list(args)}
    if verb == "run":
        call_args["wait_s"] = wait_s
    return {"target": target, "type": verb, "args": call_args}


def replace_call(path: str, old: str, new: str) -> dict:
    code = f"import pathlib; p = pathlib.Path('{path}'); p.write_text(p.read_text().replace('{old}', '{new}'))"
    return run_call("python3", "-c", code)


def solve_turns(fixes=(("adder.py", "a - b", "a + b"), ("muler.py", "a + b", "a * b")),
                msg: str = "Fix the calc", lane: str | None = None, after=()) -> list[dict]:
    """The turns of an agent that fixes the given files, runs the test, commits, and ends.
    One call per turn. `lane`/`after` are the scripted plug's (see backends/scripted.py)."""
    calls = [run_call("python3", "test_calc.py")]
    calls += [replace_call(*f) for f in fixes]
    calls += [run_call("python3", "test_calc.py"),
              run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                       "commit", "-q", "-a", "-m", msg)]
    turns = [{"calls": [c]} for c in calls] + [{"calls": [{"name": "done"}]}]
    for t in turns:
        if lane:
            t["lane"] = lane
    if after:
        turns[0]["after"] = list(after)
    return turns


# ---- boot ------------------------------------------------------------------------------

def tool_record(repo: str, work: str) -> dict:
    """The exec agent the agents run their commands through: in the repo, any command, a
    fixed environment (no key of the run is in it)."""
    return {"handler_module": "exec_bridge.tools", "ingress_rule": "allow_all_ingress",
            "cwd": repo, "allowed": [".*"],
            "inherit_env": ["PATH", "LANG", "TMPDIR"],
            "env": {"HOME": os.path.join(work, "sandbox"), "GIT_CONFIG_NOSYSTEM": "1",
                    "GIT_TERMINAL_PROMPT": "0", "PYTHONDONTWRITEBYTECODE": "1"}}


def create(team, payload: dict) -> dict:
    return records.create(team, payload)


def start(ctx, extra_args=team_mod.HEADLESS, roles: bool = False, sub: str | None = None,
          env: dict | None = None):
    """Start the team binary as fantastic's CLI, in the run folder (or its `sub` folder, for a
    test that runs twice). `roles` turns on the `agentic_resources` bundle
    (`FANTASTIC_AGENTIC_RESOURCES=1`). `env`: more variables for the kernel (a credential store's
    `env` backend reads its values from them). Returns the Team; `ctx.repo` is the fixture."""
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    env = {"E2E_MODEL_KEY": keys["model_key"], **(env or {})}
    if roles:
        env["FANTASTIC_AGENTIC_RESOURCES"] = "1"
    ctx.work = os.path.join(ctx.run_dir, sub) if sub else ctx.run_dir
    os.makedirs(ctx.work, exist_ok=True)
    # the run folders sit inside the team's own checkout: a `git` an agent runs outside the
    # fixture repo must stop at the run folder, never reach the checkout above it
    env["GIT_CEILING_DIRECTORIES"] = os.path.dirname(os.path.abspath(ctx.work))
    ctx.repo = make_fixture(ctx.work)
    os.makedirs(os.path.join(ctx.work, "sandbox"), exist_ok=True)
    ctx.team = team_mod.Team(ctx.binary, ctx.work, env)
    ctx.team.start(extra_args=extra_args)
    if hasattr(ctx.model, "watch"):  # the vllm and anthropic plugs count the agents' usage through the team
        ctx.model.watch(ctx.team)
    return ctx.team


def add_base(ctx) -> None:
    """The two agents every test of this group's lead and workers use: the credential store
    (the model's key from the environment, the way `backend: env` says) and the exec agent."""
    create(ctx.team, {"id": "credentials", "parent_id": "user", "handler_module": "credential_store.tools",
                      "backend": "env",
                      "refs": {f"model/{ctx.model.host}": {"host": ctx.model.host,
                                                            "env": {"api_key": "E2E_MODEL_KEY"}}}})
    create(ctx.team, dict(tool_record(ctx.repo, ctx.work), id=EXEC, parent_id="user"))


LEAD = "lead"
MODEL = "scripted-model"

# A lead's check-ins: its own scheduler wakes it to look at its agents (the team's lead bundles
# carry the same paragraph). Given only on a model test (costs gpu): a script follows no prompt.
CHECK_INS = (
    "Check-ins: before your first spawn or message to an agent, make your own scheduler (`create_agent` on "
    "`kernel_state`: `handler_module: scheduler.tools`, `parent_id`: your id, `can_call: {\"$parent\": [\"send\"]}`, "
    "`callable_by: {\"$parent\": \"*\"}`) and `schedule` on it `{target: <your id>, payload: {type: send, text: "
    "\"[check-in agents] check your agents\"}, interval_seconds: 120}`; start every schedule's text with "
    "`[check-in <name>]` so you know what ticked, and change an interval when the pace changes (`unschedule`, "
    "then `schedule`). On each check-in look at every agent you started: one that loops or is stuck you halt "
    "(`stop`) and report `task failed: <agent>: <why>` to your parent. Stuck is: NOT generating (its `status`) "
    "and no new row since your last check-in, two check-ins in a row; the same step or error over and over is "
    "looping. An agent that is generating is busy, never stuck. Keep the check-ins until your task is complete: unscheduling every schedule "
    "(`unschedule`) is your very last call, after every other step and wait, right before you end; the "
    "scheduler agent stays. Never end your run while an agent you started is still working; its report or your "
    "next check-in wakes you. To wait, end your turn with `await`; never start a timer or sleep job to wait.")
SCHEDULER_VERBS = ["schedule", "unschedule", "list", "pause", "resume", "tick_now", "history"]


def check_ins_on(ctx) -> bool:
    """Leads get `CHECK_INS` on a model test, unless `E2E_CHECK_INS=0` (the A/B run without them)."""
    return ctx.test.get("costs") == "gpu" and os.environ.get("E2E_CHECK_INS", "1") != "0"


def _with(grants: dict, key: str, verbs: list) -> None:
    """Add `verbs` to `grants[key]` (a `"*"` stays)."""
    have = grants.get(key)
    if have == "*" or have == ["*"]:
        return
    grants[key] = list(dict.fromkeys([*(have or []), *verbs]))


def check_ins(ctx, rec: dict) -> None:
    """Make `rec` a lead with check-ins: the grants to make, use and delete its own scheduler, and
    `$subtree` `send` so the scheduler, its child, may wake it; on a model test also `CHECK_INS`
    (the grants go to the scripted twin too: the sub-lead bundle it spawns asks for them)."""
    _with(rec["can_call"], "$subtree", SCHEDULER_VERBS)
    _with(rec["can_call"], "kernel_state", ["create_agent", "delete_agent"])
    _with(rec["callable_by"], "$subtree", ["send"])
    if not check_ins_on(ctx):
        return
    rec["post_system_prompt"] = CHECK_INS
    ctx.check_in_leads = [*getattr(ctx, "check_in_leads", []), rec["id"]]


# THE PROVIDER: one per team, the connection to the plug's model (its kind, endpoint, key, the
# models it serves and its default). Every AI agent of a test is an AI chat (`ai_chat.tools`)
# completing through it; none names a provider field. Made under the first chat (the lead).
SCHEDULER = "scheduler.tools"  # a lead's check-in scheduler: at most one per lead, may stay
PROVIDER = "provider"
PROVIDER_STORE = "provider_store"
CHAT = "ai_chat.tools"
PROVIDER_VERBS = ["complete", "watch"]


def provider_record(ctx, parent: str) -> dict:
    """The provider's record: the plug's AI fields (its `model` is seeded as the default in the
    provider's store by `add_provider`), the models the run uses (`E2E_MODEL`, and
    `E2E_WORKER_MODEL` when set) as its list, the key when keyed."""
    rec = dict(ctx.model.ai_record())
    default = rec.pop("model", None)
    models = list(dict.fromkeys(m for m in (default, worker_model()) if m))
    rec.update({"id": PROVIDER, "parent_id": parent, "models": models,
                "memory_store_id": PROVIDER_STORE,  # its own state (the default model) is kept there
                "can_call": {"$self": "*", PROVIDER_STORE: "*"},
                "callable_by": {"$self": "*", "$parent": [*PROVIDER_VERBS, "models", "set_default_model"]}})
    if ctx.model.keyed:
        rec["credentials"] = model_credentials(ctx)
        rec["can_call"]["credentials"] = ["get"]
    return rec


def plant_provider_default(team, model: str) -> None:
    """PRESEED the provider's memory store with its default model, from above (fantastic's e2e
    `memory.plant`): `dump` the store, put the `setting` row under `<guid>|provider` (the key the
    provider reads under), `inject`. The row the provider reads: `{kind: setting, name:
    default_model, value}`."""
    for _ in range(50):  # the kernel answers `send` a moment after its socket opens
        r = team.send("kernel", {"type": "reflect"}).get("result") or {}
        if r.get("guid"):
            break
        time.sleep(0.2)
    owner = f"{r['guid']}|{PROVIDER}"
    dump = team.send(PROVIDER_STORE, {"type": "dump"}).get("result") or {}
    rows = [x for x in dump.get("rows") or []
            if not (x.get("owner") == owner and (x.get("row") or {}).get("kind") == "setting")]
    rows.append({"owner": owner, "row": {"id": 1, "kind": "setting", "name": "default_model", "value": model}})
    done = team.send(PROVIDER_STORE, {"type": "inject", "tables": {"rows": rows, "fold_refs": dump.get("fold_refs") or []}})
    assert "error" not in (done.get("result") or {}), f"plant the provider's default: {done}"


def add_provider(ctx, parent: str) -> None:
    """Make the provider under `parent` (once), admitting every chat that exists."""
    team = ctx.team
    have = team.agents()
    if PROVIDER in have:
        return
    rec = provider_record(ctx, parent)
    for id, a in have.items():
        if a.get("handler_module") != CHAT and a.get("provider_id") != PROVIDER:
            continue
        rec["callable_by"][id] = PROVIDER_VERBS
    # as fantastic's e2e plants a conversation (E75: `plant_store`, then the agent): the store
    # first, a ram_store, a child of the lead like the provider; the provider's default model
    # PLANTED in it under the provider's key before the provider exists; then the provider, which
    # reads it there. A RAM store starts empty: every later start of the team plants it again.
    records.create(team, {"id": PROVIDER_STORE, "parent_id": parent, "handler_module": "ram_store.tools",
                          "callable_by": {"$self": "*", PROVIDER: "*"}})
    default = ctx.model.ai_record().get("model")
    if default:
        plant = lambda t: plant_provider_default(t, default)  # noqa: E731
        plant(team)
        team.on_start = [*getattr(team, "on_start", []), plant]
    records.create(team, rec)
    if ctx.model.keyed and "credentials" in have:
        records.merge(team, "credentials", "callable_by", {PROVIDER: ["get"]}, team.agents())


def make_agent(ctx, id: str, parent: str, grants: dict | None = None, key: bool = False,
               lead: bool = False, **extra) -> str:
    """One AI agent: an AI chat (`ai_chat.tools`) under `parent` with the fixed id `id`, completing
    through the team's provider (`PROVIDER`, made under the first chat) on the provider's default
    model, or the `model` named in `extra`. `grants` is its `can_call` beyond `$self`, the shared
    `memory` store and its provider (agent -> verbs, `"*"` for every verb). An edge is written on
    both ends: the caller's `can_call` in this payload, the callee's `callable_by` for the callees
    that exist already (a child made later admits its parent through `$parent`). `key` is kept for
    the callers: the key is the provider's, never a chat's. `lead`: it runs agents, so it gets
    check-ins (`check_ins`). Returns `id`."""
    team = ctx.team
    can_call = {"$self": "*", "memory": "*", PROVIDER: PROVIDER_VERBS, **(grants or {})}
    rec = {"id": id, "parent_id": parent, "handler_module": CHAT, "provider_id": PROVIDER,
           "memory_store_id": "memory", "can_call": can_call, "callable_by": {"$self": "*", "$parent": "*"}}
    if lead:
        check_ins(ctx, rec)
        can_call = rec["can_call"]
    rec.update(extra)
    records.create(team, rec)
    if PROVIDER not in team.agents():
        add_provider(ctx, id)
    have = team.agents()
    for callee, verbs in can_call.items():
        if callee in have and callee != "$self":
            records.merge(team, callee, "callable_by", {id: verbs}, have)
    return id


def make_lead(ctx, grants: dict | None = None, subtree: bool = False, runs_agents: bool = False) -> str:
    """The lead: `make_agent` under `user` with the model's key. `subtree` lets it call
    everything below it; `runs_agents`: it has agents to look after, so it gets check-ins."""
    grants = dict(grants or {})
    # fantastic's own hints send every agent to `send kernel {type: docs, of: ...}` for a verb's
    # signature: a lead that may not read them stops and asks the operator, who is not there
    # (only a listed grant is extended: with none, the lead keeps the reach it has)
    ks = grants.get("kernel_state")
    if isinstance(ks, list):
        grants["kernel_state"] = list(dict.fromkeys([*ks, "docs"]))
    lead = make_agent(ctx, LEAD, "user", {**({"$subtree": "*"} if subtree else {}), **grants},
                      key=True, lead=runs_agents)
    return lead


TESTS = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "tests")


def roles_dir(name: str) -> str:
    """The role bundles of the cli test `name` (`<role>/bundle.yaml` each): the gpu twin of a
    test reads the same ones."""
    return os.path.join(TESTS, "cli", name, "bundles")


def model_credentials(ctx) -> dict | None:
    """The `credentials` an AI of this plug carries (the store and the model's ref), or None for
    a keyless server."""
    return {"store": "credentials", "ref": f"model/{ctx.model.host}"} if ctx.model.keyed else None


def add_cadencer(ctx) -> None:
    """The cadencer (`team_cadencer.tools`, id `cadencer`), made once the test's own agents are, so
    their making tells nobody: from then on each AI agent added or removed tells its lead's team the
    team's structure (a turn on each of them)."""
    create(ctx.team, {"id": "cadencer", "parent_id": "user", "handler_module": "team_cadencer.tools",
                      "callable_by": {"$self": "*", "$parent": "*"}})
    ctx.team.send("cadencer", {"type": "boot"})


def schedules_left(ctx) -> list[str]:
    """The schedules still set at the end, as `<scheduler>:<schedule id>`: a lead unschedules its
    check-ins as its last call; its scheduler agent may stay."""
    left = []
    for i, a in ctx.team.agents().items():
        if a.get("handler_module") != "scheduler.tools":
            continue
        got = ctx.team.send(i, {"type": "list"}, timeout=60).get("result") or {}
        left += [f"{i}:{s.get('id') or s.get('schedule_id')}" for s in got.get("schedules") or []]
    return left


def without_new_schedulers(team, after, before):
    """`after` (a tree dict or an id list) without the scheduler agents made during the run: a
    lead's scheduler may stay (`check_ins_ran` checks it holds no schedule and is one per lead)."""
    new = {i for i, a in team.agents().items() if a.get("handler_module") == SCHEDULER and i not in before}
    if isinstance(after, dict):
        return {k: v for k, v in after.items() if k not in new}
    return type(after)(x for x in after if x not in new)


def schedulers_per_lead(ctx) -> dict:
    """lead id -> how many scheduler agents it has (a lead keeps at most one)."""
    per: dict = {}
    for a in ctx.team.agents().values():
        if a.get("handler_module") == SCHEDULER:
            per[a.get("parent_id")] = per.get(a.get("parent_id"), 0) + 1
    return per


def check_ins_played(ctx, v, lanes=("main",)) -> None:
    """Each scripted lead (its lane) made its own scheduler, a tick of it reached the lead, and the
    no schedule is left at the end (its scheduler agent may stay). Call it while the team runs."""
    got = ctx.model.check_in_lanes
    v.check(f"a check-in from its own scheduler reached each lead ({', '.join(lanes)})", set(lanes) <= got,
            f"lanes with a check-in: {sorted(got)}, {ctx.model.check_in_turns} requests")
    left = schedules_left(ctx)
    v.check("no schedule is left (a scheduler agent may stay)", not left, str(left))
    per = schedulers_per_lead(ctx)
    v.check("no more than one scheduler per lead", all(n <= 1 for n in per.values()), str(per))
    # team#291: a lead waits by ending its turn with `await` (a tick or a report wakes it), never with a
    # timer: no model request carries a `sleep` job (a whole-second `sleep N` as a timer; the setup's `sleep 0.2` poll loops are not one)
    timer = [b[:80] for b in ctx.model.raw_bodies if re.search(r"<command>\s*sleep\s*</command>|sleep [1-9]\d*(?![\d.])", b)]
    v.check("no sleep job was started to wait", not timer, str(timer[:2]))


def report_call(text: str, parent: str = LEAD) -> dict:
    """A worker's report to its parent: a plain `send` (no report verb), which gives the parent a turn."""
    return {"target": parent, "type": "send", "args": {"text": text}}


def reports_reached(ctx, v, texts, lane: str = "main") -> None:
    """Each report `texts` reached the AI parent (`lane`) as a message it asked the model with: the
    parent was given a turn by it (a request of its own whose conversation holds the text) and the
    script's next turn for it was served."""
    asked = [b for b, l in zip(ctx.model.raw_bodies, ctx.model.request_lanes) if l == lane]
    missing = [t for t in texts if not any(t in b for b in asked)]
    v.check(f"each report (a plain send) reached its parent and gave it a turn ({lane})",
            not missing and ctx.model.consumed, f"not seen by {lane}: {missing}")


def worker_model() -> str | None:
    """The workers' model (`E2E_WORKER_MODEL`): a precreated worker runs it and the lead is told to
    spawn its workers with it, so leads (`E2E_MODEL`) and workers run different models; None: every
    agent runs the lead's own."""
    return os.environ.get("E2E_WORKER_MODEL") or None


def make_registry(ctx, total: int, parent: str = LEAD, roles: str | None = None) -> str:
    """The roles registry `roles` (`agentic_resources`) under `parent`, with `limits.total`, and
    the test's ROLE BUNDLES (a folder of `<role>/bundle.yaml`; default the test's own `bundles/`)
    written under the team's `.fantastic/bundles/` and loaded into the running kernel
    (`reload_bundle`). A role names no model: a spawned agent runs on its spawner's backend, with
    the model the spawn names. The lead may call the registry."""
    team = ctx.team
    src = roles or os.path.join(ctx.dir, "bundles")
    dst = os.path.join(ctx.work, ".fantastic", "bundles")
    for name in sorted(os.listdir(src)):
        shutil.copytree(os.path.join(src, name), os.path.join(dst, name), dirs_exist_ok=True)
        if not check_ins_on(ctx):  # a sub-lead bundle carries the check-ins paragraph: off, it goes
            y = os.path.join(dst, name, "bundle.yaml")
            with open(y) as f:
                text = f.read()
            with open(y, "w") as f:
                f.write(text.replace("\n\n    " + CHECK_INS, ""))
        loaded = team.send("kernel", {"type": "reload_bundle", "name": name})
        assert (loaded.get("result") or {}).get("reloaded") == name, f"bundle {name}: {loaded}"
    records.create(team, {"id": "roles", "parent_id": parent, "handler_module": "agentic_resources.tools",
                          "limits": {"total": total},
                          "can_call": {"$self": "*"}, "callable_by": {"$self": "*", "$parent": "*"}})
    records.link(team, "roles", "memory")
    records.link(team, parent, "roles", ["roles", "role", "spawn", "get_count", "fleet"])
    return "roles"


def ids(team) -> dict:
    """Every agent's id with its handler: the record set, to compare before and after."""
    return {i: a.get("handler_module") for i, a in team.agents().items()}


def tree(team) -> dict:
    """The record set with its shape: id -> (handler, parent)."""
    return {i: (a.get("handler_module"), a.get("parent_id")) for i, a in team.agents().items()}


def delete_calls(*targets: str) -> list[dict]:
    return [{"target": "kernel", "type": "delete_agent", "args": {"id": t}} for t in targets]


def wait_served(ctx, wait_s: float = 120.0) -> bool:
    """Wait until the scripted model has played every turn; then let the last reply settle."""
    import time
    end = time.time() + wait_s
    while time.time() < end and ctx.model.served < len(ctx.model.turns):
        time.sleep(0.3)
    time.sleep(2.0)
    return ctx.model.served >= len(ctx.model.turns)


def dump(obj) -> str:
    return json.dumps(obj, sort_keys=True)[:600]
