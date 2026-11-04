"""The team page without a browser: `page_driver.js` runs the served page's own script on a stub DOM,
bridged to the live /team_ws/ws socket of the team (the page's real leg, with its real grants).
Needs node >= 22 (a global WebSocket); a test without it SKIPs."""

import json
import os
import re
import shutil
import subprocess

DRIVER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "page_driver.js")


def node_ok() -> bool:
    node = shutil.which("node")
    if not node:
        return False
    out = subprocess.run([node, "--version"], capture_output=True, text=True).stdout.strip()
    m = re.match(r"v(\d+)\.", out)
    return bool(m) and int(m.group(1)) >= 22


class PageError(Exception):
    """A step of the page that cannot be done (for example a row that is not drawn)."""


class Page:
    """The node driver: JSON lines in, JSON lines out."""

    def __init__(self, port: int, log_path: str) -> None:
        self.log = open(log_path, "wb")
        self.proc = subprocess.Popen(["node", DRIVER, str(port)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=self.log, text=True)

    def cmd(self, **c) -> dict:
        self.proc.stdin.write(json.dumps(c) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            raise RuntimeError("the page driver exited")
        out = json.loads(line)
        if out.get("error") or out.get("fatal"):
            raise PageError(f"{c.get('cmd')} {c.get('id') or c.get('worker') or ''}: {out.get('error') or out.get('fatal')}")
        return out

    def close(self) -> None:
        try:
            self.proc.stdin.write(json.dumps({"cmd": "quit"}) + "\n")
            self.proc.stdin.flush()
        except OSError:
            pass
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()
