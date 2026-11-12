"""The ssh remote of the cli_ssh twins (#242): a local Podman container, reached over ssh.

The container is Linux with `sshd`, `git`, `python3` and the Linux `fantastic_team`, built from the
same source tree inside a Rust container (the Mac binary does not run there). Nothing here is a
team or fantastic verb: it is the box a person would ssh into.

- BUILD, once per source tree (`image()`): a builder image (the toolchain of `rust-toolchain.toml`
  plus the libraries the Linux build links) compiles `fantastic_team` with the source mounted
  read-only and the cargo target and registry under `build_dir()` (bind-mounted, never in the
  container); a runtime image takes the binary. Both are cached by tag: the runtime tag is a hash
  of the source (tracked files outside `e2e/`, uncommitted edits, untracked files), so a second run
  of the same tree builds nothing.
- START (`Remote.start`): a throwaway ed25519 key per run in the run folder (never committed; the
  run folder is gitignored), the container's sshd on a mapped 127.0.0.1 port, a login `team` that
  takes only that key.
- GATEWAY (`gateway_url`): a service of the test on the host's 127.0.0.1 (the fake forge) is
  reached from the container through Podman's host gateway, `host.containers.internal`. Under a
  podman machine (macOS) that name already reaches the host's loopback. Under local rootless
  Podman with pasta (Linux) it reaches the host's outside address, where a 127.0.0.1 service does
  not listen, so the container gets pasta's `--map-host-loopback` on a link-local address and the
  name is pinned to it (`network_args`).
- TEARDOWN (`Remote.stop`): the tunnels the helper opened are closed and the container removed,
  pass or fail (`with Remote(...)`).
- No Podman, or no running machine: `require()` raises `Skip` with the reason.

`python3 e2e/helpers/podman_remote.py --smoke` builds, starts, and walks the remote half of the
weak wiring once (headless kernel, `web` + `web_ws` made with `send` over ssh, the leg reached
through a tunnel, a clone from the host's git server through the gateway, stop, teardown).
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import secrets
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
E2E = os.path.dirname(HERE)
ROOT = os.path.dirname(E2E)

IMAGE = "localhost/fantastic-team-ssh"
BUILDER = "localhost/fantastic-team-ssh-builder"
USER = "team"
GATEWAY = "host.containers.internal"
SSH_PORT = "22/tcp"
LOGIN = "127.0.0.1"
HOST_LOOPBACK = "169.254.1.3"  # in the container, the host's 127.0.0.1 under local rootless pasta

# The builder: the repo's toolchain and what the Linux build needs: D-Bus for fantastic's system
# agents (pkg-config finds it), and Go and zip for fantastic's canvas pack (its build script builds
# esbuild from pinned source when no prebuilt one is there for the platform; zip and unzip pack the page).
BUILDER_FILE = """FROM docker.io/library/rust:{toolchain}-slim-trixie
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libdbus-1-dev git golang-go \\
    zip unzip ca-certificates && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_TOOLCHAIN={toolchain}
