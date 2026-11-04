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
import shutil
import subprocess
import tempfile

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


def run_call(command: str, *args: str, target: str = EXEC) -> dict:
    return {"target": target, "type": "run", "args": {"command": command, "args": list(args), "wait_s": 30}}


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


def start(ctx, extra_args=team_mod.HEADLESS, roles: bool = False, sub: str | None = None):
    """Start the team binary as fantastic's CLI, in the run folder (or its `sub` folder, for a
    test that runs twice). `roles` turns on the `agentic_resources` bundle
    (`FANTASTIC_AGENTIC_RESOURCES=1`). Returns the Team; `ctx.repo` is the fixture."""
    keys = run_flow.fake_keys(ctx)
    ctx.secrets = keys
    env = {"E2E_MODEL_KEY": keys["model_key"]}
    if roles:
        env["FANTASTIC_AGENTIC_RESOURCES"] = "1"
    ctx.work = os.path.join(ctx.run_dir, sub) if sub else ctx.run_dir
    os.makedirs(ctx.work, exist_ok=True)
    ctx.repo = make_fixture(ctx.work)
    os.makedirs(os.path.join(ctx.work, "sandbox"), exist_ok=True)
    ctx.team = team_mod.Team(ctx.binary, ctx.work, env)
    ctx.team.start(extra_args=extra_args)
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


def make_agent(ctx, id: str, parent: str, grants: dict | None = None, key: bool = False,
               **extra) -> str:
    """One AI agent of the scripted model: `create_agent` under `parent` with the fixed id `id`.
    `grants` is its `can_call` beyond `$self` and the shared `memory` store (agent -> verbs,
    `"*"` for every verb). An edge is written on both ends: the caller's `can_call` in this
    payload, the callee's `callable_by` for the callees that exist already (a child made later
    admits its parent through `$parent`). `key` gives it the model's credential ref (a spawned
    agent has none: a spawn does not open the credential store). Returns `id`."""
    team = ctx.team
    can_call = {"$self": "*", "memory": "*", **({"credentials": "*"} if key else {}), **(grants or {})}
    rec = {"id": id, "parent_id": parent, "handler_module": "anthropic_backend.tools",
           "model": MODEL, "endpoint": ctx.model.endpoint, "memory_store_id": "memory",
           "can_call": can_call, "callable_by": {"$self": "*", "$parent": "*"}}
    if key:
        rec["credentials"] = {"store": "credentials", "ref": f"model/{ctx.model.host}"}
    rec.update(extra)
    records.create(team, rec)
    have = team.agents()
    for callee, verbs in can_call.items():
        if callee in have and callee != "$self":
            records.merge(team, callee, "callable_by", {id: verbs}, have)
    return id


def make_lead(ctx, grants: dict | None = None, subtree: bool = False) -> str:
    """The lead: `make_agent` under `user` with the model's key. `subtree` lets it call
    everything below it."""
    lead = make_agent(ctx, LEAD, "user", {**({"$subtree": "*"} if subtree else {}), **(grants or {})},
                      key=True)
    return lead


def make_registry(ctx, total: int, parent: str = LEAD) -> str:
    """The roles registry `roles` (fantastic's `agentic_resources`) under `parent`: one tier
    `small` (the scripted model), the role cards of the test's `roles/` folder copied next to
    the kernel (where `role_dirs: [roles]` reads them), `limits.total`. The lead may call it."""
    team = ctx.team
    shutil.copytree(os.path.join(ctx.dir, "roles"), os.path.join(ctx.work, "roles"))
    records.create(team, {"id": "roles", "parent_id": parent, "handler_module": "agentic_resources.tools",
                          "role_dirs": ["roles"], "limits": {"total": total},
                          "tiers": {"small": {"handler_module": "anthropic_backend.tools", "model": MODEL,
                                              "endpoint": ctx.model.endpoint, "memory_store_id": "memory"}},
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


def precreated(ctx, v, expect: dict, lanes: set[str], task: str) -> None:
    """The shared run of the tests whose whole tree is made with `create_agent` before the task:
    make the agents of `expect` (id -> parent, parents first), hand the lead its task, then check:
    solved; the tree is exactly `expect` and unchanged by the run; every lane in `lanes` asked
    the model. An agent may call its children; one with no child may call the exec agent."""
    start(ctx)
    add_base(ctx)
    base = set(ids(ctx.team))
    for id, parent in expect.items():
        kids = {c: "*" for c, p in expect.items() if p == id}
        make_agent(ctx, id, parent, kids or {EXEC: "*"}, key=(id == LEAD))
    before = tree(ctx.team)
    v.check("the tree is the one made and the base, nothing else",
            set(before) == base | set(expect), str(sorted(set(before) - base)))
    v.check("each agent sits under the parent it was made under",
            all(before[i][1] == p for i, p in expect.items()),
            str({i: before[i][1] for i in expect}))
    ctx.team.send(LEAD, {"type": "send", "text": task})
    v.check("the scripted model played every turn, none past the end",
            wait_served(ctx, 120) and ctx.model.consumed,
            f"served {ctx.model.served} of {len(ctx.model.turns)}")
    ok, why = solved(ctx.repo)
    v.check("solved (the test passes on HEAD, the fix is committed)", ok, why)
    v.check("each agent of the tree that was told a task asked the model", lanes <= set(ctx.model.request_lanes),
            str(sorted(set(ctx.model.request_lanes))))
    after = tree(ctx.team)
    v.check("the same agents under the same parents, nothing extra", after == before,
            f"before {sorted(before)} after {sorted(after)}")


ALT_SCREEN = ("\x1b[?1049h", "\x1b[?25l", "\x1b[2J")


def descendants(pid: int) -> list[int]:
    out = [pid]
    r = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True)
    for line in r.stdout.split():
        out += descendants(int(line))
    return out


def listeners(pid: int):
    """TCP LISTEN ports of pid and its children (a set of strings), or None when this host
    has neither `lsof` nor `/proc/net/tcp` to tell."""
    pids = descendants(pid)
    if shutil.which("lsof"):
        r = subprocess.run(["lsof", "-nP", "-a", "-p", ",".join(map(str, pids)), "-iTCP",
                            "-sTCP:LISTEN", "-Fn"], capture_output=True, text=True)
        return {l.rsplit(":", 1)[-1] for l in r.stdout.splitlines() if l.startswith("n")}
    if os.path.exists("/proc/net/tcp"):
        inodes = set()
        for p in pids:
            try:
                for fd in os.listdir(f"/proc/{p}/fd"):
                    link = os.readlink(os.path.join(f"/proc/{p}/fd", fd))
                    if link.startswith("socket:["):
                        inodes.add(link[8:-1])
            except OSError:
                pass
        ports = set()
        for f in ("/proc/net/tcp", "/proc/net/tcp6"):
            try:
                rows = open(f).read().splitlines()[1:]
            except OSError:
                continue
            for row in rows:
                c = row.split()
                if c[3] == "0A" and c[9] in inodes:  # 0A = LISTEN
                    ports.add(str(int(c[1].rsplit(":", 1)[1], 16)))
        return ports
    return None
