"""The ssh twins of the cli scenarios (#242, #252): one task split across the local kernel (the master)
and one in a Podman box reached over ssh, wired the WEAK way (existing pieces only, no new code in the product).

THE WIRING (fantastic's canvas is the model: `src/tui/src/kern/web.rs`, `user` -> `web` -> the socket
leg, the leg reaching `user`, whose `forward` makes the call):

1. START: the lead runs, through its exec agent `worker_exec`, `start` of
   `ssh ... -L N:127.0.0.1:N team@box 'cd w && exec fantastic_team --headless'` (the job holds the
   tunnel and the remote kernel; `echo $$ > team.pid` keeps its pid). Then one raw
   `ssh ... 'fantastic_team send ...'` makes the remote `web` host on 127.0.0.1:N under `user`, its
   `web_ws` leg `remote_ws` under it (the canvas's edges: `web` mounts the leg, the leg reaches
   `user` alone, `user` admits the leg's `forward`), the remote exec agent `worker_exec` in the
   clone (it may call the master's agents: `<master guid>|*`, send/watch/status/stop), the remote work
   program `w/work.py`, and boots `web`. Then `git clone` of the test's git server inside the box
   (through Podman's host gateway), by the same raw path.
2. BRIDGE: the lead's `create_agent` of a `ws_bridge` agent `remote` on the local kernel, dialling
   the tunnel's local end (`ws://127.0.0.1:N/remote_ws/ws`), as the canvas page dials `canvas_ws`.
   The bridge says `hello` with the master's guid, so the remote reaches the master's agents as
   `<master guid>|<agent>` (fantastic#476). Its leg is open (`allow_all_ingress`) and grants exactly
   send/watch/status/stop on the master's backends that the remote is to call: the bridge's `can_call`
   and each backend's `callable_by` (`grant_leg`, `open_backends`).
3. REMOTE TOOLS: calls on the remote kernel go through the bridge's `forward` (to `user`, whose
   `forward` makes the call on the remote kernel): the remote exec agent's `run`, in the clone.
4. REMOTE WORK: THE BACKENDS ARE ON THE MASTER ONLY, and so is the model's key. The box holds tools
   (the exec agent, git, the clone) and no AI agent: what a remote agent does with a model it asks
   the master's backend for, with `send` and `watch` of `<master guid>|<backend>` over the same
   connection (the remote work program `work.py`, a job of the remote exec agent, the way a program
   and an LLM call are substitutes in fantastic's exec_bridge). The backend's turn runs on the master;
   its `status`, `part_*` and `done` events come back to the program as the bridge's `event` frames.
   The backend's tool calls reach the box's exec agent through the bridge's `forward`.
5. STOP: the bridge deleted, SIGTERM to the remote kernel over ssh with a bounded wait (the binary
   has no stop word), the tunnel job stopped.

The scripted twins (`tests/cli_ssh/`) script those raw commands: `script.json` is written by
`write_scripts()` from the builders below (`python3 -m helpers.ssh_twins --write` in `e2e/`; the
selftest checks the files match), with `{{ssh_port}}`, `{{ssh_key}}`, `{{tunnel_port}}`,
`{{master_guid}}`, `{{remote_git_url}}` bound once the box runs (`ScriptedModel.bind`). The gpu
twins (`tests/gpu_cli_ssh/`) tell the lead in plain words where the box is and what the wiring is
(`task_text`), and leave the commands to the model.

THE CHECKS, after the run (`Box.end`): the test's git server holds a linear history with a commit
made in the local clone and one made in the box's clone (each found in that clone's reflog), and
the fixture is solved there; the bridge is gone, the tunnel's port refuses, no exec job runs; the
remote kernel is stopped; started once more, its records hold the remote base and no AI agent
(no `*_backend`), no spawned agent, no orphan; the box's folder, environment and container hold no
model key; the remote work program's arrival log shows `status` and `part_*` before `done`
(`streamed`: the backend's stream crossed the bridge); then the container is removed (`with Box(...)`).
`streamed` is itself checked against mutations (an unwatched stream, a refused grant, parts only after
`done`), and `cli_ssh/precreated_1_2` also seals the watch grant on the live bridge and breaks the tunnel
(in flight, then redial, then the watch goes on).
"""

from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
import time

from . import cli, folds, podman_remote as pr, records, team as team_mod, web
from .podman_remote import shq

HERE = os.path.dirname(os.path.abspath(__file__))
TESTS = os.path.join(os.path.dirname(HERE), "tests")

BRIDGE = "remote"  # the local ws_bridge
LEG = "remote_ws"  # the remote web_ws leg
WEB = "web"  # the remote HTTP host
WORK_FILE = "work.py"  # the remote work program, in DIR (the exec agent runs in DIR/repo: `../work.py`)
HOME = f"/home/{pr.USER}"  # the box's home: the login's
DIR = "w"  # the remote kernel's folder, in the box's home
REPO = "repo"  # the clone, in DIR
REMOTE_REPO = f"{HOME}/{DIR}/{REPO}"
STOP_WAIT_S = 30  # the bounded wait after SIGTERM

SCENARIOS = ("single_lead", "lead_spawns_two", "precreated_1_2", "precreated_tree", "leads_spawn_leads")
# The twin of #253 (git credentials through the bridge) has a script of its own and no gpu twin.
GIT_TWIN = "git_credentials"
# The twin of #284 (a lead's check-ins: a generating sub-lead is left alone, an idle one is stopped) has no gpu twin either.
BUSY_TWIN = "checkins_busy_stuck"
SCRIPTED = SCENARIOS + (GIT_TWIN, BUSY_TWIN)

# The fixes, one per side: the local side fixes adder.py, the remote side muler.py.
ADD = ("adder.py", "a - b", "a + b")
MUL = ("muler.py", "a + b", "a * b")


# ---- the raw commands ------------------------------------------------------------------

def ssh_args(command: str, tunnel: bool = False) -> list[str]:
    """The argv of a raw ssh to the box (placeholders for the run's port, key and tunnel)."""
    fwd = ("{{tunnel_port}}", "{{tunnel_port}}") if tunnel else None
    return pr.ssh_argv("{{ssh_port}}", "{{ssh_key}}", command, fwd)


def run_call(argv: list[str], wait_s: int = 60, verb: str = "run") -> dict:
    return cli.run_call(argv[0], *argv[1:], wait_s=wait_s, verb=verb)


def ssh_call(command: str, wait_s: int = 60) -> dict:
    return run_call(ssh_args(command), wait_s)


def kernel_start_command() -> str:
    """The remote kernel, in the foreground of the tunnel's ssh session (its pid kept)."""
    return (f"mkdir -p {DIR} && cd {DIR} && echo $$ > team.pid && "
            "exec fantastic_team --headless >> team.log 2>&1")


def detached_kernel_command() -> str:
    """The remote kernel on its own (nohup, its pid kept), for the twin that breaks the tunnel
    without ending the kernel: the tunnel is a job of its own (`tunnel_turn`)."""
    return (f"mkdir -p {DIR} && cd {DIR} && {{ nohup fantastic_team --headless >> team.log 2>&1 & "
            "echo $! > team.pid; }")


def remote_send(item: dict) -> str:
    """`fantastic_team send '<item>'` in the box; the tunnel's port is a JSON number."""
    text = json.dumps(item, separators=(",", ":")).replace('"{{tunnel_port}}"', "{{tunnel_port}}")
    return "fantastic_team send " + shq(text)


def quiet(command: str) -> str:
    """A `send` whose reply the lead does not read (the exit code says whether it ran)."""
    return command + " > /dev/null"


# What the setup prints: the remote kernel's agent ids, one line (its replies would fill the
# lead's context with records it does not read).
LIST_IDS = ("import json,sys; print(\" \".join(sorted(a[\"id\"] for a in "
            "json.load(sys.stdin)[\"result\"][\"agents\"])))")


def kernel_item(payload: dict) -> dict:
    return {"target": "kernel", "payload": payload}


# `user` admits the leg's `forward`: its `callable_by` read, the leg's entry merged, written back
# (`update_agent` replaces a whole map), the way a person edits it with `send` and a pipe.
MERGE_USER = ("import json,sys; a={x[\"id\"]:x for x in json.load(sys.stdin)[\"result\"][\"agents\"]}; "
              "cb=a[\"user\"].get(\"callable_by\") or {}; cb[\"" + LEG + "\"]=[\"forward\",\"send\"]; "
              "sys.stdout.write(json.dumps({\"target\":\"kernel\",\"payload\":{\"type\":\"update_agent\","
              "\"id\":\"user\",\"callable_by\":cb}}))")