"""

# The box: sshd that takes only the run's key, git, python3, the binary. The key comes in the
# environment at start (`AUTHORIZED_KEY`) and is written by the entrypoint.
IMAGE_FILE = """FROM docker.io/library/debian:trixie
RUN apt-get update && apt-get install -y --no-install-recommends openssh-server git python3 \\
    libdbus-1-3 ca-certificates && rm -rf /var/lib/apt/lists/* \\
    && useradd -m -s /bin/bash {user} && mkdir -p /run/sshd && ssh-keygen -A
COPY sshd_config /etc/ssh/sshd_config.d/remote.conf
COPY entrypoint.sh /entrypoint.sh
COPY fantastic_team /usr/local/bin/fantastic_team
RUN chmod 755 /entrypoint.sh /usr/local/bin/fantastic_team
EXPOSE 22
ENTRYPOINT ["/entrypoint.sh"]
"""

SSHD_CONFIG = """PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
PubkeyAuthentication yes
AllowUsers {user}
AllowTcpForwarding local
X11Forwarding no
PermitUserEnvironment no
"""

ENTRYPOINT = """#!/bin/sh
set -eu
[ -n "${{AUTHORIZED_KEY:-}}" ] || {{ echo "no AUTHORIZED_KEY" >&2; exit 2; }}
install -d -m 700 -o {user} -g {user} /home/{user}/.ssh
printf '%s\\n' "$AUTHORIZED_KEY" > /home/{user}/.ssh/authorized_keys
chown {user}:{user} /home/{user}/.ssh/authorized_keys
chmod 600 /home/{user}/.ssh/authorized_keys
unset AUTHORIZED_KEY
exec /usr/sbin/sshd -D -e
"""


class RemoteError(RuntimeError):
    pass


# ---- podman ----------------------------------------------------------------------------

def podman() -> str | None:
    """The podman binary: `E2E_PODMAN`, else on PATH, else the macOS installer's place."""
    given = os.environ.get("E2E_PODMAN")
    if given:
        return given if os.access(given, os.X_OK) else None
    found = shutil.which("podman")
    if found:
        return found
    mac = "/opt/podman/bin/podman"
    return mac if os.access(mac, os.X_OK) else None


def why_not() -> str | None:
    """Why the remote cannot run here, or None: podman, a running machine (`podman info`), and
    ssh / ssh-keygen on this side."""
    exe = podman()
    if not exe:
        return "no podman on this machine"
    for tool in ("ssh", "ssh-keygen"):
        if not shutil.which(tool):
            return f"no {tool} on this machine"
    try:
        r = subprocess.run([exe, "info", "--format", "{{.Host.Arch}}"], capture_output=True, text=True,
                           timeout=60)
    except (OSError, subprocess.TimeoutExpired) as e:
        return f"podman does not answer: {e}"
    if r.returncode != 0:
        return "podman has no running machine: " + (r.stderr.strip().splitlines() or ["podman info failed"])[-1]
    return None


def require() -> None:
    """Raise `Skip` with the reason when the remote cannot run here (the display-only tests' way)."""
    from .verdict import Skip
    why = why_not()
    if why:
        raise Skip(why)


def pm(*args: str, check: bool = True, timeout: float = 600, input: str | None = None) -> subprocess.CompletedProcess:
    exe = podman()
    if not exe:
        raise RemoteError("no podman on this machine")
    r = subprocess.run([exe, *args], capture_output=True, text=True, timeout=timeout, input=input)
    if check and r.returncode != 0:
        raise RemoteError(f"podman {' '.join(args[:3])} failed ({r.returncode}): {(r.stderr or r.stdout)[-1500:]}")
    return r


def network_args() -> list[str]:
    """The `podman run` network options that let `GATEWAY` reach the host's 127.0.0.1. Local
    rootless Podman on pasta: pasta maps the host's loopback to `HOST_LOOPBACK` and the name is
    pinned to it. A podman machine (the service is remote) or another network: none, the machine's
    gateway already reaches the host's loopback."""
    r = pm("info", "--format", "{{.Host.ServiceIsRemote}} {{.Host.Security.Rootless}} {{.Host.RootlessNetworkCmd}}",
           check=False, timeout=60)
    if r.returncode == 0 and r.stdout.split() == ["false", "true", "pasta"]:
        return ["--network", f"pasta:--map-host-loopback,{HOST_LOOPBACK}",
                "--add-host", f"{GATEWAY}:{HOST_LOOPBACK}"]
    return []


def have_image(ref: str) -> bool:
    return pm("image", "exists", ref, check=False, timeout=60).returncode == 0


# ---- the build -------------------------------------------------------------------------

def toolchain(root: str = ROOT) -> str:
    """The channel `rust-toolchain.toml` names."""
    with open(os.path.join(root, "rust-toolchain.toml")) as f:
        for line in f:
            k, _, v = line.partition("=")
            if k.strip() == "channel":
                return v.strip().strip('"')
    raise RemoteError("rust-toolchain.toml names no channel")


def source_tag(root: str = ROOT) -> str:
    """A hash of the source the binary is built from: the tracked files outside `e2e/` as the
    index has them, the uncommitted edits, and the untracked files. The same tree, the same tag."""
    git = lambda *a: subprocess.run(["git", *a], cwd=root, capture_output=True, check=True).stdout
    spec = ["--", ".", ":(exclude)e2e"]
    h = hashlib.sha256()
    h.update(git("ls-files", "-s", *spec))
    h.update(git("diff", "HEAD", "--binary", *spec))
    for name in sorted(git("ls-files", "-o", "--exclude-standard", *spec).decode().splitlines()):
        h.update(name.encode() + b"\0")
        try:
            with open(os.path.join(root, name), "rb") as f:
                h.update(hashlib.sha256(f.read()).digest())
        except OSError:
            pass
    return h.hexdigest()[:16]


def build_dir() -> str:
    """Where the Linux cargo target and registry live (bind-mounted into the builder):
    `E2E_PODMAN_BUILD_DIR`, else `<CARGO_TARGET_DIR or ROOT/target>/podman-linux`."""
    given = os.environ.get("E2E_PODMAN_BUILD_DIR")
    if given:
        return os.path.abspath(given)
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(ROOT, "target")
    return os.path.join(os.path.abspath(target), "podman-linux")


def builder_tag(tool: str) -> str:
    """The builder's tag: the toolchain and a hash of its Containerfile, so a changed recipe builds anew."""
    return f"{tool}-" + hashlib.sha256(BUILDER_FILE.format(toolchain=tool).encode()).hexdigest()[:8]


def builder_argv(root: str, build: str, tool: str) -> list[str]:
    """The `podman run` that compiles the Linux binary: source read-only, target and registry
    in `build`."""
    return ["run", "--rm", "-v", f"{root}:/src:ro", "-v", f"{build}:/build",
            "-e", "CARGO_HOME=/build/cargo-home", "-e", "CARGO_TARGET_DIR=/build/target",
            "-e", "GOPATH=/build/go", "-e", "GOCACHE=/build/go-cache",
            # no debug info: the link of a debug-info binary is killed for memory in a 2 GiB podman machine
            "-e", "CARGO_PROFILE_DEV_DEBUG=0", "-w", "/src", f"{BUILDER}:{builder_tag(tool)}",
            "cargo", "build", "--locked", "-p", "fantastic-team", "--bin", "fantastic_team"]


def image(root: str = ROOT, log=print) -> str:
    """The runtime image of this source tree, built when its tag is missing. Returns its ref."""
    ref = f"{IMAGE}:{source_tag(root)}"
    if have_image(ref):
        log(f"  remote: image {ref} is cached")
        return ref
    tool = toolchain(root)
    started = time.time()
    with tempfile.TemporaryDirectory(dir=build_dir_ready()) as ctxdir:
        builder = f"{BUILDER}:{builder_tag(tool)}"
        if not have_image(builder):
            log(f"  remote: building the builder image {builder}")
            write(ctxdir, "Containerfile", BUILDER_FILE.format(toolchain=tool))
            pm("build", "-t", builder, "-f", os.path.join(ctxdir, "Containerfile"), ctxdir,
               timeout=1800)
        log("  remote: compiling the Linux fantastic_team (cargo, in the builder)")
        build = build_dir_ready()
        pm(*builder_argv(root, build, tool), timeout=5400)
        binary = os.path.join(build, "target", "debug", "fantastic_team")
        if not os.path.isfile(binary):
            raise RemoteError(f"the Linux build left no {binary}")
        shutil.copy2(binary, os.path.join(ctxdir, "fantastic_team"))
        write(ctxdir, "Containerfile", IMAGE_FILE.format(user=USER))
        write(ctxdir, "sshd_config", SSHD_CONFIG.format(user=USER))
        write(ctxdir, "entrypoint.sh", ENTRYPOINT.format(user=USER))
        pm("build", "-t", ref, "-f", os.path.join(ctxdir, "Containerfile"), ctxdir, timeout=1800)
    log(f"  remote: built {ref} in {time.time() - started:.0f}s")
    return ref


def build_dir_ready() -> str:
    d = build_dir()
    os.makedirs(d, exist_ok=True)
    return d


def write(folder: str, name: str, text: str) -> None:
    with open(os.path.join(folder, name), "w") as f:
        f.write(text)


# ---- the key, the gateway, ssh ---------------------------------------------------------

def make_key(folder: str) -> str:
    """A throwaway ed25519 key at `folder/id_remote` (no passphrase, 0600). Returns its path."""
    os.makedirs(folder, exist_ok=True)
    key = os.path.join(folder, "id_remote")
    for p in (key, key + ".pub"):
        if os.path.exists(p):
            os.remove(p)
    subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "e2e-remote", "-f", key],
                   check=True, capture_output=True)
    os.chmod(key, 0o600)
    return key


def gateway_url(url: str) -> str:
    """A URL of a service on this host's 127.0.0.1 (or localhost), as the container dials it."""
    for sep in ("//", "@"):
        for host in ("127.0.0.1", "localhost"):
            url = url.replace(f"{sep}{host}:", f"{sep}{GATEWAY}:")
    return url


def ssh_options(port: int, key: str) -> list[str]:
    """The options of every ssh to the box: the run's key only, no prompt, no host-key file."""
    return ["-p", str(port), "-i", key, "-o", "IdentitiesOnly=yes", "-o", "BatchMode=yes",
            "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
            "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=10"]


def ssh_argv(port: int, key: str, command: str | None = None, forward: tuple[int, int] | None = None) -> list[str]:
    """`ssh -p PORT -i KEY [-L local:127.0.0.1:remote] team@127.0.0.1 [COMMAND]`: the raw command
    an exec agent runs. With `forward` and no command, `-N`: the tunnel alone."""
    argv = ["ssh", *ssh_options(port, key)]
    if forward:
        argv += ["-o", "ExitOnForwardFailure=yes", "-L", f"{forward[0]}:127.0.0.1:{forward[1]}"]
        if command is None:
            argv.append("-N")
    argv.append(f"{USER}@{LOGIN}")
    if command is not None:
        argv.append(command)
    return argv


class Remote:
    """One container for one run. `with Remote(run_dir) as r:` starts it and always removes it."""

    def __init__(self, run_dir: str, image_ref: str | None = None, log=print) -> None:
        self.run_dir = run_dir
        self.image_ref = image_ref
        self.log = log
        self.name = "e2e-remote-" + secrets.token_hex(4)
        self.key: str | None = None
        self.port: int | None = None
        self.tunnels: list[subprocess.Popen] = []
        self.started = False

    def __enter__(self) -> "Remote":
        try:
            self.start()
        except BaseException:
            self.stop()
            raise
        return self

    def __exit__(self, *_):
        self.stop()

    def start(self, wait_s: float = 60.0) -> "Remote":
        ref = self.image_ref or image(log=self.log)
        self.key = make_key(os.path.join(self.run_dir, "remote-key"))
        with open(self.key + ".pub") as f:
            pub = f.read().strip()
        self.started = True  # from here on, stop() removes the container
        self._run(ref, pub)
        self.port = self.mapped_port()
        end = time.time() + wait_s
        while time.time() < end:
            if self.ssh("true", check=False, timeout=20).returncode == 0:
                self.log(f"  remote: {self.name} answers ssh on {LOGIN}:{self.port}")
                return self
            time.sleep(0.5)
        raise RemoteError(f"the container's sshd did not answer in {wait_s}s: {self.logs()[-800:]}")

    def _run(self, ref: str, pub: str) -> None:
        exe = podman()
        env = dict(os.environ, AUTHORIZED_KEY=pub)  # the key's public half, never in an argv
        r = subprocess.run([exe, "run", "-d", "--name", self.name, *network_args(), "-p", f"{LOGIN}::22",
                            "-e", "AUTHORIZED_KEY", ref], capture_output=True, text=True, env=env, timeout=120)
        if r.returncode != 0:
            raise RemoteError(f"podman run failed: {r.stderr[-800:]}")

    def mapped_port(self) -> int:
        out = pm("port", self.name, SSH_PORT, timeout=60).stdout.strip().splitlines()
        if not out:
            raise RemoteError(f"{self.name} maps no port for {SSH_PORT}")
        return int(out[0].rsplit(":", 1)[1])

    def argv(self, command: str | None = None, forward: tuple[int, int] | None = None) -> list[str]:
        return ssh_argv(self.port, self.key, command, forward)

    def ssh(self, command: str, check: bool = True, timeout: float = 120) -> subprocess.CompletedProcess:
        r = subprocess.run(self.argv(command), capture_output=True, text=True, timeout=timeout,
                           stdin=subprocess.DEVNULL)
        if check and r.returncode != 0:
            raise RemoteError(f"ssh {command!r} failed ({r.returncode}): {(r.stderr or r.stdout)[-800:]}")
        return r

    def tunnel(self, local: int, remote: int, wait_s: float = 20.0) -> subprocess.Popen:
        """`ssh -N -L local:127.0.0.1:remote`, kept until `stop`; returns once the local end listens."""
        import socket
        p = subprocess.Popen(self.argv(forward=(local, remote)), stdin=subprocess.DEVNULL,
                             stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.tunnels.append(p)
        end = time.time() + wait_s
        while time.time() < end:
            if p.poll() is not None:
                raise RemoteError(f"the tunnel exited: {p.stderr.read().decode(errors='replace')[-400:]}")
            with socket.socket() as s:
                s.settimeout(1)
                if s.connect_ex(("127.0.0.1", local)) == 0:
                    return p
            time.sleep(0.2)
        raise RemoteError(f"the tunnel's local end {local} did not listen in {wait_s}s")

    def logs(self) -> str:
        r = pm("logs", self.name, check=False, timeout=60)
        return (r.stdout or "") + (r.stderr or "")

    def exists(self) -> bool:
        return pm("container", "exists", self.name, check=False, timeout=60).returncode == 0

    def listening(self) -> set[int]:
        """The TCP ports the box listens on (its /proc/net/tcp and tcp6, LISTEN rows)."""
        rows = self.ssh("cat /proc/net/tcp /proc/net/tcp6 2>/dev/null | awk '$4==\"0A\"{print $2}'",
                        check=False).stdout.split()
        return {int(a.rsplit(":", 1)[1], 16) for a in rows}

    def stop(self) -> None:
        """Close the tunnels, remove the container (pass or fail). Safe to call twice."""
        for p in self.tunnels:
            if p.poll() is None:
                p.terminate()
                try:
                    p.wait(10)
                except subprocess.TimeoutExpired:
                    p.kill()
                    p.wait()
        self.tunnels = []
        if self.started:
            pm("rm", "-f", "-t", "5", self.name, check=False, timeout=120)
            self.started = False


# ---- the smoke run ---------------------------------------------------------------------

def smoke() -> int:
    """Build, start, and walk the remote half of the weak wiring once; print each step's time."""
    sys.path.insert(0, E2E)
    from helpers import forge as forge_mod, web
    from helpers.team import free_port
    why = why_not()
    if why:
        print(f"SKIP podman remote smoke ({why})")
        return 0
    t0 = time.time()
    run_dir = os.path.join(E2E, "runs", "podman-smoke-" + time.strftime("%Y%m%d-%H%M%S"))
    os.makedirs(run_dir)
    bare = forge_mod.make_bare(os.path.join(run_dir, "forge"))
    forge_mod.seed_clone(bare, os.path.join(run_dir, "clone"))
    forge = forge_mod.Forge(os.path.join(run_dir, "forge")).start()
    ref = image()
    print(f"image ready: {time.time() - t0:.0f}s")
    failed = []

    def check(label, ok, detail=""):
        print(f"  [{'ok' if ok else 'FAIL'}] {label}" + ("" if ok else f"  {detail}"))
        if not ok:
            failed.append(label)

    try:
        with Remote(run_dir, ref) as r:
            t1 = time.time()
            # the tunnel's two ends are one port number: `web` admits a Host naming its own port only
            # (fantastic's host guard), and a client of the local end sends `127.0.0.1:<local>`
            n = free_port()
            r.ssh("mkdir -p w && cd w && { nohup fantastic_team --headless > team.log 2>&1 & echo $! > team.pid; } "
                  "&& for i in $(seq 100); do [ -f .fantastic/cli/mcp.yaml ] && exit 0; sleep 0.2; done; exit 1")
            send = lambda item: r.ssh("cd w && fantastic_team send " + shq(item)).stdout
            out = send(json.dumps({"target": "kernel", "payload": {
                "type": "create_agent", "id": "web", "parent_id": "user", "handler_module": "web.tools",
                "port": n}}))
            check("send over ssh made the remote `web` host", '"completed"' in out, out[-300:])
            out = send(json.dumps({"target": "kernel", "payload": {
                "type": "create_agent", "id": "remote_ws", "parent_id": "web", "handler_module": "web_ws.tools",
                "ingress_rule": "allow_all_ingress"}}))
            check("send over ssh made its `web_ws` leg", '"completed"' in out, out[-300:])
            # the canvas's edges (fantastic `src/tui/src/kern/web.rs` `ensure`): `web` mounts the leg
            # (`get_routes`, both ends); the leg reaches `user` alone, whose `forward` makes the call
            for item in ({"type": "update_agent", "id": "remote_ws",
                          "callable_by": {"$self": "*", "web": ["get_routes"]},
                          "can_call": {"$self": "*", "user": ["forward"]}},
                         {"type": "update_agent", "id": "web", "can_call": {"$self": "*", "remote_ws": ["get_routes"]}}):
                out = send(json.dumps({"target": "kernel", "payload": item}))
                check(f"update_agent {item['id']} (the canvas's edges)", '"completed"' in out, out[-300:])
            user = json.loads(send(json.dumps({"target": "kernel", "payload": {"type": "list_agents"}})))
            cb = {a["id"]: a for a in user["result"]["agents"]}["user"].get("callable_by") or {}
            out = send(json.dumps({"target": "kernel", "payload": {
                "type": "update_agent", "id": "user", "callable_by": dict(cb, remote_ws=["forward"])}}))
            check("update_agent user: the leg may ask it to forward", '"completed"' in out, out[-300:])
            out = send(json.dumps({"target": "web", "payload": {"type": "boot"}}))  # mounts the leg, listens
            check("boot web over ssh: it mounts the leg and listens", '"remote_ws"' in out, out[-300:])
            local = n
            r.tunnel(local, n)
            reply = None
            try:
                ws = web.WsClient(local, path="/remote_ws/ws")
                reply = ws.call("user", {"type": "forward", "target": "kernel", "payload": {"type": "list_agents"}})
                ws.close()
            except (OSError, RuntimeError) as e:
                print(f"  socket: {e}")
            ids = {a.get("id") for a in ((reply or {}).get("agents") or ((reply or {}).get("result") or {}).get("agents") or [])}
            check("through the tunnel, the leg's /remote_ws/ws carries a call: user.forward lists the remote agents",
                  {"web", "remote_ws", "user"} <= ids, str(reply)[:400])
            url = gateway_url(f"http://127.0.0.1:{forge.httpd.server_address[1]}/{forge_mod.REPO}.git")
            auth = base64.b64encode(f"x-access-token:{forge.token}".encode()).decode()
            header = shq(f"http.extraHeader=Authorization: Basic {auth}")  # the token stays out of the URL
            c = r.ssh(f"git -c {header} clone -q {shq(url)} w/repo && git -C w/repo log --format=%s", check=False)
            check("git clone of the host's git server through the gateway", c.returncode == 0 and "seed" in c.stdout,
                  (c.stderr or c.stdout)[-300:])
            ports = sorted(r.listening())
            check("the box listens on sshd and the web host's port only", set(ports) <= {22, n}, str(ports))
            stop = r.ssh(stop_line("w"), check=False)
            check("the remote kernel stops on SIGTERM over ssh", stop.returncode == 0, (stop.stderr or stop.stdout)[-300:])
            print(f"remote walk: {time.time() - t1:.0f}s")
        check("the container is removed", not r.exists())
    finally:
        forge.stop()
    print(f"podman remote smoke: {'FAIL' if failed else 'PASS'} in {time.time() - t0:.0f}s")
    return 1 if failed else 0


# The binary has no `stop` word: the kernel started with `echo $! > team.pid` is stopped the way
# `helpers/team.py` stops a local one, SIGTERM and a bounded wait for the drain.
STOP = ("cd {dir} && p=$(cat {pid}) && kill -TERM $p && "
        "for i in $(seq {wait}); do kill -0 $p 2>/dev/null || exit 0; sleep 0.2; done; exit 1")


def stop_line(dir: str, pid: str = "team.pid", wait_s: int = 30) -> str:
    """The shell line that SIGTERMs the kernel whose pid is in `dir/pid` and waits for it, `wait_s` at most."""
    return STOP.format(dir=dir, pid=pid, wait=wait_s * 5)


def shq(s: str) -> str:
    return "'" + s.replace("'", "'\\''") + "'"


if __name__ == "__main__":
    if sys.argv[1:] == ["--smoke"]:
        sys.exit(smoke())
    if sys.argv[1:] == ["--build"]:
        print(image())
        sys.exit(0)
    sys.exit("usage: podman_remote.py --smoke | --build")
