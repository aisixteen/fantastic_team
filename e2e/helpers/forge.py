"""A fake forge: the GitHub API a team needs, plus git over smart HTTP.

Stdlib only, on 127.0.0.1, port 0. It stands in for api.github.com AND for the
git host: `GET /repos/acme/widgets/installation`, the token mint, one labelled
issue with a `## SPEC` comment, `POST /repos/acme/widgets/pulls` (recorded
verbatim), and `/acme/widgets.git/...` served by `git http-backend` from a bare
repo, only for the minted installation token.

The token is built at run time from pieces: no token-shaped literal is in source.
"""

from __future__ import annotations

import base64
import json
import os
import re
import secrets
import shutil
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

OWNER, NAME = "acme", "widgets"
REPO = f"{OWNER}/{NAME}"
MAINTAINER = "maintainer-login"
APP_LOGIN = "team-bot[bot]"
ISSUE_NUMBER = 1
ISSUE_TITLE = "Add a greeting file"
SPEC = "## SPEC\nAdd a file greeting.txt that says hello, and check it with cat.\n"


def make_token() -> str:
    """An installation token the fake mints, new every run."""
    return "".join(["gh", "s_", secrets.token_hex(18)])


def make_app_key(path: str) -> bool:
    """Write a throwaway RSA private key (PEM) at `path`. False when openssl is absent."""
    exe = shutil.which("openssl")
    if not exe:
        return False
    r = subprocess.run([exe, "genrsa", "-traditional", "-out", path, "2048"],
                       capture_output=True)
    if r.returncode != 0:  # older openssl has no -traditional and already writes PKCS#1
        r = subprocess.run([exe, "genrsa", "-out", path, "2048"], capture_output=True)
    if r.returncode != 0:
        return False
    os.chmod(path, 0o600)
    return True