# What the remote may do on the master's backends, and what the master lets its leg do on them:
# exactly these four (the backend's `send`, its `watch` of the stream, `status`, `stop`).
BACKEND_VERBS = ["send", "watch", "status", "stop"]

# What the remote may do on the master's credential store: read a ref (`get`), nothing else.
STORE_VERBS = ["get"]

# The box's exec agent: the tools of the remote kernel (it runs in the clone), and the one agent
# there that calls out: the master's agents, as `<master guid>|<agent>`, those four verbs (the
# master's own grants on its backends say which of them answer).
REMOTE_EXEC = {"id": cli.EXEC, "parent_id": "user", "handler_module": "exec_bridge.tools",
               "ingress_rule": "allow_all_ingress", "cwd": REMOTE_REPO, "allowed": [".*"],
               "inherit_env": ["PATH", "LANG", "HOME"],
               "env": {"GIT_CONFIG_NOSYSTEM": "1", "GIT_TERMINAL_PROMPT": "0", "PYTHONDONTWRITEBYTECODE": "1"},
               "can_call": {"$self": "*", "{{master_guid}}|*": BACKEND_VERBS}}

# THE REMOTE WORK PROGRAM, a job of the exec agent (fantastic's connector is put on top of it at the
# box). It is what a remote agent that wants a model does: `watch` the master's backend, `send` it the
# work, and note every event that comes back, in arrival order, one JSON line each, in
# `<label>.events.jsonl`; `<label>.result.json` says how it ended. The backend's turn runs on the
# master: nothing here holds a model, a provider or a key.
#   work.py [--client C] [--until N] [--timeout S] [--no-watch] [--no-send] GUID BACKEND LABEL [TEXT...]
# It waits for N `status` `done` events of the client C (default LABEL), then exits 0; 1 when the
# backend refused (`error` in the reply or in an event), no `done` came in time, or a transport broke.
WORK_PROGRAM = '''
import json, sys, threading

flags = {"client": None, "until": 1, "timeout": 60.0, "watch": True, "send": True}
args = sys.argv[1:]
while args and args[0].startswith("--"):
    f = args.pop(0)
    if f in ("--client", "--until", "--timeout"):
        v = args.pop(0)
        flags[f[2:]] = {"client": str, "until": int, "timeout": float}[f[2:]](v)
    elif f == "--no-watch":
        flags["watch"] = False
    elif f == "--no-send":
        flags["send"] = False
    else:
        sys.exit("work: unknown flag " + f)
guid, backend, label = args[:3]
text = " ".join(args[3:])
client = flags["client"] or label
target = guid + "|" + backend
events = "../" + label + ".events.jsonl"
done = {"n": 0}
refused = []
enough = threading.Event()


def note(p):
    row = {k: p[k] for k in ("type", "phase", "client_id", "kind", "stream_id") if p.get(k) is not None}
    if p.get("error"):
        row["error"] = str(p["error"])[:200]
        row["reason"] = p.get("reason")
        refused.append(row)
    with open(events, "a") as f:
        f.write(json.dumps(row) + "\\n")
    if row.get("error"):
        enough.set()
    elif p.get("type") == "status" and p.get("phase") == "done" and p.get("client_id") == client:
        done["n"] += 1
        if done["n"] >= flags["until"]:
            enough.set()


reply_error = None
if flags["watch"]:
    kernel.watch(target, note)
try:
    if flags["send"]:
        reply = kernel.send(target, {"type": "send", "text": text, "client_id": client})
    else:
        # watch only: a `status` call is the barrier (one link keeps its frames in order, so the
        # watch is served before the answer comes); `<label>.ready` says the watch is in place
        reply = kernel.send(target, {"type": "status"})
        open("../" + label + ".ready", "w").write("ready")
    if isinstance(reply, dict) and reply.get("error"):
        reply_error = str(reply["error"])[:300]
except RuntimeError as e:
    reply_error = str(e)[:300]
if flags["watch"] and not reply_error:
    enough.wait(flags["timeout"])
ok = not reply_error and not refused and (enough.is_set() or not flags["watch"])
result = {"label": label, "backend": backend, "ok": ok, "done": done["n"], "reply_error": reply_error,
          "refused": refused}
with open("../" + label + ".result.json", "w") as f:
    f.write(json.dumps(result))
print(json.dumps(result))
sys.exit(0 if ok else 1)
'''


def work_install_command() -> str:
    """The program into the box's folder: the exec agent's `connector` (fantastic's wire client,
    its `source`) put on top of `WORK_PROGRAM`."""
    body = base64.b64encode(WORK_PROGRAM.encode()).decode()
    code = ('import base64,json,sys; src=json.load(sys.stdin)["result"]["source"]; '
            f'open("{WORK_FILE}","w").write(src+base64.b64decode("{body}").decode())')
    return remote_send({"target": cli.EXEC, "payload": {"type": "connector"}}) + " | python3 -c " + shq(code)


def setup_command() -> str:
    """One raw ssh: wait for the remote kernel's socket, then the web host, its leg and the
    canvas's edges, `user`'s grant for the leg, the remote exec agent (it may call the master's
    agents), the work program, and boot `web` (it mounts the leg and listens on the tunnel's port)."""
    wait = (f"for i in $(seq 150); do [ -f {DIR}/.fantastic/cli/mcp.yaml ] && break; sleep 0.2; done && "
            f"cd {DIR} && [ -f .fantastic/cli/mcp.yaml ]")
    steps = [wait,
             quiet(remote_send(kernel_item({"type": "create_agent", "id": WEB, "parent_id": "user",
                                            "handler_module": "web.tools", "port": "{{tunnel_port}}"}))),
             quiet(remote_send(kernel_item({"type": "create_agent", "id": LEG, "parent_id": WEB,
                                            "handler_module": "web_ws.tools", "ingress_rule": "allow_all_ingress",
                                            "callable_by": {"$self": "*", WEB: ["get_routes"]},
                                            "can_call": {"$self": "*", "user": ["forward"]}}))),
             quiet(remote_send(kernel_item({"type": "update_agent", "id": WEB,
                                            "can_call": {"$self": "*", LEG: ["get_routes"]}}))),
             quiet(remote_send(kernel_item({"type": "list_agents"})) + " | python3 -c " + shq(MERGE_USER)
                   + " | xargs -0 fantastic_team send"),
             quiet(remote_send(kernel_item(dict(REMOTE_EXEC, type="create_agent")))),
             work_install_command(),
             quiet(remote_send({"target": WEB, "payload": {"type": "boot"}})),
             remote_send(kernel_item({"type": "list_agents"})) + " | python3 -c " + shq(LIST_IDS)]
    return " && ".join(steps)


def clone_command() -> str:
    return f"cd {DIR} && git clone -q {{{{remote_git_url}}}} {REPO} && git -C {REPO} log --oneline"


def stop_command() -> str:
    """SIGTERM to the remote kernel and a bounded wait (no stop word)."""
    return pr.stop_line(DIR, "team.pid", STOP_WAIT_S)


# ---- the bridge's calls ----------------------------------------------------------------

def bridge_record(backends=(), stores=()) -> dict:
    """The master's bridge to the leg. The leg is open and may call exactly `BACKEND_VERBS` on
    `backends` (the master's backends the remote is to call; a backend minted later is opened by
    `grant_leg`) and `get` on `stores` (the master's credential stores the remote reads: #253). The
    other end of each grant is the callee's `callable_by` (`open_backends`; the store's record file, `fleets/git_credentials`, for the store)."""
    rec = {"id": BRIDGE, "parent_id": "user", "handler_module": "ws_bridge.tools",
           "ingress_rule": "allow_all_ingress", "transport": "ws", "host": "127.0.0.1",
           "port": "{{tunnel_port}}", "peer_id": LEG}
    if backends or stores:
        rec["can_call"] = {"$self": "*", **{b: list(BACKEND_VERBS) for b in backends},
                           **{st: list(STORE_VERBS) for st in stores}}
    return rec


def forward(target: str, payload: dict, timeout: int = 120) -> dict:
    """A call on the remote kernel: the bridge ships it to the leg, the leg asks `user` to forward it."""
    return {"target": BRIDGE, "type": "forward",
            "args": {"target": "user", "payload": {"type": "forward", "target": target, "payload": payload},
                     "forward_timeout": timeout}}


def remote_run(call: dict) -> dict:
    """The exec agent's `run` of a local call, in the box's clone, through the bridge."""
    a = call["args"]
    return forward(cli.EXEC, {"type": "run", "command": a["command"], "args": a["args"], "wait_s": 30})


def work(backend: str, label: str, text: str, flags=()) -> dict:
    """A remote agent's model work: the work program, a job of the box's exec agent, `send`s `text` to
    the master's `backend` and `watch`es it (`<master guid>|<backend>`, over the bridge). The lead starts
    it with the exec agent's `run`, through the bridge; its reply is the program's output."""
    argv = [f"../{WORK_FILE}", *flags, "{{master_guid}}", backend, label, text]
    return forward(cli.EXEC, {"type": "run", "command": "python3", "args": argv, "wait_s": 60})


def merged(*turns: dict) -> dict:
    """One turn that makes the calls of `turns` together: a lead's kick-off `send` (`reply_to` none: it has no reply to
    wake the lead) rides in the turn of the call that does."""
    return dict(turns[-1], calls=[c for x in turns for c in x["calls"]])


def t(call: dict, lane: str | None = None, after=(), grant=()) -> dict:
    """One turn of a script. `grant`: backends the operator opens to the bridge leg before the turn is
    answered (`grant_leg`: for a backend minted by a spawn, whose id the script cannot name earlier)."""
    turn = {"calls": [call]}
    if lane:
        turn["lane"] = lane
    if after:
        turn["after"] = list(after)
    if grant:
        turn["grant_leg"] = list(grant)
    return turn


def open_turns(backends=(), after=(), detached: bool = False, stores=(), git: bool = False) -> list[dict]:
    """The lead's start: the tunnel and the remote kernel, the remote's surface and exec agent,
    the clone in the box, the bridge. `detached`: the kernel on its own and the tunnel a job of its
    own (the twin that breaks the tunnel and redials). `git`: the box's repository is made without a
    credential (`git_box_command`: no clone, the git connector fetches), and the bridge's leg may read `stores`."""
    if detached:
        first = [t(ssh_call(detached_kernel_command()), after=after), t(tunnel_call())]
    else:
        first = [t(run_call(ssh_args(kernel_start_command(), tunnel=True), verb="start"), after=after)]
    return first + [t(ssh_call(setup_command())),
                    t(ssh_call(git_box_command() if git else clone_command())),
                    t({"target": "kernel", "type": "create_agent", "args": bridge_record(backends, stores)})]


def tunnel_call() -> dict:
    """The tunnel alone, a job of the exec agent (`ssh -N -L`)."""
    return run_call(ssh_args(None, tunnel=True), verb="start")


AWAIT = {"name": "await"}


def stop_turns(detached: bool = False) -> list[dict]:
    """SIGTERM to the remote kernel over ssh. Started in the tunnel's ssh session, the session ends
    with the kernel, so two messages come back, in either order: the stop's reply and the tunnel job's
    `job_done`. The first is answered with `await` (the other is in flight), the second goes on.
    `detached`: the tunnel is a job of its own, stopped after the kernel."""
    if detached:
        return [t(ssh_call(stop_command())), t({"target": cli.EXEC, "type": "stop", "args": {}})]
    return [t(ssh_call(stop_command())), t(AWAIT)]


def close_turns(remote_ids=(), verify: bool = True, detached: bool = False) -> list[dict]:
    """The lead's end: the remote agents it made deleted through the bridge, the bridge deleted, the
    remote kernel stopped over ssh (its tunnel closes with it); then the test run on the pulled tree."""
    turns = [t(forward("kernel", {"type": "delete_agent", "id": i})) for i in remote_ids]
    turns += [t({"target": "kernel", "type": "delete_agent", "args": {"id": BRIDGE}}), *stop_turns(detached)]
    if verify:
        turns += [t(cli.run_call("git", "pull", "-q", "--ff-only")), t(cli.run_call("python3", "test_calc.py"))]
    return turns + [t({"name": "done"})]


def check_ins_open(me: str, lane: str | None = None) -> list[dict]:
    """A lead's own check-ins (`cli.CHECK_INS`): it makes its scheduler and schedules a tick to itself
    (each second, so the scripted run sees one; the plug answers a tick outside the script)."""
    sid = f"{me}_checkins"
    return [t({"target": "kernel", "type": "create_agent", "args": {
                "id": sid, "parent_id": me, "handler_module": "scheduler.tools",
                "can_call": {"$parent": ["send"]}, "callable_by": {"$parent": "*"}}}, lane),
            t({"target": sid, "type": "schedule", "args": {
                "target": me, "payload": {"type": "send", "text": "[check-in agents] check your agents"},
                "interval_seconds": 1}}, lane)]


def check_ins_close(me: str, lane: str | None = None, after=()) -> dict:
    """The lead deletes its scheduler when its task is complete (held a moment, so a tick lands first)."""
    return dict(t({"target": "kernel", "type": "delete_agent", "args": {"id": f"{me}_checkins"}}, lane, after=after),
                delay=2)


# A backend the remote watches opens its first turn with words, so a TEXT part streams to its inbox (a turn
# of calls alone streams call parts, which go to the agents they call, not to a watcher of the backend).
SAY = "Working on it."


def say_first(turns: list[dict], say: str = SAY) -> list[dict]:
    turns[0]["say"] = say
    return turns


def fix_turns(fix, msg: str, lane: str | None, pull: bool = False, after=(), remote: bool = False,
              say: str | None = None, report: str | None = None) -> list[dict]:
    """A worker's turns: (pull,) test, fix, test, commit, push, done. On its own kernel's exec agent;
    `remote`: on the box's, through the bridge (a master backend working in the remote workspace).
    `say`: words before its first call (a backend the remote watches: see `SAY`). `report`: the run ends with
    this plain `send` to its parent instead of `done` (the send blocks until the parent is idle, so the parent
    deletes the worker as soon as it has read it: a worker's `done` after it would never be served)."""
    calls = [cli.run_call("git", "pull", "-q", "--ff-only")] if pull else []
    calls += [cli.run_call("python3", "test_calc.py"), cli.replace_call(*fix), cli.run_call("python3", "test_calc.py"),
              cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                           "commit", "-q", "-a", "-m", msg),
              cli.run_call("git", "push", "-q", "origin", "HEAD:main")]
    if remote:
        calls = [remote_run(c) for c in calls]
    turns = [t(c, lane) for c in calls] + [t(cli.report_call(report) if report else {"name": "done"}, lane)]
    if after:
        turns[0]["after"] = list(after)
    if say:
        turns[0]["say"] = say
    return turns


def history(id: str) -> dict:
    """Results back: a master agent's conversation (a backend's turns are on the master)."""
    return {"target": id, "type": "history", "args": {}}


def remote_log_call() -> dict:
    """Results back from the remote work: the box clone's last commit, through the bridge (the
    remote exec agent's `run`)."""
    return forward(cli.EXEC, {"type": "run", "command": "git", "args": ["log", "-1", "--format=%s"], "wait_s": 30})


# ---- the five scripts -------------------------------------------------------------------

def script_single_lead() -> list[dict]:
    """The lead alone, twice (A: laid as a record file; B: fantastic's recipe). Locally it fixes
    adder.py and pushes; in its remote workspace (the box's exec agent, through the bridge) it
    pulls, fixes muler.py, commits and pushes. The lead is the only backend, on the master. Then the folds
    (`helpers/folds.py`): message 1 ends with a thought and a large reply, message 2 recalls both and closes."""
    def one() -> list[dict]:
        local = [t(c) for c in (cli.run_call("python3", "test_calc.py"), cli.replace_call(*ADD),
                                cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                                             "commit", "-q", "-a", "-m", "Fix add"),
                                cli.run_call("git", "push", "-q", "origin", "HEAD:main"))]
        remote = [t(remote_run(c))
                  for c in (cli.run_call("git", "pull", "-q", "--ff-only"), cli.replace_call(*MUL),
                            cli.run_call("python3", "test_calc.py"),
                            cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                                         "commit", "-q", "-a", "-m", "Fix mul"),
                            cli.run_call("git", "push", "-q", "origin", "HEAD:main"))]
        return open_turns() + local + remote + folds.turns_one() + folds.turns_two() + close_turns()
    return one() + one()


# `precreated_1_2` is also the reconnect twin: the remote kernel on its own and the tunnel a job of its own
# (so the tunnel can be broken and made again with the kernel up), and two messages to the lead with the
# operator's phase between them (`reconnect_phase` in the probe): the lead's turns of each, the turns of
# w1, and of w2 before the phase (the fix) and in it (a slow turn of w3's, a backend kept for it, and a turn of
# the master's on w2).
SLOW_S = 20  # a slow turn of w3's, long enough to break the tunnel while it is in flight


def script_precreated_1_2() -> list[dict]:
    """The lead, w1 and w2 on the master (made before the task). w1 fixes adder.py and pushes. The
    remote agent (the work program in the box) sends w2 its work and watches it; w2's turn runs on
    the master and its tools are the box's exec agent: it pulls, fixes muler.py and pushes (message 1).
    Then the operator seals the watch grant (the work program's watch gets nothing), breaks the tunnel
    while a call to w2 is in flight, redials with the bridge's `reconnect`, and a watcher of w2 started
    before keeps streaming (the phase, in the probe). The lead reads w2's conversation back and
    closes (message 2)."""
    one = open_turns(backends=("w2",), detached=True) + check_ins_open("lead") + [
        t({"target": "w1", "type": "send", "args": {"text": "[lane:w1] Fix adder.py, commit and push."}}),
        t(work("w2", "w2", "[lane:w2] Pull, fix muler.py, commit and push."), after=["w1"]),
        t({"name": "done"}),
    ]
    two = [t(history("w2"), after=["w2"]), check_ins_close("lead")] + close_turns(detached=True)
    # the turns of the phase: each ends its run with `done`, words first (a text part streams). The call that is
    # in flight when the tunnel breaks goes to w1 (slow); the turn of the master's after the redial is w2's
    during = [dict(t({"name": "done"}, "w3"), say="Working on it, slowly.", delay=SLOW_S),
              dict(t({"name": "done"}, "w2"), say="Done, after the redial.")]
    return (one + two + fix_turns(ADD, "Fix add", "w1")
            + fix_turns(MUL, "Fix mul", "w2", pull=True, remote=True, say=SAY) + during)


def phases_1_2() -> dict:
    """How many turns each part of `script_precreated_1_2` has, for the probe to wait for."""
    return {"one": len(open_turns(backends=("w2",), detached=True)) + len(check_ins_open("lead")) + 3,
            "two": 2 + len(close_turns(detached=True)),
            "w1": len(fix_turns(ADD, "", "w1")), "w2": len(fix_turns(MUL, "", "w2", True, remote=True)), "during": 2}


def script_precreated_tree() -> list[dict]:
    """The lead, s1 and w1 under it, s2 and w2 under it: all on the master (made before the task). s1
    has w1 fix adder.py; then the remote agent sends s2 its work and watches it, and s2 has w2 (on the
    master, its tools the box's exec agent) fix muler.py."""
    lead = open_turns(backends=("s2",)) + check_ins_open("lead") + [
        t({"target": "s1", "type": "send", "args": {"text": "[lane:s1] Have your worker fix adder.py."}}),
        t(work("s2", "s2", "[lane:s2] Have your worker w2 fix muler.py."), after=["s1", "w1"]),
        t(history("w2"), after=["s2", "w2"]),
        check_ins_close("lead"),
    ] + close_turns()
    s1 = check_ins_open("s1", "s1") + [
        t({"target": "w1", "type": "send", "args": {"text": "[lane:w1] Fix adder.py, commit and push."}}, "s1"),
        check_ins_close("s1", "s1", after=["w1"]), t({"name": "done"}, "s1")]
    s2 = say_first(check_ins_open("s2", "s2") + [
        t({"target": "w2", "type": "send", "args": {"text": "[lane:w2] Pull, fix muler.py, commit and push."}}, "s2"),
        check_ins_close("s2", "s2", after=["w2"]), t({"name": "done"}, "s2")])
    return (lead + s1 + s2 + fix_turns(ADD, "Fix add", "w1")
            + fix_turns(MUL, "Fix mul", "w2", pull=True, remote=True))


W1_REPORT = "w1 done: adder.py is fixed and pushed"
S1_REPORT = "s1 done: adder.py is fixed and pushed"


def script_lead_spawns_two() -> list[dict]:
    """The lead spawns w1 and w2 on the master's registry. w1 fixes adder.py and pushes and reports to the lead
    with a plain send; the lead deletes it as soon as it has read it. Then the remote
    agent sends w2 its work and watches it (w2 is minted when spawned: the operator opens it to the leg
    when the script reaches that turn); w2's tools are the box's exec agent: it pulls, fixes muler.py
    and pushes. The lead deletes w2 too."""
    lead = open_turns() + [
        t({"target": "bundles", "type": "spawn", "args": {"bundle": "worker"}}),
        merged(t({"target": "{{minted:1}}", "type": "send", "args": {"reply_to": "none", "text": "[lane:w1] Fix adder.py, commit and push."}}),
               t({"target": "bundles", "type": "spawn", "args": {"bundle": "worker"}})),
        *check_ins_open("lead"),
        t(AWAIT),  # w1's report (a plain send) wakes the lead: it deletes w1 at once, then hands w2 its work
        t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}),
        t(work("{{minted:2}}", "w2", "[lane:w2] Pull, fix muler.py, commit and push."), after=["w1"],
          grant=["{{minted:2}}"]),
        t(remote_log_call(), after=["w2"]),
        t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:2}}"}}),
        check_ins_close("lead"),
    ] + close_turns()
    return lead + fix_turns(ADD, "Fix add", "w1", report=W1_REPORT) + fix_turns(MUL, "Fix mul", "w2", pull=True, remote=True, say=SAY)


def script_leads_spawn_leads() -> list[dict]:
    """The lead spawns s1 and s2 on the master's registry. s1 spawns its worker, which fixes adder.py and
    pushes. Then the remote agent sends s2 its work and watches it (s2 is minted: opened to the leg at
    that turn); s2 spawns its worker (on the master; its tools are the box's exec agent), which pulls,
    fixes muler.py and pushes; each sub-lead deletes its worker, the lead deletes both sub-leads."""
    lead = open_turns() + [
        t({"target": "bundles", "type": "spawn", "args": {"bundle": "sublead"}}),
        t({"target": "{{minted:1}}", "type": "send", "args": {"text": "[lane:s1] Have a worker fix adder.py, then delete it."}}),
        t({"target": "bundles", "type": "spawn", "args": {"bundle": "sublead"}}),
        *check_ins_open("lead"),
        t(work("{{minted:2}}", "s2", "[lane:s2] Have a worker pull, fix muler.py and push, then delete it."),
          after=["s1", "w1"], grant=["{{minted:2}}"]),
        t(remote_log_call(), after=["s2", "w2"]),
        t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}),
        t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:2}}"}}),
        check_ins_close("lead"),
    ] + close_turns()
    s1 = [t({"target": "bundles", "type": "spawn", "args": {"bundle": "worker"}}, "s1"),
          t({"target": "{{minted:1}}", "type": "send", "args": {"text": "[lane:w1] Fix adder.py, commit and push."}}, "s1"),
          t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}, "s1", after=["w1"]),
          t({"name": "done"}, "s1")]
    s2 = say_first([t({"target": "bundles", "type": "spawn", "args": {"bundle": "worker"}}, "s2"),
                    t({"target": "{{minted:1}}", "type": "send", "args": {"text": "[lane:w2] Pull, fix muler.py, commit and push."}}, "s2"),
                    t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}, "s2", after=["w2"]),
                    t({"name": "done"}, "s2")])
    return (lead + s1 + s2 + fix_turns(ADD, "Fix add", "w1")
            + fix_turns(MUL, "Fix mul", "w2", pull=True, remote=True))


BUSY_S = 8  # the slow sub-lead's first turn: it stays generating across the lead's two check-ins
STUCK_SAY = "task failed: the second sub-lead was not generating and added no row in two check-ins in a row; I stopped it."


