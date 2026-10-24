"""Start and stop the team binary in a run dir, and talk to it.

The binary runs headless (`--dir RUN`). Everything it is told goes through
`fantastic_team send '<item>'` (attach-only, over `.fantastic/cli/mcp.sock`).
"""

from __future__ import annotations

import json
import os
import re
import signal
import subprocess
import time

SOCK = os.path.join(".fantastic", "cli", "mcp.sock")


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

    def write_yaml(self, template_path: str, variables: dict) -> None:
        with open(template_path) as f:
            text = fill_text(f.read(), variables)
        with open(os.path.join(self.dir, "team.yaml"), "w") as f:
            f.write(text)

    def start(self, wait_s: float = 60.0) -> None:
        log = open(self.log_path, "wb")
        self.proc = subprocess.Popen([self.binary, "--dir", self.dir], env=self.env,
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


def boot(team: "Team", yaml_template: str, variables: dict) -> None:
    """Bring the team up from `yaml_template`.

    The binary's own start-up (plan the tree from team.yaml, apply it, then run
    the shell) cannot persist the tree into a fresh directory at this commit: it
    seeds a sealed store, and the shell then finds no `kernel_memory`. So:
    1. start with no team.yaml (the shell seeds its own base tree), add one
       open file store at the root with a public verb, stop;
    2. write team.yaml and start again: the start-up now finds a store, applies
       the tree to disk, and the shell boots it.
    """
    for d in ("work/wt", "work/tasks", "sandbox/home"):  # the binary makes none of these
        os.makedirs(os.path.join(team.dir, d), exist_ok=True)
    team.start()
    r = team.send("kernel", {"type": "create_agent", "id": "team_store", "parent_id": "kernel",
                             "handler_module": "file_bridge.tools", "root": ".fantastic",
                             "allow": ["."], "ingress_rule": "allow_all_ingress"})
    if r.get("status") != "completed":
        raise RuntimeError(f"cannot add the root store: {r}")
    team.stop()
    team.write_yaml(yaml_template, variables)
    team.start()


def wire(team: "Team") -> None:
    """The operator's grants a team.yaml cannot say: the credential store admits
    the lead (its model key) and the github connector (the app key); the puller
    may call github and the queue folder; the operator (the `send` leg) may call
    them too; the worker and queue folders are opened for the lead (they are
    sealed by default, and so no model could use them)."""
    both = {"$self": "*", "$parent": "*", "puller": "*", "mcp": "*"}
    steps = [
        ("credentials", {"callable_by": {"$self": "*", "github": "*", "lead": "*"}}),
        ("puller", {"can_call": {"$self": "*", "github": "*", "queue_files": "*"}}),
        ("github", {"callable_by": both, "can_call": {"$self": "*", "credentials": "*"}}),
        ("queue_files", {"callable_by": both, "ingress_rule": "allow_all_ingress"}),
        ("worker_files", {"ingress_rule": "allow_all_ingress"}),
    ]
    for agent, fields in steps:
        r = team.send("kernel", dict(fields, type="update_agent", id=agent))
        if r.get("status") != "completed":
            raise RuntimeError(f"cannot wire {agent}: {r}")


def log_calls(log_text: str) -> list[tuple[str, str]]:
    """`(agent, verb)` of every `[n] <agent> call <verb>` line the team printed."""
    return re.findall(r"\[\d+\] (\w+) call (\w+)", log_text)


def in_order(seen: list, wanted: list) -> bool:
    """True when `wanted` is a subsequence of `seen`."""
    it = iter(seen)
    return all(any(w == s for s in it) for w in wanted)
