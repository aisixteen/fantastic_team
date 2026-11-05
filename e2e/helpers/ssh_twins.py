"""The ssh twins of the cli scenarios (#242): one task split across the local kernel and one in a
Podman box reached over ssh, wired the WEAK way (existing pieces only, no new code in the product).

THE WIRING (fantastic's canvas is the model: `src/tui/src/kern/web.rs`, `user` -> `web` -> the socket
leg, the leg reaching `user`, whose `forward` makes the call):

1. START: the lead runs, through its exec agent `worker_exec`, `start` of
   `ssh ... -L N:127.0.0.1:N team@box 'cd w && exec fantastic_team --headless'` (the job holds the
   tunnel and the remote kernel; `echo $$ > team.pid` keeps its pid). Then one raw
   `ssh ... 'fantastic_team send ...'` makes the remote `web` host on 127.0.0.1:N under `user`, its
   `web_ws` leg `remote_ws` under it (the canvas's edges: `web` mounts the leg, the leg reaches
   `user` alone, `user` admits the leg's `forward`), the remote exec agent `worker_exec` in the
   clone, and boots `web`. Then `git clone` of the test's git server inside the box (through
   Podman's host gateway), by the same raw path.
2. BRIDGE: the lead's `create_agent` of a `ws_bridge` agent `remote` on the local kernel, dialling
   the tunnel's local end (`ws://127.0.0.1:N/remote_ws/ws`), as the canvas page dials `canvas_ws`.
3. REMOTE AGENTS: `create_agent` sent through the bridge's `forward` (to `user`, whose `forward`
   makes the call on the remote kernel); messages the same way. Results come back as the forward's
   reply (the remote agent's `history`).
4. STOP: the remote agents deleted through the bridge, the bridge deleted, SIGTERM to the remote
   kernel over ssh with a bounded wait (the binary has no stop word), the tunnel job stopped.

The scripted twins (`tests/cli_ssh/`) script those raw commands: `script.json` is written by
`write_scripts()` from the builders below (`python3 -m helpers.ssh_twins --write` in `e2e/`; the
selftest checks the files match), with `{{ssh_port}}`, `{{ssh_key}}`, `{{tunnel_port}}`,
`{{remote_endpoint}}`, `{{remote_git_url}}` bound once the box runs (`ScriptedModel.bind`). The gpu
twins (`tests/gpu_cli_ssh/`) tell the lead in plain words where the box is and what the wiring is
(`task_text`), and leave the commands to the model.

THE CHECKS, after the run (`Box.end`): the test's git server holds a linear history with a commit
made in the local clone and one made in the box's clone (each found in that clone's reflog), and
the fixture is solved there; the bridge is gone, the tunnel's port refuses, no exec job runs; the
remote kernel is stopped; started once more over ssh, its records hold the remote base and no
spawned agent, no orphan; then it is stopped and the container removed (`with Box(...)`).
"""

from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
import time

from . import cli, podman_remote as pr, team as team_mod, web
from .podman_remote import shq

HERE = os.path.dirname(os.path.abspath(__file__))
TESTS = os.path.join(os.path.dirname(HERE), "tests")

BRIDGE = "remote"  # the local ws_bridge
LEG = "remote_ws"  # the remote web_ws leg
WEB = "web"  # the remote HTTP host
HOME = f"/home/{pr.USER}"  # the box's home: the login's
DIR = "w"  # the remote kernel's folder, in the box's home
REPO = "repo"  # the clone, in DIR
REMOTE_REPO = f"{HOME}/{DIR}/{REPO}"
STOP_WAIT_S = 30  # the bounded wait after SIGTERM

SCENARIOS = ("single_lead", "lead_spawns_two", "precreated_1_2", "precreated_tree", "leads_spawn_leads",
             "headless_and_canvas")

# The fixes, one per side: the local side fixes adder.py, the remote side muler.py.
ADD = ("adder.py", "a - b", "a + b")
MUL = ("muler.py", "a + b", "a * b")


# ---- the raw commands ------------------------------------------------------------------