def script_checkins_busy_stuck() -> list[dict]:
    """The lead fixes muler.py in the box itself (through the bridge) and pushes; then it spawns s1 and s2 on
    the master's registry. s1 pulls, fixes adder.py, pushes and reports with a plain send, after a slow first
    turn (it is generating the whole time); s2 awaits with nothing in flight (its run ends: idle, no new row).
    The lead's two check-ins (its own scripted turns; a tick is a notice the plug answers outside the script)
    read each sub-lead's `status` and `history`: s1 is generating both times and is left alone, s2 is not and
    adds no row both times, so the lead stops it; it waits for s1's report, deletes both and says `task failed:`."""
    sub1, sub2 = "{{minted:1}}", "{{minted:2}}"
    ask = lambda to, verb: t({"target": to, "type": verb, "args": {}})  # noqa: E731
    checks = []
    for n in range(2):
        checks += [dict(ask(sub1, "status"), delay=2), ask(sub2, "status"), ask(sub2, "history")]
    remote = [t(remote_run(c)) for c in (
        cli.run_call("git", "pull", "-q", "--ff-only"), cli.replace_call(*MUL), cli.run_call("python3", "test_calc.py"),
        cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid", "commit", "-q", "-a",
                     "-m", "Fix mul"),
        cli.run_call("git", "push", "-q", "origin", "HEAD:main"))]
    close = close_turns()
    close[-1] = dict(close[-1], say=STUCK_SAY)
    lead = open_turns() + remote + [
        t({"target": "bundles", "type": "spawn", "args": {"bundle": "sublead"}}),
        t({"target": "bundles", "type": "spawn", "args": {"bundle": "sublead"}}),
        merged(t({"target": "{{minted:1}}", "type": "send", "args": {"reply_to": "none", "text": "[lane:s1] Fix adder.py and push; it takes a while."}}),
               t({"target": "{{minted:2}}", "type": "send", "args": {"reply_to": "none", "text": "[lane:s2] Wait for a message that never comes."}}),
               check_ins_open("lead")[0]),
        check_ins_open("lead")[1], *checks, ask(sub2, "stop"),
        t(AWAIT),  # s1's report wakes the lead
        t({"target": "kernel", "type": "delete_agent", "args": {"id": sub1}}),
        t({"target": "kernel", "type": "delete_agent", "args": {"id": sub2}}),
        check_ins_close("lead"),
    ] + close
    s1 = fix_turns(ADD, "Fix add", "s1", pull=True, report=S1_REPORT)
    s1[1]["delay"] = BUSY_S  # the 2nd turn: the lead's send waits for the child's first answer, so the slow turn is the next one
    return lead + s1 + [t(AWAIT, "s2")]


# ---- the git twin (#253): the remote's git credentials come through the bridge --------------

GIT = "git"  # the git connector (`team_git.tools`) on the remote kernel
GIT_STORE = "git_credentials"  # the master's credential store: the forge's token, and nothing else
GIT_REF = "git/forge"  # its ref, bound to the forge's host
GIT_ENV = "E2E_GIT_TOKEN"  # where the master's store reads the token from (its `env` backend: no value at rest)
WT_ROOT = f"{HOME}/{DIR}/wt"  # every worktree of the connector is inside
WORKTREE = f"{WT_ROOT}/muler"
BRANCH = "work/muler"
REVOKED = "work/revoked"  # pushed with the grant revoked (refused), then again after it is given back
CLOSED = "work/closed"  # pushed after the bridge is deleted: refused
AUTHOR = ("Remote Worker", "remote@example.invalid")

# The box reaches the test's git server on its OWN 127.0.0.1: the connector sends a credential only over https, or
# over http to loopback, and the box dials the server through Podman's gateway (not loopback). This relay
# (a job of the box, `nohup`) listens on 127.0.0.1:PORT in the box and carries the bytes to the gateway; it is the
# stand-in for the TLS a real forge has, and it sees the wire as any hop would.
RELAY = "\nimport socket, sys, threading\n\n" + web.PIPE + """lport, host, port = int(sys.argv[1]), sys.argv[2], int(sys.argv[3])
srv = socket.socket()
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", lport))
srv.listen(16)
while True:
    c, _ = srv.accept()
    u = socket.create_connection((host, port))
    threading.Thread(target=pipe, args=(c, u), daemon=True).start()
    threading.Thread(target=pipe, args=(u, c), daemon=True).start()
"""


def git_box_command() -> str:
    """The box's repository, with no credential on it: the relay, then `git init` and the forge as `origin`
    (no clone: the connector has no clone verb; its `fetch` brings the objects, with the token it reads from
    the master's store at that moment)."""
    body = base64.b64encode(RELAY.encode()).decode()
    boot = f'import base64; exec(base64.b64decode("{body}"))'
    return (f"cd {DIR} && mkdir -p wt && {{ nohup python3 -c {shq(boot)} {{{{forge_port}}}} {pr.GATEWAY} "
            "{{forge_port}} > relay.log 2>&1 < /dev/null & echo $! > relay.pid; } && "
            f"git init -q -b main {REPO} && git -C {REPO} remote add origin {{{{remote_git_url}}}}")


def remote_git_record() -> dict:
    """The git connector on the REMOTE kernel. Its `credentials` name the store on the MASTER, `<master guid>|git_credentials`
    (reached over the bridge, per operation); the remote may call that store's `get` and nothing else of it."""
    store = "{{master_guid}}|" + GIT_STORE
    return {"type": "create_agent", "id": GIT, "parent_id": "user", "handler_module": "team_git.tools",
            "ingress_rule": "allow_all_ingress", "repo_dir": REMOTE_REPO, "workspace_root": WT_ROOT,
            "remote": "origin", "default_branch": "main", "credentials": {"store": store, "ref": GIT_REF},
            "author_name": AUTHOR[0], "author_email": AUTHOR[1],
            "can_call": {"$self": "*", store: list(STORE_VERBS)}}


def git_call(verb: str, **args) -> dict:
    """A verb of the remote git connector, through the bridge."""
    return forward(GIT, {"type": verb, **args})


def raw_git(verb: str, **args) -> dict:
    """The same verb by the box's own `send` over ssh (no bridge: the lead's way in when the leg is gone)."""
    return ssh_call(f"cd {DIR} && " + remote_send({"target": GIT, "payload": {"type": verb, **args}}))


def branch_call(name: str) -> dict:
    return remote_run(cli.run_call("git", "branch", name, "origin/main"))


def set_store_grant(grant: dict) -> dict:
    """The master's end of the leg's read of the store: its `callable_by` (a whole map, never empty: `$self` alone is
    the revoked state, the leg's entry is the grant)."""
    return {"target": "kernel", "type": "update_agent", "args": {"id": GIT_STORE, "callable_by": grant}}


def script_git_credentials() -> list[dict]:
    """One lead, three messages. 1 (setup): the box and the bridge (the leg may `get` from the master's store), the git
    connector on the remote kernel naming that store. 2 (the work,
    ON THE REMOTE): fetch, a worktree of origin/main, the fix to muler.py, commit, fetch, push a work branch. Then the
    operator's phase (the probe looks for the token on both sides). 3: the store's grant revoked, a push over the open
    bridge (refused), the grant given back, the push again (lands); the bridge deleted, a push by the box's own `send`
    (refused); the remote stopped."""
    setup = open_turns(stores=(GIT_STORE,), git=True) + [
        t(ssh_call(f"cd {DIR} && " + remote_send(kernel_item(remote_git_record())))),
        t({"name": "done"})]
    work = [t(git_call("fetch")),
            t(git_call("worktree_add", path=WORKTREE, branch=BRANCH, base="origin/main")),
            t(remote_run(cli.replace_call(f"{WORKTREE}/muler.py", "a + b", "a * b"))),
            t(git_call("commit", path=WORKTREE, message="Fix mul")),
            t(git_call("fetch")),
            t(git_call("push", path=WORKTREE, branch=BRANCH)),
            t({"name": "done"})]
    close = [t(branch_call(REVOKED)), t(branch_call(CLOSED)),
             t(set_store_grant({"$self": "*"})),
             t(git_call("push", path=WORKTREE, branch=REVOKED)),
             t(set_store_grant({BRIDGE: list(STORE_VERBS)})),
             t(git_call("push", path=WORKTREE, branch=REVOKED)),
             t({"target": "kernel", "type": "delete_agent", "args": {"id": BRIDGE}}),
             t(raw_git("push", path=WORKTREE, branch=CLOSED)),
             *stop_turns(), t({"name": "done"})]
    return setup + work + close


def phases_git() -> dict:
    """How many turns each message of `script_git_credentials` has (the probe waits for them)."""
    setup = len(open_turns(stores=(GIT_STORE,), git=True)) + 2
    return {"setup": setup, "work": 7, "close": len(script_git_credentials()) - setup - 7}


SCRIPTS = {GIT_TWIN: script_git_credentials, BUSY_TWIN: script_checkins_busy_stuck, "single_lead": script_single_lead, "precreated_1_2": script_precreated_1_2,
           "precreated_tree": script_precreated_tree, "lead_spawns_two": script_lead_spawns_two,
           "leads_spawn_leads": script_leads_spawn_leads}


def script_path(name: str) -> str:
    return os.path.join(TESTS, "cli_ssh", name, "script.json")


def render_script(name: str) -> str:
    return json.dumps(SCRIPTS[name](), indent=1) + "\n"


def write_scripts() -> None:
    for name in SCRIPTED:
        with open(script_path(name), "w") as f:
            f.write(render_script(name))


def stale_scripts() -> list[str]:
    """The cli_ssh scripts that differ from what the builders write (empty when in sync)."""
    out = []
    for name in SCRIPTED:
        try:
            with open(script_path(name)) as f:
                if f.read() != render_script(name):
                    out.append(name)
        except OSError:
            out.append(name)
    return out


# ---- the leg's grants on the master's backends --------------------------------------------

def master_guid(team) -> str:
    """The master kernel's guid, the address of its agents on the remote (`<guid>|<agent>`): the root's reflect."""
    r = team.send("kernel", {"type": "reflect"}).get("result") or {}
    guid = r.get("guid")
    if not guid:
        raise RuntimeError(f"the master kernel reports no guid: {str(r)[:300]}")
    return guid


def open_backend(team, backend: str) -> None:
    """The backend's end of the leg's grant: `callable_by` the bridge, exactly `BACKEND_VERBS`."""
    records.merge(team, backend, "callable_by", {BRIDGE: list(BACKEND_VERBS)})


def grant_leg(team, backends) -> None:
    """Open master backends to the leg, both ends of each grant (the way an operator would): the
    bridge's `can_call` and the backend's `callable_by`, exactly `BACKEND_VERBS`, the entries each
    already holds kept."""
    for b in backends:
        records.merge(team, BRIDGE, "can_call", {b: list(BACKEND_VERBS)})
        open_backend(team, b)


def seal_watch(team, backend: str) -> None:
    """The mutation: the backend stops letting the leg `watch` it (send, status and stop stay)."""
    records.merge(team, backend, "callable_by", {BRIDGE: [v for v in BACKEND_VERBS if v != "watch"]})


# ---- what the box saw: the stream, and the key it must not hold ---------------------------

def streamed(rows: list[dict], client: str) -> tuple[bool, str]:
    """Did a backend's stream reach the remote program? `rows` are its arrival log (one dict per event,
    in the order they came). Good when, for the events of `client`, a `status` came first and at least
    one `part_*` before the first `status` `done`, and nothing was refused. (False, why) otherwise."""
    refused = [r for r in rows if r.get("error")]
    if refused:
        return False, f"refused: {refused[0]}"
    mine = [r for r in rows if r.get("client_id") == client]
    done = next((i for i, r in enumerate(mine) if r.get("type") == "status" and r.get("phase") == "done"), None)
    if done is None:
        return False, f"no `status` `done` for {client!r} in {len(rows)} event(s): {[r.get('type') for r in rows]}"
    before = [r.get("type") for r in mine[:done]]
    if "status" not in before:
        return False, f"no `status` before `done`: {before}"
    if not any(str(x).startswith("part_") for x in before):
        return False, f"no `part_*` before `done`: {before}"
    return True, ""


# ---- the harness: the box, the fixture on the git server, the checks -------------------

def git_url(ctx) -> str:
    """The test's git server (the fake forge's smart HTTP) as this host dials it."""
    from .forge import REPO as FORGE_REPO
    return f"{ctx.forge.url}/{FORGE_REPO}.git"


def auth_header(ctx) -> str:
    return "Authorization: Basic " + base64.b64encode(f"x-access-token:{ctx.forge.token}".encode()).decode()


def gitconfig(url: str, header: str) -> str:
    """A git config that sends the forge's token to that URL only (it stays out of every command)."""
    base = url.rsplit("/", 2)[0] + "/"
    return f'[http "{base}"]\n\textraHeader = {header}\n'


