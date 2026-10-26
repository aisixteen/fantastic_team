"""The app shell (`fantastic_team_app`) and the in-process handle example, as processes.

Both are built by `run.py` next to the team binary (`cargo build -p fantastic-team-app`,
`cargo build -p fantastic-team --example handle_serve`). A test whose binary is not
there raises `Skip`, not a failure.
"""

from __future__ import annotations

import os
import signal
import socket
import subprocess
import time

from .verdict import Skip
from . import web


def app_binary(ctx) -> str:
    p = os.path.join(os.path.dirname(os.path.abspath(ctx.binary)), "fantastic_team_app")
    if not os.path.isfile(p):
        raise Skip("fantastic_team_app is not built next to --bin")
    return p


def handle_example(ctx) -> str:
    p = os.path.join(os.path.dirname(os.path.abspath(ctx.binary)), "examples", "handle_serve")
    if not os.path.isfile(p):
        raise Skip("the handle_serve example is not built next to --bin")
    return p


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def env_of(team) -> dict:
    """The same minimal environment the team binary gets (no secrets from the launcher)."""
    return dict(team.env)


class Proc:
    """A spawned process whose combined output goes to a log file in the run dir."""

    def __init__(self, argv, env: dict, cwd: str, log_path: str, stdin=subprocess.DEVNULL) -> None:
        self.log_path = log_path
        self.argv = list(argv)
        log = open(log_path, "wb")
        self.p = subprocess.Popen(argv, env=env, cwd=cwd, stdin=stdin, stdout=log,
                                  stderr=subprocess.STDOUT)
        log.close()

    @property
    def pid(self) -> int:
        return self.p.pid

    def log(self) -> str:
        try:
            with open(self.log_path, errors="replace") as f:
                return f.read()
        except OSError:
            return ""

    def wait_line(self, needle: str, timeout: float = 60.0) -> str | None:
        """The first log line holding `needle`, or None (also when the process ended without it)."""
        end = time.time() + timeout
        while time.time() < end:
            for line in self.log().splitlines():
                if needle in line:
                    return line
            if self.p.poll() is not None:
                for line in self.log().splitlines():
                    if needle in line:
                        return line
                return None
            time.sleep(0.15)
        return None

    def signal(self, sig: int) -> None:
        if self.p.poll() is None:
            self.p.send_signal(sig)

    def wait(self, timeout: float = 30.0) -> int | None:
        try:
            return self.p.wait(timeout)
        except subprocess.TimeoutExpired:
            return None

    def reap(self) -> None:
        """Test cleanup: SIGINT, then SIGKILL; never leaves the process or its children."""
        if self.p.poll() is None:
            for kid in child_pids(self.p.pid):
                _kill(kid)
            self.p.send_signal(signal.SIGINT)
            if self.wait(10) is None:
                self.p.kill()
                self.p.wait()


def _kill(pid: int) -> None:
    try:
        os.kill(pid, signal.SIGKILL)
    except OSError:
        pass


def run_app(ctx, args, cwd: str | None = None, name: str = "app") -> Proc:
    """Spawn `fantastic_team_app ARGS` with the team's minimal environment."""
    team = ctx.team
    env = env_of(team) if team else {"PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                                      "HOME": os.path.join(ctx.run_dir, "home"),
                                      "TMPDIR": os.environ.get("TMPDIR", "/tmp")}
    return Proc([app_binary(ctx), *map(str, args)], env, cwd or ctx.run_dir,
                os.path.join(ctx.run_dir, f"{name}.log"))


def child_pids(pid: int) -> list[int]:
    r = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True)
    return [int(x) for x in r.stdout.split()]


def alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    # a zombie is not alive for our purposes
    r = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True)
    return bool(r.stdout.strip()) and not r.stdout.strip().startswith("Z")


def wait_gone(pid: int, timeout: float = 15.0) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        if not alive(pid):
            return True
        time.sleep(0.15)
    return not alive(pid)


def wait_http(url: str, timeout: float = 60.0) -> bool:
    """Poll until `url` answers 200."""
    end = time.time() + timeout
    while time.time() < end:
        try:
            if web.http_get(url, timeout=3)[0] == 200:
                return True
        except OSError:
            pass
        time.sleep(0.2)
    return False


def wait_refused(port: int, timeout: float = 15.0) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        if web.refuses(port):
            return True
        time.sleep(0.15)
    return web.refuses(port)


def files_holding(root: str, needle: str, skip=("secrets",)) -> list[str]:
    """Paths under `root` (not under a top-level folder in `skip`) whose bytes hold `needle`."""
    hits = []
    for base, dirs, names in os.walk(root):
        if os.path.abspath(base) == os.path.abspath(root):
            dirs[:] = [d for d in dirs if d not in skip]
        for n in names:
            path = os.path.join(base, n)
            try:
                if os.path.islink(path):
                    continue
                with open(path, "rb") as f:
                    if needle.encode() in f.read():
                        hits.append(os.path.relpath(path, root))
            except OSError:
                pass
    return hits