def ssh_args(command: str, tunnel: bool = False) -> list[str]:
    """The argv of a raw ssh to the box (placeholders for the run's port, key and tunnel)."""
    fwd = ("{{tunnel_port}}", "{{tunnel_port}}") if tunnel else None
    return pr.ssh_argv("{{ssh_port}}", "{{ssh_key}}", command, fwd)


def run_call(argv: list[str], wait_s: int = 60, verb: str = "run") -> dict:
    args = {"command": argv[0], "args": list(argv[1:])}
    if verb == "run":
        args["wait_s"] = wait_s
    return {"target": cli.EXEC, "type": verb, "args": args}


def ssh_call(command: str, wait_s: int = 60) -> dict:
    return run_call(ssh_args(command), wait_s)


def kernel_start_command(roles: bool = False) -> str:
    """The remote kernel, in the foreground of the tunnel's ssh session (its pid kept)."""
    env = "FANTASTIC_AGENTIC_RESOURCES=1 " if roles else ""
    return (f"mkdir -p {DIR} && cd {DIR} && echo $$ > team.pid && "
            f"{env}exec fantastic_team --headless >> team.log 2>&1")


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

REMOTE_EXEC = {"id": cli.EXEC, "parent_id": "user", "handler_module": "exec_bridge.tools",
               "ingress_rule": "allow_all_ingress", "cwd": REMOTE_REPO, "allowed": [".*"],
               "inherit_env": ["PATH", "LANG", "HOME"],
               "env": {"GIT_CONFIG_NOSYSTEM": "1", "GIT_TERMINAL_PROMPT": "0", "PYTHONDONTWRITEBYTECODE": "1"}}


def setup_command(cards: dict[str, str] | None = None) -> str:
    """One raw ssh: wait for the remote kernel's socket, then the web host, its leg and the
    canvas's edges, `user`'s grant for the leg, the remote exec agent, the role cards (if any),
    and boot `web` (it mounts the leg and listens on the tunnel's port)."""
    wait = (f"for i in $(seq 150); do [ -S {DIR}/.fantastic/cli/mcp.sock ] && break; sleep 0.2; done && "
            f"cd {DIR} && [ -S .fantastic/cli/mcp.sock ]")
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
             quiet(remote_send(kernel_item(dict(REMOTE_EXEC, type="create_agent"))))]
    for name, text in sorted((cards or {}).items()):
        if "'" in text:
            raise ValueError(f"role card {name} holds a single quote: it rides in a quoted shell word")
        steps.append(f"mkdir -p roles && printf '%s' {shq(text)} > roles/{name}")
    steps.append(quiet(remote_send({"target": WEB, "payload": {"type": "boot"}})))
    steps.append(remote_send(kernel_item({"type": "list_agents"})) + " | python3 -c " + shq(LIST_IDS))
    return " && ".join(steps)


def clone_command() -> str:
    return f"cd {DIR} && git clone -q {{{{remote_git_url}}}} {REPO} && git -C {REPO} log --oneline"


def stop_command() -> str:
    """SIGTERM to the remote kernel and a bounded wait (no stop word)."""
    return (f"cd {DIR} && p=$(cat team.pid) && kill -TERM $p && "
            f"for i in $(seq {STOP_WAIT_S * 5}); do kill -0 $p 2>/dev/null || exit 0; sleep 0.2; done; exit 1")


# ---- the bridge's calls ----------------------------------------------------------------

def bridge_record() -> dict:
    return {"id": BRIDGE, "parent_id": "user", "handler_module": "ws_bridge.tools",
            "ingress_rule": "allow_all_ingress", "transport": "ws", "host": "127.0.0.1",
            "port": "{{tunnel_port}}", "peer_id": LEG}


def forward(target: str, payload: dict, timeout: int = 120) -> dict:
    """A call on the remote kernel: the bridge ships it to the leg, the leg asks `user` to forward it."""
    return {"target": BRIDGE, "type": "forward",
            "args": {"target": "user", "payload": {"type": "forward", "target": target, "payload": payload},
                     "forward_timeout": timeout}}


def remote_ai(id: str, parent: str = "user") -> dict:
    """An AI agent on the remote kernel: the plug's model through the gateway, the shared memory
    store. No grants are written: the remote kernel's reach grants every AI agent (fantastic
    `src/tui/src/kern/reach.rs`)."""
    return {"type": "create_agent", "id": id, "parent_id": parent, "handler_module": "anthropic_backend.tools",
            "model": cli.MODEL, "endpoint": "{{remote_endpoint}}", "memory_store_id": "memory"}


def remote_registry(total: int) -> dict:
    """The remote roles registry (fantastic's `agentic_resources`), cards under `w/roles`."""
    return {"type": "create_agent", "id": "roles", "parent_id": "user", "handler_module": "agentic_resources.tools",
            "role_dirs": ["roles"], "limits": {"total": total},
            "tiers": {"small": {"handler_module": "anthropic_backend.tools", "model": cli.MODEL,
                                "endpoint": "{{remote_endpoint}}", "memory_store_id": "memory"}}}


def t(call: dict, lane: str | None = None, after=()) -> dict:
    turn = {"calls": [call]}
    if lane:
        turn["lane"] = lane
    if after:
        turn["after"] = list(after)
    return turn


def open_turns(roles: dict[str, str] | None = None, after=()) -> list[dict]:
    """The lead's start: the tunnel and the remote kernel, the remote's surface and exec agent,
    the clone in the box, the bridge."""
    return [t(run_call(ssh_args(kernel_start_command(roles is not None), tunnel=True), verb="start"), after=after),
            t(ssh_call(setup_command(roles))),
            t(ssh_call(clone_command())),
            t({"target": "kernel", "type": "create_agent", "args": bridge_record()})]


AWAIT = {"name": "await"}


def stop_turns() -> list[dict]:
    """SIGTERM to the remote kernel over ssh. The tunnel's ssh session ends with the kernel, so two
    messages come back, in either order: the stop's reply and the tunnel job's `job_done`. The
    first is answered with `await` (the other is in flight), the second goes on."""
    return [t(ssh_call(stop_command())), t(AWAIT)]


def close_turns(remote_ids=(), verify: bool = True) -> list[dict]:
    """The lead's end: the remote agents deleted through the bridge, the bridge deleted, the remote
    kernel stopped over ssh (its tunnel closes with it); then the test run on the pulled tree."""
    turns = [t(forward("kernel", {"type": "delete_agent", "id": i})) for i in remote_ids]
    turns += [t({"target": "kernel", "type": "delete_agent", "args": {"id": BRIDGE}}), *stop_turns()]
    if verify:
        turns += [t(cli.run_call("git", "pull", "-q", "--ff-only")), t(cli.run_call("python3", "test_calc.py"))]
    return turns + [t({"name": "done"})]


def fix_turns(fix, msg: str, lane: str | None, pull: bool = False, after=()) -> list[dict]:
    """A worker's turns on its own side's exec agent: (pull,) test, fix, test, commit, push, done."""
    calls = [cli.run_call("git", "pull", "-q", "--ff-only")] if pull else []
    calls += [cli.run_call("python3", "test_calc.py"), cli.replace_call(*fix), cli.run_call("python3", "test_calc.py"),
              cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                           "commit", "-q", "-a", "-m", msg),
              cli.run_call("git", "push", "-q", "origin", "HEAD:main")]
    turns = [t(c, lane) for c in calls] + [t({"name": "done"}, lane)]
    if after:
        turns[0]["after"] = list(after)
    return turns


def history(id: str) -> dict:
    """Results back: the remote agent's conversation, as the forward's reply."""
    return forward(id, {"type": "history"})


def remote_log_call() -> dict:
    """Results back from a spawned remote agent, whose conversation only its spawner reads (the
    registry gives the spawner pause/resume/stop on it): the box clone's last commit, through the
    bridge (the remote exec agent's `run`)."""
    return forward(cli.EXEC, {"type": "run", "command": "git", "args": ["log", "-1", "--format=%s"], "wait_s": 30})


# ---- the six scripts -------------------------------------------------------------------

def cards_of(name: str) -> dict[str, str]:
    folder = cli.roles_dir(name)
    out = {}
    for f in sorted(os.listdir(folder)):
        with open(os.path.join(folder, f)) as fh:
            out[f] = fh.read()
    return out


def script_single_lead() -> list[dict]:
    """The lead alone, twice (A: made with create_agent; B: fantastic's recipe). Locally it fixes
    adder.py and pushes; in its remote workspace (the box's exec agent, through the bridge) it
    pulls, fixes muler.py, commits and pushes."""
    def one() -> list[dict]:
        local = [t(c) for c in (cli.run_call("python3", "test_calc.py"), cli.replace_call(*ADD),
                                cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                                             "commit", "-q", "-a", "-m", "Fix add"),
                                cli.run_call("git", "push", "-q", "origin", "HEAD:main"))]
        remote = [t(forward(cli.EXEC, {"type": "run", "command": c["args"]["command"], "args": c["args"]["args"],
                                       "wait_s": 30}))
                  for c in (cli.run_call("git", "pull", "-q", "--ff-only"), cli.replace_call(*MUL),
                            cli.run_call("python3", "test_calc.py"),
                            cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                                         "commit", "-q", "-a", "-m", "Fix mul"),
                            cli.run_call("git", "push", "-q", "origin", "HEAD:main"))]
        return open_turns() + local + remote + close_turns()
    return one() + one()


def script_precreated_1_2() -> list[dict]:
    """The lead and w1 local (w1 made before the task); w2 made on the remote kernel by the lead
    through the bridge. w1 fixes adder.py and pushes, then w2 pulls, fixes muler.py and pushes;
    the lead reads w2's conversation back through the bridge and deletes it there."""
    lead = open_turns() + [
        t(forward("kernel", remote_ai("w2"))),
        t({"target": "w1", "type": "send", "args": {"text": "[lane:w1] Fix adder.py, commit and push."}}),
        t(forward("w2", {"type": "send", "text": "[lane:w2] Pull, fix muler.py, commit and push."}), after=["w1"]),
        t(history("w2"), after=["w2"]),
    ] + close_turns(["w2"])
    return lead + fix_turns(ADD, "Fix add", "w1") + fix_turns(MUL, "Fix mul", "w2", pull=True)


def script_precreated_tree() -> list[dict]:
    """Local: the lead, s1, w1 (made before the task). Remote: s2 and w2 under it, made by the lead
    through the bridge. s1 has w1 fix adder.py; then s2 (remote) has w2 (remote) fix muler.py."""
    lead = open_turns() + [
        t(forward("kernel", remote_ai("s2"))),
        t(forward("kernel", remote_ai("w2", parent="s2"))),
        t({"target": "s1", "type": "send", "args": {"text": "[lane:s1] Have your worker fix adder.py."}}),
        t(forward("s2", {"type": "send", "text": "[lane:s2] Have your worker w2 fix muler.py."}), after=["s1", "w1"]),
        t(history("w2"), after=["s2", "w2"]),
    ] + close_turns(["w2", "s2"])
    s1 = [t({"target": "w1", "type": "send", "args": {"text": "[lane:w1] Fix adder.py, commit and push."}}, "s1"),
          t({"name": "done"}, "s1", after=["w1"])]
    s2 = [t({"target": "w2", "type": "send", "args": {"text": "[lane:w2] Pull, fix muler.py, commit and push."}}, "s2"),
          t({"name": "done"}, "s2", after=["w2"])]
    return lead + s1 + s2 + fix_turns(ADD, "Fix add", "w1") + fix_turns(MUL, "Fix mul", "w2", pull=True)


def script_lead_spawns_two() -> list[dict]:
    """The lead spawns w1 on the local registry and w2 on the remote one (through the bridge);
    w1 fixes adder.py and pushes, then w2 pulls, fixes muler.py and pushes; the lead deletes each
    where it lives."""
    lead = open_turns(cards_of("lead_spawns_two")) + [
        t(forward("kernel", remote_registry(1))),
        t({"target": "roles", "type": "spawn", "args": {"role": "worker", "task": "[lane:w1] Fix adder.py, commit and push."}}),
        t(forward("roles", {"type": "spawn", "role": "worker", "task": "[lane:w2] Pull, fix muler.py, commit and push."}),
          after=["w1"]),
        t(remote_log_call(), after=["w2"]),
        t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}),
    ] + close_turns(["{{minted:2}}", "roles"])
    return lead + fix_turns(ADD, "Fix add", "w1") + fix_turns(MUL, "Fix mul", "w2", pull=True)


def script_leads_spawn_leads() -> list[dict]:
    """The lead spawns s1 on the local registry and s2 on the remote one (through the bridge); each
    sub-lead spawns its worker on its own kernel's registry and deletes it; the lead deletes s1 and,
    through the bridge, s2."""
    lead = open_turns(cards_of("leads_spawn_leads")) + [
        t(forward("kernel", remote_registry(2))),
        t({"target": "roles", "type": "spawn", "args": {"role": "sublead", "task": "[lane:s1] Have a worker fix adder.py, then delete it."}}),
        t(forward("roles", {"type": "spawn", "role": "sublead",
                            "task": "[lane:s2] Have a worker pull, fix muler.py and push, then delete it."}),
          after=["s1", "w1"]),
        t(remote_log_call(), after=["s2", "w2"]),
        t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}),
    ] + close_turns(["{{minted:2}}", "roles"])
    s1 = [t({"target": "roles", "type": "spawn", "args": {"role": "worker", "task": "[lane:w1] Fix adder.py, commit and push."}}, "s1"),
          t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}, "s1", after=["w1"]),
          t({"name": "done"}, "s1")]
    s2 = [t({"target": "roles", "type": "spawn", "args": {"role": "worker", "task": "[lane:w2] Pull, fix muler.py, commit and push."}}, "s2"),
          t({"target": "kernel", "type": "delete_agent", "args": {"id": "{{minted:1}}"}}, "s2", after=["w2"]),
          t({"name": "done"}, "s2")]
    return lead + s1 + s2 + fix_turns(ADD, "Fix add", "w1") + fix_turns(MUL, "Fix mul", "w2", pull=True)


def notes_call(line: str) -> dict:
    if "'" in line:
        raise ValueError("a note rides in a single-quoted Python string")
    code = f"open('NOTES.md', 'a').write('{line}\\n')"
    return cli.run_call("python3", "-c", code)


def commit_push(msg: str) -> list[dict]:
    return [cli.run_call("git", "add", "NOTES.md"),
            cli.run_call("git", "-c", "user.name=Worker", "-c", "user.email=worker@example.invalid",
                         "commit", "-q", "-m", msg),
            cli.run_call("git", "push", "-q", "origin", "HEAD:main")]


CANVAS = {"type": "create_agent", "id": "sketch", "parent_id": "rlead", "handler_module": "canvas_page.tools"}


def wait_up_command() -> str:
    """The restarted remote kernel is up: its socket file, `web` booted (it listens on the tunnel's
    port again; a record is not booted by the load)."""
    return (f"for i in $(seq 150); do [ -S {DIR}/.fantastic/cli/mcp.sock ] && break; sleep 0.2; done && "
            f"cd {DIR} && " + quiet(remote_send({"target": WEB, "payload": {"type": "boot"}})) + " && "
            "for i in $(seq 150); do (echo > /dev/tcp/127.0.0.1/{{tunnel_port}}) 2>/dev/null && exit 0; sleep 0.2; done; exit 1")


def script_headless_and_canvas() -> list[dict]:
    """Three messages to the lead. 1 (the local kernel runs `canvas --port P`): the remote,
    `rlead` made there, the lead's note committed and pushed, `rlead` told to make its canvas
    (`create_agent` of a `canvas_page` under itself) and add its note. 1b: the bridge deleted
    and the remote kernel stopped. 2 (the local kernel restarted as `webui --port P2`): the
    tunnel and the remote kernel again, the bridge reconnected. 3: `sketch` and `rlead` deleted
    through the bridge, the bridge deleted, the remote stopped (its tunnel closes with it)."""
    one = open_turns() + [
        t(forward("kernel", remote_ai("rlead"))),
        t(notes_call("local: the notes of the lead")),
        *[t(c) for c in commit_push("Notes: the lead")],
        t(forward("rlead", {"type": "send", "text": "[lane:r] Make a canvas for your notes and add your note."})),
        t(remote_log_call(), after=["r"]),
        t({"name": "done"}),
    ]
    # the bridge is deleted first: its open forwarded channel would keep the tunnel's ssh session
    # alive after the remote kernel ends (it is made again in 2)
    pause = [t({"target": "kernel", "type": "delete_agent", "args": {"id": BRIDGE}}), *stop_turns(),
             t({"name": "done"})]
    # the lead's kernel restarted: its first request rebuilds its working context from its records
    # (fantastic ai-core's recall pass), a request of its own, answered before the message's turns
    two = [t({"text": "The context as it is."}),
           t(run_call(ssh_args(kernel_start_command(), tunnel=True), verb="start")),
           t(ssh_call(wait_up_command())),
           t({"target": "kernel", "type": "create_agent", "args": bridge_record()}),
           t({"name": "done"})]
    three = [t(forward("kernel", {"type": "delete_agent", "id": CANVAS["id"]})),
             t(forward("kernel", {"type": "delete_agent", "id": "rlead"})),
             t({"target": "kernel", "type": "delete_agent", "args": {"id": BRIDGE}}),
             *stop_turns(),
             t(cli.run_call("git", "pull", "-q", "--ff-only")),
             t({"name": "done"})]
    r = [t({"target": "kernel", "type": "create_agent", "args": {k: v for k, v in CANVAS.items() if k != "type"}}, "r"),
         t(cli.run_call("git", "pull", "-q", "--ff-only"), "r"),
         t(notes_call("remote: the notes of rlead"), "r"),
         *[t(c, "r") for c in commit_push("Notes: rlead")],
         t({"name": "done"}, "r")]
    return one + pause + two + three + r


CANVAS_PHASES = (12, 4, 5, 7)  # the lead's turns per message of script_headless_and_canvas (lane r: 7 more)


SCRIPTS = {"single_lead": script_single_lead, "precreated_1_2": script_precreated_1_2,
           "precreated_tree": script_precreated_tree, "lead_spawns_two": script_lead_spawns_two,
           "leads_spawn_leads": script_leads_spawn_leads, "headless_and_canvas": script_headless_and_canvas}


def script_path(name: str) -> str:
    return os.path.join(TESTS, "cli_ssh", name, "script.json")


def render_script(name: str) -> str:
    return json.dumps(SCRIPTS[name](), indent=1) + "\n"


def write_scripts() -> None:
    for name in SCENARIOS:
        with open(script_path(name), "w") as f:
            f.write(render_script(name))