class Box:
    """One container for one twin: `with Box(ctx) as box:` starts it and always removes it. The
    fixture is the test's git server's (`publish`); the clones carry the forge's token in a git
    config (the local sandbox's, the box's `~/.gitconfig`), never in a command. `git_token=False` (the
    twin of #253): the box is given no token at all, and the master's sandbox none either: the git
    server's token is read from the master's credential store, through the bridge, per operation."""

    def __init__(self, ctx, name: str = "box", git_token: bool = True) -> None:
        self.ctx = ctx
        self.git_token = git_token
        self.remote = pr.Remote(os.path.join(ctx.run_dir, name))
        self.tunnel_port = team_mod.free_port()
        self.stock: set[str] = set()

    def __enter__(self) -> "Box":
        self.remote.__enter__()
        try:
            self.lay_stock()
            if self.git_token:
                self.write_box_gitconfig()
            self.stock_records = self.records(f"{DIR}0")  # a remote kernel on the stock records: base's stock agents and the ones its boot makes
            self.stock = set(self.stock_records)
        except BaseException:
            self.remote.stop()
            raise
        return self

    def __exit__(self, *_):
        self.remote.stop()

    def lay_stock(self) -> None:
        """Base's stock records (the `stock` layer of `e2e/fleets/`) into `~/w/.fantastic` and `~/w0/.fantastic`
        of the box, before any kernel starts there: each kernel loads them at its first start, the team's seed
        does not run, and the remote folder holds base's stock agents and what the lead creates, nothing else.
        One file per write, through ssh (copy only: no record is read)."""
        stock = os.path.join(cli.FLEETS, "stock")
        for folder in (DIR, f"{DIR}0"):
            for rel, text in team_mod.fleet_files([stock], {}):
                path = f"{folder}/.fantastic/{rel}"
                r = subprocess.run(self.remote.argv(f"mkdir -p {shq(os.path.dirname(path))} && cat > {shq(path)}"),
                                   input=text, capture_output=True, text=True, timeout=60)
                if r.returncode != 0:
                    raise pr.RemoteError(f"box stock record {path}: {r.stderr[-400:]}")

    def write_box_gitconfig(self) -> None:
        text = gitconfig(pr.gateway_url(git_url(self.ctx)), auth_header(self.ctx))
        r = subprocess.run(self.remote.argv("cat > .gitconfig && chmod 600 .gitconfig"), input=text,
                           capture_output=True, text=True, timeout=60)
        if r.returncode != 0:
            raise pr.RemoteError(f"box git config: {r.stderr[-400:]}")

    def publish(self) -> None:
        """The fixture (`ctx.repo`, made by `cli.start`) becomes the git server's `main`, and the
        local clone tracks it there; the local sandbox's git config carries the token."""
        ctx = self.ctx
        bare = os.path.join(ctx.run_dir, "forge", *git_url(ctx).rsplit("/", 2)[-2:])
        cli.git(ctx.repo, "push", "-q", "--force", bare, "main")
        cli.git(ctx.repo, "remote", "add", "origin", git_url(ctx))
        if self.git_token:
            with open(os.path.join(ctx.work, "sandbox", ".gitconfig"), "w") as f:
                f.write(gitconfig(git_url(ctx), auth_header(ctx)))
        cli.git(ctx.repo, "-c", "http.extraHeader=" + auth_header(ctx), "fetch", "-q", "origin")
        cli.git(ctx.repo, "branch", "-q", "-u", "origin/main", "main")

    def values(self) -> dict:
        """The script's run-time values: the box's ssh port and key, the tunnel's port, the master's
        guid (the address of its agents on the remote: `<guid>|<agent>`) and the git server as the box
        dials it (Podman's host gateway). The box is given no model endpoint: it holds no model."""
        vals = {"ssh_port": str(self.remote.port), "ssh_key": self.remote.key, "tunnel_port": str(self.tunnel_port),
                "master_guid": master_guid(self.ctx.team),
                "remote_git_url": pr.gateway_url(git_url(self.ctx))}
        if not self.git_token:  # the box's own 127.0.0.1 (its relay, `RELAY`): the connector sends a credential to loopback
            vals["remote_git_url"] = git_url(self.ctx)
            vals["forge_port"] = str(self.ctx.forge.port)
        return vals

    def bind(self, keep: tuple = ()) -> None:
        """Fill the script's values, and let the plug's `grant_leg` turns open a minted backend to the leg.
        `keep`: names a later `bind` fills (the folds' row ids)."""
        self.ctx.model.leg_grant = lambda ids: grant_leg(self.ctx.team, ids)
        self.ctx.model.bind(keep=keep, **self.values())

    def open_backends(self, ids) -> None:
        """The master's end of the leg's grants on backends that exist before the task (the bridge's
        `can_call` end is in its record: `bridge_record`)."""
        for b in ids:
            open_backend(self.ctx.team, b)

    # -- the remote kernel, by the harness (the checks only) --

    def records(self, folder: str = DIR) -> dict:
        """Start the remote kernel in `folder` over ssh, list its records, stop it (SIGTERM, a
        bounded wait). id -> record."""
        start = (f"mkdir -p {folder} && cd {folder} && {{ nohup fantastic_team --headless >> check.log 2>&1 & "
                 "echo $! > check.pid; } && for i in $(seq 150); do [ -f .fantastic/cli/mcp.yaml ] && break; sleep 0.2; done && "
                 + remote_send(kernel_item({"type": "list_agents"})))
        try:
            out = self.remote.ssh(start, timeout=120).stdout
        finally:
            self.remote.ssh(pr.stop_line(folder, "check.pid", STOP_WAIT_S), check=False)
        return {a["id"]: a for a in json.loads(out)["result"]["agents"]}

    def remote_log(self) -> str:
        return self.remote.ssh(f"cat {DIR}/team.log", check=False).stdout

    def reflog_commits(self, remote: bool) -> list[str]:
        cmd = ["reflog", "--format=%H %gs"]
        if remote:
            out = self.remote.ssh(f"git -C {DIR}/{REPO} " + " ".join(shq(c) for c in cmd), check=False).stdout
        else:
            out = cli.git(self.ctx.repo, *cmd, check=False).stdout
        # a commit, or one a rebase replayed (`pull --rebase … (pick): …`: a push rejected, then rebased)
        return [line.split()[0] for line in out.splitlines()
                if (" commit" in line or "(pick):" in line) and "commit (initial)" not in line]

    # -- the checks --

    def end(self, v, remote_base=(), label: str = "", solved: bool = True, streams=(), landed: bool = True) -> dict:
        """The checks every twin shares. `remote_base`: the agents the lead keeps on the remote kernel
        beyond its stock (the surface and the exec agent are always there). `solved`: the fixture
        is the task. `landed`: one commit from each
        side is on the git server (the git twin pushes work branches and checks them itself). `streams`: the
        `(label, client)` of each remote work run whose arrival log must show the stream. Returns the
        remote records; the git server's files at HEAD are in `self.server_files`."""
        ctx = self.ctx
        with tempfile.TemporaryDirectory() as tmp:
            server = os.path.join(tmp, "server")
            bare = os.path.join(ctx.run_dir, "forge", *git_url(ctx).rsplit("/", 2)[-2:])
            subprocess.run(["git", "clone", "-q", bare, server], check=True, capture_output=True)
            if solved:
                ok, why = cli.solved(server)
                v.check(f"{label}solved on the test's git server (the test passes on its HEAD, the fix is committed)",
                        ok, why)
            self.server_files = {}
            for name in os.listdir(server):
                if os.path.isfile(os.path.join(server, name)):
                    with open(os.path.join(server, name), errors="replace") as f:
                        self.server_files[name] = f.read()
            log = cli.git(server, "log", "--format=%H %s").stdout.split("\n")
            merges = cli.git(server, "rev-list", "--merges", "HEAD").stdout.strip()
        on_server = [line.split()[0] for line in log if line.strip()]
        local = [c for c in self.reflog_commits(False) if c in on_server]
        remote = [c for c in self.reflog_commits(True) if c in on_server]
        if landed:
            v.check(f"{label}both sides' commits landed on the git server, one after the other (linear history)",
                    bool(local) and bool(remote) and not set(local) & set(remote) and not merges,
                    f"local {local} remote {remote} merges {merges!r} server {log[:5]}")
        agents = ctx.team.agents()
        v.check(f"{label}the bridge is deleted", BRIDGE not in agents, str(sorted(agents)))
        v.check(f"{label}the tunnel is closed (its local port refuses)", web.refuses(self.tunnel_port))
        jobs = (ctx.team.send(cli.EXEC, {"type": "status"}).get("result") or {}).get("jobs") or []
        v.check(f"{label}no exec job runs on the local kernel", not [j for j in jobs if j.get("status") == "running"],
                str(jobs)[:300])
        alive = self.remote.ssh(f"cd {DIR} && kill -0 $(cat team.pid) 2>/dev/null", check=False).returncode == 0
        v.check(f"{label}the remote kernel is stopped", not alive)
        log_text = self.remote_log()
        v.check(f"{label}the remote kernel served the bridge: its leg asked `user` to forward",
                ("user", "forward") in team_mod.log_calls(log_text), log_text[-300:])
        recs = self.records()
        want = self.stock | {WEB, LEG, cli.EXEC} | set(remote_base)
        v.check(f"{label}remote: the records are its stock, the surface, the exec agent{' and ' + ', '.join(remote_base) if remote_base else ''}"
                " (every remote agent deleted)", set(recs) == want,
                f"extra {sorted(set(recs) - want)} missing {sorted(want - set(recs))}")
        left = [i for i, a in recs.items() if a.get("spawned_by")]
        v.check(f"{label}remote: no spawned agent is left", not left, str(left))
        orphans = [i for i, a in recs.items() if a.get("parent_id") and a["parent_id"] not in recs]
        v.check(f"{label}remote: no orphans (every agent's parent exists)", not orphans, str(orphans))
        backends = {i: a.get("handler_module") for i, a in recs.items() if "backend" in (a.get("handler_module") or "")}
        v.check(f"{label}remote: no backend on the box (its records hold none; the selftest checks that no script or setup "
                "command makes one)", not backends, str(backends))
        for work_label, client in streams:
            self.check_stream(v, work_label, client, label)
        self.remote_log_text = log_text
        return recs

    # -- the operator's calls (the reconnect phase): no lead, no model --

    def remote_call(self, target: str, payload: dict, timeout: int = 60) -> dict:
        """A call on the remote kernel the operator makes through the bridge (the lead's `forward`, without the lead)."""
        r = self.ctx.team.send(BRIDGE, {"type": "forward", "target": "user", "forward_timeout": timeout,
                                        "payload": {"type": "forward", "target": target, "payload": payload}},
                               timeout=timeout + 30)
        return r.get("result") or {}

    def wait_remote_file(self, name: str, wait_s: int = 60) -> bool:
        """True once a file of the remote kernel's folder exists (a bounded wait in the box, as the others)."""
        r = self.remote.ssh(f"for i in $(seq {wait_s * 5}); do [ -e {DIR}/{shq(name)} ] && exit 0; sleep 0.2; done; exit 1",
                            check=False, timeout=wait_s + 30)
        return r.returncode == 0

    def tunnel_argv(self) -> list[str]:
        return pr.ssh_argv(self.remote.port, self.remote.key, None, (self.tunnel_port, self.tunnel_port))

    # -- what the box holds and saw --

    def remote_text(self, name: str) -> str:
        """A file of the remote kernel's folder (empty when it is not there)."""
        return self.remote.ssh(f"cat {DIR}/{shq(name)} 2>/dev/null", check=False).stdout

    def events(self, work_label: str) -> list[dict]:
        """The arrival log of a remote work run: one dict per event, in the order they came."""
        return [json.loads(line) for line in self.remote_text(f"{work_label}.events.jsonl").splitlines() if line.strip()]

    def check_stream(self, v, work_label: str, client: str, label: str = "") -> None:
        rows = self.events(work_label)
        ok, why = streamed(rows, client)
        v.check(f"{label}remote: the master backend's stream reached the box, `status` and `part_*` before "
                f"`done` ({work_label}: {len(rows)} events)", ok, why)

    def removed(self, v, label: str = "") -> None:
        v.check(f"{label}the container is removed", not self.remote.exists())


