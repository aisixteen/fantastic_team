"""Talk to the team's web UI from a probe: a free port, a plain GET, and the page's
websocket leg (`/team_ws/ws`). Stdlib only.

The frames are the ones web/page_shim.js uses: {type:'call', target, payload, id} is
answered by {type:'reply', id, data} or {type:'error', id, error}.
"""

from __future__ import annotations

import base64
import json
import os
import socket
import struct
import time
import urllib.error
import urllib.request


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def http_get(port: int, path: str, timeout: float = 10.0) -> tuple[int, str]:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{port}{path}", timeout=timeout) as r:
            return r.status, r.read().decode(errors="replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace")


def wait_http(port: int, path: str, wait_s: float = 30.0) -> bool:
    """True once GET path answers 200."""
    end = time.time() + wait_s
    while time.time() < end:
        try:
            if http_get(port, path, 3.0)[0] == 200:
                return True
        except OSError:
            pass
        time.sleep(0.3)
    return False


class WsLeg:
    """One websocket to the page's leg; `call` is what `fantastic.send` does in the page."""

    def __init__(self, port: int, path: str = "/team_ws/ws", timeout: float = 20.0) -> None:
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
                           f"Upgrade: websocket\r\nConnection: Upgrade\r\n"
                           f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
                           f"Origin: http://127.0.0.1:{port}\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise RuntimeError(f"websocket closed in the handshake: {head!r}")
            head += chunk
        head, _, self.buf = head.partition(b"\r\n\r\n")
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise RuntimeError(f"websocket handshake refused: {head!r}")
        self.n = 0

    def _send(self, text: str) -> None:
        data = text.encode()
        mask = os.urandom(4)
        n = len(data)
        hdr = bytes([0x81])
        if n < 126:
            hdr += bytes([0x80 | n])
        elif n < 65536:
            hdr += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            hdr += bytes([0x80 | 127]) + struct.pack(">Q", n)
        self.sock.sendall(hdr + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def _need(self, n: int) -> bytes:
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RuntimeError("websocket closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _frame(self) -> tuple[int, bytes]:
        b0, b1 = self._need(2)
        n = b1 & 0x7F
        if n == 126:
            n = struct.unpack(">H", self._need(2))[0]
        elif n == 127:
            n = struct.unpack(">Q", self._need(8))[0]
        mask = self._need(4) if b1 & 0x80 else None
        data = self._need(n)
        if mask:
            data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        return b0 & 0x0F, data

    def call(self, target: str, payload: dict, timeout: float = 30.0) -> dict:
        """The reply's `data`. Raises RuntimeError on an `error` frame."""
        self.n += 1
        cid = f"e{self.n}"
        self._send(json.dumps({"type": "call", "target": target, "payload": payload, "id": cid}))
        self.sock.settimeout(timeout)
        while True:
            op, data = self._frame()
            if op == 8:
                raise RuntimeError("websocket closed by the team")
            if op != 1:
                continue
            f = json.loads(data)
            if f.get("id") != cid:
                continue
            if f.get("type") == "error":
                raise RuntimeError(str(f.get("error")))
            return f.get("data")

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass
