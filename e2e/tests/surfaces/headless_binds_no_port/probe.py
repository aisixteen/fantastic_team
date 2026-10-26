"""Headless (no surface flag): the team binds no TCP port, draws no terminal UI,
and a verb still works over the socket. `--port N` is the positive control: the
same listener check then sees port N."""

import os
import shutil
import socket
import subprocess
import time

from helpers import team as team_mod
from helpers.verdict import Verdict

ALT_SCREEN = ("\x1b[?1049h", "\x1b[?25l", "\x1b[2J")


def descendants(pid):
    out = [pid]
    r = subprocess.run(["pgrep", "-P", str(pid)], capture_output=True, text=True)
    for line in r.stdout.split():
        out += descendants(int(line))
    return out


def listeners(pid):
    """TCP LISTEN ports of pid and its children, or None when this host cannot tell."""
    pids = descendants(pid)
    if shutil.which("lsof"):
        r = subprocess.run(["lsof", "-nP", "-a", "-p", ",".join(map(str, pids)), "-iTCP",
                            "-sTCP:LISTEN", "-Fn"], capture_output=True, text=True)
        return {l.rsplit(":", 1)[-1] for l in r.stdout.splitlines() if l.startswith("n")}
    if os.path.exists("/proc/net/tcp"):
        inodes = set()
        for p in pids:
            fd_dir = f"/proc/{p}/fd"
            try:
                for fd in os.listdir(fd_dir):
                    link = os.readlink(os.path.join(fd_dir, fd))
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


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def run(ctx):
    v = Verdict("surfaces/headless_binds_no_port")
    if listeners(os.getpid()) is None:
        print("SKIP: neither lsof nor /proc/net/tcp is available to list listening sockets")
        return 0
    team = team_mod.Team(ctx.binary, ctx.run_dir, {})
    other = None
    try:
        team_mod.boot(team, os.path.join(ctx.dir, "team.yaml"), ctx.vars)
        v.check("the headless team is running", team.proc.poll() is None)
        v.check("it serves the operator socket", os.path.exists(os.path.join(team.dir, team_mod.SOCK)))
        ports = listeners(team.proc.pid)
        v.check("it binds no TCP port", ports == set(), str(ports))
        log = team.log()
        v.check("it draws no terminal UI", not any(s in log for s in ALT_SCREEN))
        v.check("it serves no canvas page", "the canvas: http" not in log)
        r = team.send("kernel", {"type": "list_agents"})
        v.check("a verb works over the socket with no surface on", r.get("status") == "completed", str(r)[:300])
        code = team.stop()
        v.check("it stops cleanly on SIGTERM", code in (0, -15), str(code))

        # control: with --port the same check must see the listener
        d = os.path.join(ctx.run_dir, "ported")
        os.makedirs(d)
        port = free_port()
        log2 = os.path.join(d, "team.log")
        with open(log2, "wb") as lf:
            other = subprocess.Popen([ctx.binary, "--dir", d, "--port", str(port)], env=team.env,
                                     stdout=lf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, cwd=d)
        end = time.time() + 60
        while time.time() < end and other.poll() is None:
            if "the canvas: http://" in open(log2, errors="replace").read():
                break
            time.sleep(0.3)
        got = listeners(other.pid)
        v.check("control: with --port N the listener check sees port N", str(port) in got, f"{got}, want {port}")
    finally:
        team.stop()
        if other and other.poll() is None:
            other.terminate()
            try:
                other.wait(30)
            except subprocess.TimeoutExpired:
                other.kill()
    return v.finish()