class Forge:
    def __init__(self, root: str) -> None:
        """`root` holds `acme/widgets.git`, a bare repo made by the caller."""
        self.root = root
        self.token = make_token()
        self.requests: list[dict] = []
        self.extra_issues = 0  # more open issues (2, 3, ...) for the issue listing
        self.lock = threading.Lock()
        forge = self

        class H(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.0"

            def log_message(self, *a):  # quiet
                pass

            def do_GET(self):
                forge._handle(self)

            def do_POST(self):
                forge._handle(self)

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.httpd.server_address[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self.host = f"127.0.0.1:{self.port}"
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def start(self) -> "Forge":
        self.thread.start()
        return self

    def stop(self) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()

    # --- what a probe reads -------------------------------------------------
    def to(self, method: str, path: str) -> list[dict]:
        return [r for r in self.requests if r["method"] == method and r["path"] == path]

    def pulls(self) -> list[dict]:
        return [dict(r, json=json.loads(r["body"])) for r in self.to("POST", f"/repos/{REPO}/pulls")]

    # --- the wire -----------------------------------------------------------
    def _handle(self, h: BaseHTTPRequestHandler) -> None:
        u = urlsplit(h.path)
        n = int(h.headers.get("Content-Length") or 0)
        chunked = "chunked" in (h.headers.get("Transfer-Encoding") or "").lower()
        raw = _read_chunked(h.rfile) if chunked else (h.rfile.read(n) if n else b"")
        rec = {"method": h.command, "path": u.path, "query": u.query,
               "headers": {k.lower(): v for k, v in h.headers.items()},
               "body": raw.decode("utf-8", "replace") if not u.path.startswith(f"/{REPO}.git") else "",
               "body_len": len(raw)}
        with self.lock:
            self.requests.append(rec)
        if u.path.startswith(f"/{REPO}.git/"):
            return self._git(h, u, raw, rec)
        status, body = self._api(h.command, u.path, parse_qs(u.query), raw)
        data = json.dumps(body).encode()
        h.send_response(status)
        h.send_header("Content-Type", "application/json")
        h.send_header("Content-Length", str(len(data)))
        h.end_headers()
        h.wfile.write(data)

    def _api(self, method: str, path: str, q: dict, raw: bytes):
        r = f"/repos/{REPO}"
        if method == "GET" and path == f"{r}/installation":
            return 200, {"id": 42}
        if method == "POST" and path == "/app/installations/42/access_tokens":
            return 201, {"token": self.token, "expires_at": "2099-01-01T00:00:00Z"}
        if method == "GET" and path == r:
            return 200, {"full_name": REPO, "default_branch": "main"}
        if method == "GET" and path == f"{r}/contents/.github/team.yaml":
            text = "task_labels:\n  - team-ready\n"
            return 200, {"encoding": "base64", "content": base64.b64encode(text.encode()).decode()}
        if method == "GET" and path == f"{r}/issues":
            issues = [{"number": ISSUE_NUMBER, "title": ISSUE_TITLE, "state": "open",
                       "labels": [{"name": "team-ready"}]}]
            # a test may set `forge.extra_issues = n` for n more open issues (numbers 2..)
            issues += [{"number": ISSUE_NUMBER + i, "title": f"{ISSUE_TITLE} {i + 1}", "state": "open",
                        "labels": [{"name": "team-ready"}]} for i in range(1, self.extra_issues + 1)]
            return 200, issues
        if method == "GET" and path == f"{r}/issues/{ISSUE_NUMBER}":
            return 200, {"number": ISSUE_NUMBER, "title": ISSUE_TITLE, "state": "open",
                         "body": "Please add a greeting.", "labels": [{"name": "team-ready"}]}
        if method == "GET" and path == f"{r}/issues/{ISSUE_NUMBER}/comments":
            return 200, [{"user": {"login": MAINTAINER}, "created_at": "2026-01-01T00:00:00Z",
                          "body": SPEC}]
        if method == "GET" and path == f"{r}/issues/{ISSUE_NUMBER}/events":
            return 200, [{"event": "labeled", "label": {"name": "team-ready"},
                          "actor": {"login": MAINTAINER}, "created_at": "2026-01-01T00:00:00Z"}]
        if method == "GET" and path == f"{r}/pulls":
            return 200, []
        if method == "POST" and path == f"{r}/pulls":
            return 201, {"number": 7, "html_url": f"https://forge.invalid/{REPO}/pull/7",
                         "draft": True}
        return 404, {"message": "Not Found"}

    def _git(self, h, u, raw: bytes, rec: dict) -> None:
        want = "Basic " + base64.b64encode(f"x-access-token:{self.token}".encode()).decode()
        rec["authorized"] = h.headers.get("Authorization") == want
        if not rec["authorized"]:
            h.send_response(401)
            h.send_header("WWW-Authenticate", 'Basic realm="forge"')
            h.send_header("Content-Length", "0")
            h.end_headers()
            return
        env = {"PATH": os.environ.get("PATH", ""), "GIT_PROJECT_ROOT": self.root,
               "GIT_HTTP_EXPORT_ALL": "1", "REQUEST_METHOD": h.command,
               "PATH_INFO": u.path, "QUERY_STRING": u.query, "REMOTE_USER": "x-access-token",
               "REMOTE_ADDR": "127.0.0.1", "CONTENT_LENGTH": str(len(raw)),
               "CONTENT_TYPE": h.headers.get("Content-Type", ""),
               "GIT_CONFIG_NOSYSTEM": "1", "HOME": self.root}
        for k in ("Content-Encoding", "Git-Protocol"):
            if h.headers.get(k):
                env["HTTP_" + k.upper().replace("-", "_")] = h.headers[k]
        out = subprocess.run(["git", "http-backend"], input=raw, env=env, capture_output=True).stdout
        head, _, body = out.partition(b"\r\n\r\n")
        if not _:
            head, _, body = out.partition(b"\n\n")
        status = 200
        headers = []
        for line in head.decode("latin-1").splitlines():
            k, _, v = line.partition(":")
            if k.lower() == "status":
                status = int(v.strip().split()[0])
            elif k:
                headers.append((k, v.strip()))
        h.send_response(status)
        for k, v in headers:
            h.send_header(k, v)
        h.send_header("Content-Length", str(len(body)))
        h.end_headers()
        h.wfile.write(body)


def _read_chunked(rfile) -> bytes:
    out = b""
    while True:
        size = int(rfile.readline().strip().split(b";")[0] or b"0", 16)
        if size == 0:
            rfile.readline()
            return out
        out += rfile.read(size)
        rfile.readline()


def make_bare(root: str) -> str:
    """`<root>/acme/widgets.git`: a bare repo that accepts pushes over HTTP."""
    bare = os.path.join(root, OWNER, NAME + ".git")
    os.makedirs(os.path.dirname(bare), exist_ok=True)
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", bare], check=True)
    subprocess.run(["git", "-C", bare, "config", "http.receivepack", "true"], check=True)
    return bare


ENV_GIT = {"GIT_CONFIG_NOSYSTEM": "1", "GIT_TERMINAL_PROMPT": "0"}


def seed_clone(bare: str, clone: str, author: str = "Seed <seed@example.invalid>") -> None:
    """A clone of `bare` with one commit on main, pushed back (plain file path)."""
    name, _, email = author.partition(" <")
    email = email.rstrip(">")
    env = dict(os.environ, **ENV_GIT, GIT_AUTHOR_NAME=name, GIT_AUTHOR_EMAIL=email,
               GIT_COMMITTER_NAME=name, GIT_COMMITTER_EMAIL=email)
    run = lambda *a, cwd=None: subprocess.run(["git", *a], cwd=cwd, env=env, check=True,
                                              capture_output=True)
    run("clone", "-q", bare, clone)
    run("checkout", "-q", "-B", "main", cwd=clone)
    with open(os.path.join(clone, "README.md"), "w") as f:
        f.write("# widgets\n")
    run("add", "-A", cwd=clone)
    run("commit", "-q", "-m", "seed", cwd=clone)
    run("push", "-q", "origin", "main", cwd=clone)
