"""HTTP and websocket, stdlib only, for the tests that drive the team's web UI.

`WsClient` speaks RFC 6455 to the page's socket leg (`/team_ws/ws`) with the same
bytes the page's shim sends: `{type:call, target, payload, id}` out, `{type:reply,
id, data}` and `{type:event, src, payload}` back.
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


def http_get(url: str, timeout: float = 10.0) -> tuple[int, bytes]:
    """`(status, body)`; a 4xx/5xx is a status, a refused connection raises OSError."""
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except urllib.error.URLError as e:
        raise OSError(str(e.reason)) from None


def up(port: int, path: str = "/", host: str = "127.0.0.1") -> bool:
    """True when something answers HTTP 200 on the port."""
    try:
        return http_get(f"http://{host}:{port}{path}", timeout=3)[0] == 200
    except OSError:
        return False


def refuses(port: int, host: str = "127.0.0.1") -> bool:
    """True when nothing accepts a connection on the port."""
    try:
        socket.create_connection((host, port), timeout=2).close()
        return False
    except OSError:
        return True


def encode_frame(payload: bytes, opcode: int = 0x1) -> bytes:
    """One masked client frame (a client must mask); 7-bit, 16-bit or 64-bit length."""
    n = len(payload)
    if n < 126:
        head = struct.pack("!BB", 0x80 | opcode, 0x80 | n)
    elif n < 65536:
        head = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, n)
    else:
        head = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, n)
    mask = os.urandom(4)
    return head + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload))


class WsClient:
    def __init__(self, port: int, path: str = "/team_ws/ws", host: str = "127.0.0.1",
                 timeout: float = 10.0) -> None:
        self.sock = socket.create_connection((host, port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n"
                           f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                           "Sec-WebSocket-Version: 13\r\n\r\n").encode())
        head = b""
        while not head.endswith(b"\r\n\r\n"):
            b = self.sock.recv(1)
            if not b:
                raise OSError("websocket handshake closed: " + head.decode(errors="replace"))
            head += b
        if not head.startswith(b"HTTP/1.1 101"):
            raise OSError("websocket handshake refused: " + head.decode(errors="replace"))
        self.buf = b""
        self.n = 0
        self.tag = os.urandom(3).hex()  # call ids are unique across connections: the leg is one principal
        self.seen: list[dict] = []  # every frame read, in order

    def _need(self, n: int, deadline: float) -> bytes:
        while len(self.buf) < n:
            left = deadline - time.time()
            if left <= 0:
                raise TimeoutError("websocket read timed out")
            self.sock.settimeout(left)
            try:
                chunk = self.sock.recv(65536)
            except socket.timeout:
                raise TimeoutError("websocket read timed out") from None
            if not chunk:
                raise OSError("websocket closed")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def recv_text(self, timeout: float = 10.0) -> str:
        """The next text frame (control frames are skipped)."""
        deadline = time.time() + timeout
        while True:
            b0, b1 = self._need(2, deadline)
            n = b1 & 0x7F
            if n == 126:
                n = struct.unpack("!H", self._need(2, deadline))[0]
            elif n == 127:
                n = struct.unpack("!Q", self._need(8, deadline))[0]
            mask = self._need(4, deadline) if b1 & 0x80 else None
            body = self._need(n, deadline)
            if mask:
                body = bytes(b ^ mask[i % 4] for i, b in enumerate(body))
            op = b0 & 0x0F
            if op == 0x1:
                return body.decode()
            if op == 0x8:
                raise OSError("websocket closed by the server")

    def send_json(self, obj: dict) -> None:
        self.sock.sendall(encode_frame(json.dumps(obj).encode()))

    def _read(self, timeout: float) -> dict:
        f = json.loads(self.recv_text(timeout))
        self.seen.append(f)
        return f

    def send_call(self, target: str, payload: dict) -> str:
        """Send a call frame and return its id; read the reply with `wait_reply`."""
        self.n += 1
        cid = f"{self.tag}-{self.n}"
        self.send_json({"type": "call", "target": target, "payload": payload, "id": cid})
        return cid

    def wait_reply(self, cid: str, timeout: float = 20.0) -> dict:
        """The `data` of the reply with this id (a refusal is `{"error": ..., "refused": True}`).
        Other frames met on the way are kept in `self.seen`."""
        deadline = time.time() + timeout
        done = 0
        while True:
            for f in self.seen[done:]:
                if f.get("id") == cid and f.get("type") == "reply":
                    return f.get("data")
                if f.get("id") == cid and f.get("type") == "error":
                    return {"error": str(f.get("error") or "refused"), "refused": True}
            done = len(self.seen)
            self._read(max(0.1, deadline - time.time()))

    def call(self, target: str, payload: dict, timeout: float = 20.0) -> dict:
        """Send a call, return the `data` of its reply. `event` frames met on the way
        are kept in `self.seen` for `events()`."""
        return self.wait_reply(self.send_call(target, payload), timeout)

    def watch(self, src: str) -> None:
        """Ask for `src`'s events; they arrive as `event` frames (the page's shim sends
        the same frame and waits for no reply)."""
        self.send_json({"type": "watch", "src": src})

    def events(self, src: str | None = None, timeout: float = 0.0) -> list[dict]:
        """The `event` frames read so far (from `src` when given); reads more for
        `timeout` seconds first."""
        end = time.time() + timeout
        while time.time() < end:
            try:
                self._read(max(0.05, end - time.time()))
            except TimeoutError:
                break
        return [f for f in self.seen if f.get("type") == "event"
                and (src is None or f.get("src") == src)]

    def wait_event(self, pred, timeout: float = 30.0) -> bool:
        """Read frames until `pred(event_frame)` holds for one (already read ones count)."""
        end = time.time() + timeout
        while True:
            if any(pred(f) for f in self.seen if f.get("type") == "event"):
                return True
            if time.time() >= end:
                return False
            try:
                self._read(max(0.05, end - time.time()))
            except TimeoutError:
                return False

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass
