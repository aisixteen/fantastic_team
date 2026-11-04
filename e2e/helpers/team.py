"""Start and stop the team binary in a run dir, and talk to it.

The binary is fantastic's command line, run in the run folder (fantastic has no `--dir`:
it works in the directory it is started in): `--headless` by default, `canvas --port N`
for the web. Everything it is told goes through `fantastic_team send '<item>'` or
`fantastic_team import <folder> --parent user --approve` (attach-only, over
`.fantastic/cli/mcp.sock`). `helpers/stock.py` builds the stock team with them.
"""

from __future__ import annotations

import json
import os
import re
import signal
import socket
import subprocess
import time
import urllib.error
import urllib.request

SOCK = os.path.join(".fantastic", "cli", "mcp.sock")
HEADLESS = ("--headless",)  # no screen, no port; the default start


def free_port() -> int:
    """A TCP port that is free right now (bound on 127.0.0.1, read, released)."""
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def http_get(port: int, path: str, timeout: float = 10.0) -> tuple[int, str]:
    """One GET on 127.0.0.1:port; (status, body)."""
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=timeout) as r:
            return r.status, r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")


def fill_text(text: str, variables: dict) -> str:
    return re.sub(r"\{\{(\w+)\}\}", lambda m: str(variables[m.group(1)]), text)


class Team:
    def __init__(self, binary: str, run_dir: str, env_extra: dict[str, str]) -> None:
        self.binary = binary
        self.dir = run_dir
        self.home = os.path.join(run_dir, "home")
        os.makedirs(self.home, exist_ok=True)
        self.env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": self.home,
            "TMPDIR": os.environ.get("TMPDIR", "/tmp"),
            "LANG": "C.UTF-8",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_TERMINAL_PROMPT": "0",
            **env_extra,
        }
        self.log_path = os.path.join(run_dir, "team.log")
        self.proc: subprocess.Popen | None = None
        self.port: int | None = None  # the web UI port of the last start that had one

    def argv(self, extra_args=HEADLESS) -> list[str]:
        """The command line of a start: fantastic's own words, run in the run folder
        (`["--headless"]`, `["canvas", "--port", "8123"]`, `["--headless", "ai", "anthropic", ...]`)."""
        return [self.binary, *map(str, extra_args)]

    def start(self, wait_s: float = 60.0, extra_args=HEADLESS) -> None:
        extra = list(map(str, extra_args))
        self.port = int(extra[extra.index("--port") + 1]) if "--port" in extra else None
        log = open(self.log_path, "wb")
        self.proc = subprocess.Popen(self.argv(extra), env=self.env,
                                     stdout=log, stderr=subprocess.STDOUT,
                                     stdin=subprocess.DEVNULL, cwd=self.dir)
        end = time.time() + wait_s
        while time.time() < end:
            if self.proc.poll() is not None:
                raise RuntimeError(f"team binary exited early ({self.proc.returncode}):\n{self.log()}")
            if os.path.exists(os.path.join(self.dir, SOCK)):
                return
            time.sleep(0.2)
        raise RuntimeError(f"team binary did not open {SOCK} in {wait_s}s:\n{self.log()}")

    def import_blueprint(self, folder: str, parent: str = "user", binds=None,
                         timeout: float = 120.0) -> str:
        """`fantastic_team import FOLDER --parent PARENT --bind @p=id ... --approve` on the running
        team; the command's output (`imported N: id, id`). Raises when fantastic refuses."""
        cmd = [self.binary, "import", folder, "--parent", parent, "--approve"]
        for port, agent in (binds or {}).items():
            cmd += ["--bind", f"{port}={agent}"]
        r = subprocess.run(cmd, env=self.env, cwd=self.dir, capture_output=True, text=True,
                           timeout=timeout)
        if r.returncode != 0 or not r.stdout.startswith("imported"):
            raise RuntimeError(f"import {folder} failed ({r.returncode}): {r.stdout}{r.stderr}")
        return r.stdout.strip()

    def agents(self) -> dict:
        """Every agent record, by id (`list_agents` on the kernel)."""
        r = self.send("kernel", {"type": "list_agents"})
        return {a["id"]: a for a in r["result"]["agents"]}

    def send(self, target: str, payload: dict, timeout: float = 120.0) -> dict:
        item = json.dumps({"target": target, "payload": payload})
        r = subprocess.run([self.binary, "send", item], env=self.env, cwd=self.dir,
                           capture_output=True, text=True, timeout=timeout)
        try:
            return json.loads(r.stdout)
        except ValueError:
            return {"status": "failed", "result": {"error": (r.stdout + r.stderr).strip()}}

    def stop(self, wait_s: float = 30.0) -> int | None:
        """SIGTERM, wait for the drain; kill only if it will not go."""
        if not self.proc or self.proc.poll() is not None:
            return self.proc.returncode if self.proc else None
        self.proc.send_signal(signal.SIGTERM)
        try:
            return self.proc.wait(wait_s)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            return self.proc.wait()

    def log(self) -> str:
        try:
            with open(self.log_path, errors="replace") as f:
                return f.read()
        except OSError:
            return ""

    def wait_for(self, what, timeout: float, every: float = 0.3):
        """Poll `what()` until it returns something truthy."""
        end = time.time() + timeout
        while time.time() < end:
            v = what()
            if v:
                return v
            if self.proc and self.proc.poll() is not None:
                return None
            time.sleep(every)
        return None


CHAT_VERBS = ["history", "send", "stop", "status", "watch"]  # tree.rs CHAT_VERBS


def grant_chat(team: "Team", leg: str, workers, verbs=CHAT_VERBS) -> None:
    """Give a ws leg the page's chat grant on `workers`, on both ends (`leg`'s `can_call`
    names the worker, the worker's `callable_by` names `leg`): what `tree::grant_chat_to`
    writes for the page's leg. An operator write through the `send` leg, merged into what
    the records hold. The binary's `--port` serves the canvas, whose leg is `canvas_ws`."""
    r = team.send("kernel", {"type": "list_agents"})
    agents = {a["id"]: a for a in r["result"]["agents"]}
    for w in workers:
        cb = dict(agents[w].get("callable_by") or {})
        cb[leg] = list(verbs)
        out = team.send("kernel", {"type": "update_agent", "id": w, "callable_by": cb})
        if out.get("status") != "completed":
            raise RuntimeError(f"cannot grant {leg} on {w}: {out}")
    cc = dict((agents.get(leg) or {}).get("can_call") or {})
    for w in workers:
        cc[w] = list(verbs)
    out = team.send("kernel", {"type": "update_agent", "id": leg, "can_call": cc})
    if out.get("status") != "completed":
        raise RuntimeError(f"cannot grant {leg}: {out}")


def log_calls(log_text: str) -> list[tuple[str, str]]:
    """`(agent, verb)` of every `[n] <agent> call <verb>` line the team printed."""
    return re.findall(r"\[\d+\] (\w+) call (\w+)", log_text)


def in_order(seen: list, wanted: list) -> bool:
    """True when `wanted` is a subsequence of `seen`."""
    it = iter(seen)
    return all(any(w == s for s in it) for w in wanted)