def stale_scripts() -> list[str]:
    """The cli_ssh scripts that differ from what the builders write (empty when in sync)."""
    out = []
    for name in SCENARIOS:
        try:
            with open(script_path(name)) as f:
                if f.read() != render_script(name):
                    out.append(name)
        except OSError:
            out.append(name)
    return out


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
    config (the local sandbox's, the box's `~/.gitconfig`), never in a command."""

    def __init__(self, ctx, name: str = "box", roles: bool = False) -> None:
        self.ctx = ctx
        self.roles = roles
        self.remote = pr.Remote(os.path.join(ctx.run_dir, name))
        self.tunnel_port = team_mod.free_port()
        self.stock: set[str] = set()

    def __enter__(self) -> "Box":
        self.remote.__enter__()
        try:
            self.write_box_gitconfig()
            self.stock = set(self.records(f"{DIR}0"))  # a fresh remote kernel's own agents
        except BaseException:
            self.remote.stop()
            raise
        return self

    def __exit__(self, *_):
        self.remote.stop()

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
        with open(os.path.join(ctx.work, "sandbox", ".gitconfig"), "w") as f:
            f.write(gitconfig(git_url(ctx), auth_header(ctx)))
        cli.git(ctx.repo, "-c", "http.extraHeader=" + auth_header(ctx), "fetch", "-q", "origin")
        cli.git(ctx.repo, "branch", "-q", "-u", "origin/main", "main")

    def values(self) -> dict:
        """The script's run-time values: the box's ssh port and key, the tunnel's port, the model
        and the git server as the box dials them (Podman's host gateway)."""
        return {"ssh_port": str(self.remote.port), "ssh_key": self.remote.key, "tunnel_port": str(self.tunnel_port),
                "remote_endpoint": pr.gateway_url(self.ctx.model.endpoint),
                "remote_git_url": pr.gateway_url(git_url(self.ctx))}

    def bind(self) -> None:
        self.ctx.model.bind(**self.values())

    # -- the remote kernel, by the harness (the checks only) --

    def env(self) -> str:
        return "FANTASTIC_AGENTIC_RESOURCES=1 " if self.roles else ""

    def records(self, folder: str = DIR) -> dict:
        """Start the remote kernel in `folder` over ssh, list its records, stop it (SIGTERM, a
        bounded wait). id -> record."""
        start = (f"mkdir -p {folder} && cd {folder} && {{ {self.env()}nohup fantastic_team --headless >> check.log 2>&1 & "
                 "echo $! > check.pid; } && for i in $(seq 150); do [ -S .fantastic/cli/mcp.sock ] && break; sleep 0.2; done && "
                 + remote_send(kernel_item({"type": "list_agents"})))
        try:
            out = self.remote.ssh(start, timeout=120).stdout
        finally:
            self.remote.ssh(f"cd {folder} && p=$(cat check.pid) && kill -TERM $p && "
                            f"for i in $(seq {STOP_WAIT_S * 5}); do kill -0 $p 2>/dev/null || exit 0; sleep 0.2; done; exit 1",
                            check=False)
        return {a["id"]: a for a in json.loads(out)["result"]["agents"]}

    def remote_log(self) -> str:
        return self.remote.ssh(f"cat {DIR}/team.log", check=False).stdout

    def reflog_commits(self, remote: bool) -> list[str]:
        cmd = ["reflog", "--format=%H %gs"]
        if remote:
            out = self.remote.ssh(f"git -C {DIR}/{REPO} " + " ".join(shq(c) for c in cmd), check=False).stdout
        else:
            out = cli.git(self.ctx.repo, *cmd, check=False).stdout
        return [line.split()[0] for line in out.splitlines() if " commit" in line and "commit (initial)" not in line]

    # -- the checks --

    def end(self, v, remote_base=(), label: str = "", solved: bool = True) -> dict:
        """The checks every twin shares. `remote_base`: the agents the lead keeps on the remote kernel
        beyond its stock (the surface and the exec agent are always there). `solved`: the fixture
        is the task (the canvas twin's task is its notes, checked by its probe). Returns the remote
        records; the git server's files at HEAD are in `self.server_files`."""
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
        self.remote_log_text = log_text
        return recs

    def listening(self) -> set[int]:
        """The TCP ports the box listens on (its /proc/net/tcp and tcp6, LISTEN rows)."""
        rows = self.remote.ssh("cat /proc/net/tcp /proc/net/tcp6 2>/dev/null | awk '$4==\"0A\"{print $2}'",
                               check=False).stdout.split()
        return {int(a.rsplit(":", 1)[1], 16) for a in rows}

    def removed(self, v, label: str = "") -> None:
        v.check(f"{label}the container is removed", not self.remote.exists())


def remote_spawns(log_text: str) -> list[tuple[str, str]]:
    return [c for c in team_mod.log_calls(log_text) if c[1] == "spawn"]


def hand(ctx, lead: str, text: str, upto: int | None = None, wait_s: float = 240) -> bool:
    """Hand the lead a message; wait until the plug has served `upto` turns (default: all)."""
    r = ctx.team.send(lead, {"type": "send", "text": text})
    want = len(ctx.model.turns) if upto is None else upto
    end = time.time() + wait_s
    while time.time() < end and ctx.model.served < want:
        time.sleep(0.3)
    time.sleep(2.0)
    dump_requests(ctx)
    return r.get("status") == "completed" and ctx.model.served >= want


def came_back(ctx, lane: str | None, evidence: str) -> bool:
    """True when a request of the lead (the main lane) ends with a reply of the bridge's `forward`
    holding the remote agent's conversation (its task, `[lane:L]`) or, `lane` None, the box's own
    output, and `evidence` of the work."""
    for at, raw in zip(ctx.model.request_lanes, ctx.model.raw_bodies):
        if at != "main":
            continue
        try:
            last = json.loads(raw)["messages"][-1]["content"]
        except (ValueError, KeyError, IndexError, TypeError):
            continue
        text = last if isinstance(last, str) else json.dumps(last)
        if f"· {BRIDGE} forward ·" in text and (lane is None or f"[lane:{lane}]" in text) and evidence in text:
            return True
    return False


def dump_requests(ctx) -> None:
    """The plug's requests (lane, body) in the run folder, to read a failed run by."""
    with open(os.path.join(ctx.run_dir, "model-requests.json"), "w") as f:
        json.dump([{"lane": l, "body": b} for l, b in zip(ctx.model.request_lanes, ctx.model.raw_bodies)], f, indent=1)


def forward_replies(ctx) -> list[str]:
    """The lead's requests that answer a `forward` of the bridge (their last message)."""
    out = []
    for at, raw in zip(ctx.model.request_lanes, ctx.model.raw_bodies):
        try:
            last = json.loads(raw)["messages"][-1]["content"]
        except (ValueError, KeyError, IndexError, TypeError):
            continue
        text = last if isinstance(last, str) else json.dumps(last)
        if at == "main" and f"· {BRIDGE} forward ·" in text:
            out.append(text)
    return out


# ---- the canvas twins: the remote agents, listed through a local socket ------------------

# What a local page asks over its socket: `user.forward` to the bridge, which asks the remote
# `user` to forward `list_agents` to the remote kernel.
ASK = {"type": "forward", "target": "user", "payload": {"type": "forward", "target": "kernel",
                                                         "payload": {"type": "list_agents"}}}


def remote_agents(reply) -> dict:
    """The remote kernel's records out of a forwarded `list_agents` (the reply may be wrapped)."""
    r = reply or {}
    for _ in range(3):
        if isinstance(r, dict) and "agents" in r:
            return {a.get("id"): a for a in r["agents"]}
        r = r.get("result") if isinstance(r, dict) else None
    return {}


def ask_through(port, path: str = "/team_ws/ws") -> dict:
    """`ASK` over a local page's socket (`path`: its leg's, the team page's by default)."""
    try:
        ws = web.WsClient(int(port), path=path)
        reply = ws.call("user", {"type": "forward", "target": BRIDGE, "payload": ASK}, timeout=60)
        ws.close()
        return reply
    except OSError as e:
        return {"error": str(e)}


def shows_canvas(v, label: str, reply) -> None:
    agents = remote_agents(reply)
    sk = agents.get("sketch") or {}
    v.check(f"{label} lists the remote agents through the bridge: rlead and the canvas it made",
            "rlead" in agents and sk.get("handler_module") == "canvas_page.tools" and sk.get("parent_id") == "rlead",
            str(sorted(agents))[:300] if agents else str(reply)[:400])


if __name__ == "__main__":
    if sys.argv[1:] == ["--write"]:
        write_scripts()
        print("wrote " + ", ".join(script_path(n) for n in SCENARIOS))
        sys.exit(0)
    sys.exit("usage (from e2e/): python3 -m helpers.ssh_twins --write")
