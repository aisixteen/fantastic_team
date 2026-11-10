"""Start and stop the team binary in a run dir, and talk to it.

The binary is fantastic's command line, run in the run folder (fantastic has no `--dir`:
it works in the directory it is started in): `--headless` by default, `webui --port N`
for the web. Everything it is told goes through `fantastic_team send '<item>'` (attach-only, over
`.fantastic/cli/mcp.sock`): `create_agent` and `update_agent` on the root. A test's agents are record files
under `e2e/fleets/` (named by `fleet:` in its `test.yaml`), laid into `<run>/.fantastic/` before the first
start (`lay_fleet`); `helpers/stock.py` builds the stock team with `send` + `create_agent`.
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


def fleet_files(layers, variables: dict, own: str | None = None):
    """`(path relative to .fantastic/, text with its `{{marks}}` filled)` for every file of the layer
    folders, in order, the `own` folder last; a later layer's file of the same path replaces the
    earlier one whole. Copy and fill only: no record is read. A missing folder raises."""
    files: dict[str, str] = {}
    for folder in [*layers, *([own] if own else [])]:
        if not os.path.isdir(folder):
            raise FileNotFoundError(f"fleet layer folder {folder} does not exist")
        for dirpath, dirs, names in os.walk(folder):
            dirs.sort()
            for name in sorted(names):
                path = os.path.join(dirpath, name)
                with open(path) as f:
                    files[os.path.relpath(path, folder)] = fill_text(f.read(), variables)
    yield from files.items()


def lay_fleet(dest: str, layers, variables: dict, own: str | None = None) -> list[str]:
    """Write the fleet's files under `<dest>/.fantastic/`, before the first start, and return the ids
    (the names of the folders that hold an `agent.yaml`, the root's own file excluded)."""
    ids = []
    for rel, text in fleet_files(layers, variables, own):
        path = os.path.join(dest, ".fantastic", rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)
        if os.path.basename(rel) == "agent.yaml" and os.path.dirname(rel):
            ids.append(os.path.basename(os.path.dirname(rel)))
    return ids


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
        (`["--headless"]`, `["webui", "--port", "8123"]`, `["--headless", "ai", "anthropic", ...]`)."""
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
                # what a test preseeds into a memory store (RAM) again on every start
                for again in getattr(self, "on_start", []):
                    again(self)
                return
            time.sleep(0.2)
        raise RuntimeError(f"team binary did not open {SOCK} in {wait_s}s:\n{self.log()}")

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


def log_calls(log_text: str) -> list[tuple[str, str]]:
    """`(agent, verb)` of every `[n] <agent> call <verb>` line the team printed."""
    return re.findall(r"\[\d+\] (\w+) call (\w+)", log_text)


def in_order(seen: list, wanted: list) -> bool:
    """True when `wanted` is a subsequence of `seen`."""
    it = iter(seen)
    return all(any(w == s for s in it) for w in wanted)