def hand(ctx, lead: str, text: str, upto: int | None = None, wait_s: float = 240) -> bool:
    """Hand the lead a message; wait until the plug has served `upto` turns (default: all)."""
    r = ctx.team.send(lead, {"type": "send", "text": text})
    served = cli.wait_served(ctx, wait_s, upto)
    dump_requests(ctx)
    return r.get("status") == "completed" and served


def last_messages(ctx):
    """`(lane, text)` of the last message of each request the plug saw (the reply of the call the model made last)."""
    for at, raw in zip(ctx.model.request_lanes, ctx.model.raw_bodies):
        try:
            last = json.loads(raw)["messages"][-1]["content"]
        except (ValueError, KeyError, IndexError, TypeError):
            continue
        yield at, last if isinstance(last, str) else json.dumps(last)


def came_back(ctx, lane: str | None, evidence: str, via: str | None = None) -> bool:
    """True when a request of the lead (the main lane) ends with the reply of a call (`via`: the
    bridge's `forward` by default, else e.g. `w2 history`) that holds `evidence` and, when `lane` is
    given, the work of that lane (`[lane:L]`: a conversation)."""
    head = f"· {via or BRIDGE + ' forward'} ·"
    return any(at == "main" and head in text and (lane is None or f"[lane:{lane}]" in text) and evidence in text
               for at, text in last_messages(ctx))


def dump_requests(ctx) -> None:
    """The plug's requests (lane, body) in the run folder, to read a failed run by."""
    with open(os.path.join(ctx.run_dir, "model-requests.json"), "w") as f:
        json.dump([{"lane": l, "body": b} for l, b in zip(ctx.model.request_lanes, ctx.model.raw_bodies)], f, indent=1)


def forward_replies(ctx) -> list[str]:
    """The lead's requests that answer a `forward` of the bridge (their last message)."""
    return [text for at, text in last_messages(ctx) if at == "main" and f"· {BRIDGE} forward ·" in text]


# ---- the reconnect twin: the operator's phase between the lead's two messages ---------------

def wait_lanes(ctx, want: dict[str, int], wait_s: float = 240) -> bool:
    """Wait until the plug has served each lane `want` turns (the cursors; no turn is served twice)."""
    end = time.time() + wait_s
    while time.time() < end:
        if all(ctx.model.cursor.get(lane, 0) >= n for lane, n in want.items()):
            return True
        time.sleep(0.3)
    return False


def reconnect_phase(ctx, box: Box, v, backend: str = "w2", slow: str = "w3") -> None:
    """After message 1 (the remote work done and its stream seen), with the lead idle:
    - MUTATION: the watch grant sealed on `backend` on the master: the work program's watch gets nothing (the
      master refuses the watch, and the refusal is not sent on to the watcher), it exits 1;
    - a watcher of `backend` is started on the box (the work program, watch only);
    - the tunnel broken (its job stopped) while a call to `slow` is IN FLIGHT: that call fails `bridge_down`,
      on the box and at once. (The call goes to another backend than the watched one: an in-flight `send`
      aborted by the drop leaves its backend `thinking`, fantastic's `fantastic-bridge/src/lib.rs` read loop
      `h.abort()`, so that backend takes no more turns.);
    - the master REDIALS the known port (the bridge's `reconnect`: a ws bridge does not redial by itself), the
      next call on the remote kernel through the bridge answers, and the WATCH made before the break goes
      on: a turn of the master's on `backend` reaches the watcher, which made no new `watch`."""
    team = ctx.team
    guid = master_guid(team)
    work_py = f"../{WORK_FILE}"
    start = lambda *argv: box.remote_call(cli.EXEC, {"type": "start", "command": "python3", "args": [work_py, *argv]})

    seal_watch(team, backend)
    run = box.remote_call(cli.EXEC, {"type": "run", "command": "python3", "wait_s": 40, "args": [
        work_py, "--no-send", "--client", "sealed", "--timeout", "5", guid, backend, "sealed"]}, timeout=60)
    open_backend(team, backend)
    sealed = json.loads(box.remote_text("sealed.result.json") or "{}")
    ok, why = streamed(box.events("sealed"), "sealed")
    v.check("mutation: with the watch grant sealed on the master the box's watch gets nothing (the master refuses it and "
            "fantastic does not tell the watcher): the work program exits 1, no event arrived, the arrival check fails",
            run.get("exit_code") == 1 and not sealed.get("ok") and sealed.get("done") == 0 and not ok,
            f"{str(run)[:200]} {sealed} {why}")

    start("--no-send", "--client", "master", "--timeout", "120", guid, backend, "b")
    v.check("a watcher of the backend on the box is in place (the program's `status` barrier answered over the bridge)",
            box.wait_remote_file("b.ready"), box.remote_text("b.result.json")[:300])
    grant_leg(team, [slow])
    before = ctx.model.served
    start("--timeout", "120", guid, slow, "a", f"[lane:{slow}] Report slowly.")
    waited = time.time() + 60
    while time.time() < waited and ctx.model.served < before + 1:
        time.sleep(0.2)
    v.check("a call to a backend is in flight (its slow turn is on the master)", ctx.model.served == before + 1,
            f"served {ctx.model.served}, was {before}")

    jobs = (team.send(cli.EXEC, {"type": "status"}).get("result") or {}).get("jobs") or []
    tunnel = [j for j in jobs if j.get("status") == "running" and "-N" in (j.get("args") or [])]
    stopped = team.send(cli.EXEC, {"type": "stop", "job_id": tunnel[0]["job_id"]}) if tunnel else {}
    v.check("the tunnel's job is stopped", len(tunnel) == 1 and stopped.get("status") == "completed", str(jobs)[:300])
    got = box.wait_remote_file("a.result.json")
    failed = json.loads(box.remote_text("a.result.json") or "{}")
    v.check("the call in flight fails `bridge_down` on the box at once, not after the slow turn",
            got and not failed.get("ok") and "bridge_down" in (failed.get("reply_error") or ""), str(failed))

    job = team.send(cli.EXEC, {"type": "start", "command": box.tunnel_argv()[0], "args": box.tunnel_argv()[1:]})
    up = team.wait_for(lambda: not web.refuses(box.tunnel_port), 30)
    redial = team.send(BRIDGE, {"type": "reconnect"})
    v.check("the tunnel is made again and the master redials the known port (the bridge's `reconnect`)",
            job.get("status") == "completed" and bool(up) and (redial.get("result") or {}).get("error") is None
            and redial.get("status") == "completed", f"{str(job)[:150]} {str(redial)[:250]}")
    after = box.remote_call("kernel", {"type": "list_agents"})
    ids = {a.get("id") for a in (after.get("agents") or (after.get("result") or {}).get("agents") or [])}
    v.check("the next call on the remote kernel through the bridge answers", {WEB, LEG, cli.EXEC} <= ids, str(after)[:300])

    sent = team.send(backend, {"type": "send", "text": f"[lane:{backend}] Say you are back.", "client_id": "master"},
                     timeout=60)
    v.check("a turn of the master's on the watched backend ran after the redial", sent.get("status") == "completed",
            str(sent)[:300])
    ended = box.wait_remote_file("b.result.json", 90)
    watcher = json.loads(box.remote_text("b.result.json") or "{}")
    ok, why = streamed(box.events("b"), "master")
    v.check("the watch made before the break went on with no new `watch`: the master's turn streamed to the box "
            "(status and part_* before done)", ended and watcher.get("ok") and ok, f"{watcher} {why}")


def remote_agents(reply) -> dict:
    """The remote kernel's records out of a forwarded `list_agents` (the reply may be wrapped)."""
    r = reply or {}
    for _ in range(3):
        if isinstance(r, dict) and isinstance(r.get("agents"), list):
            # a record, or a bare id (a reply rendered short): either names an agent
            return {(a.get("id") if isinstance(a, dict) else a): (a if isinstance(a, dict) else {})
                    for a in r["agents"]}
        r = r.get("result") if isinstance(r, dict) else None
    return {}


if __name__ == "__main__":
    if sys.argv[1:] == ["--write"]:
        write_scripts()
        print("wrote " + ", ".join(script_path(n) for n in SCRIPTED))
        sys.exit(0)
    sys.exit("usage (from e2e/): python3 -m helpers.ssh_twins --write")
