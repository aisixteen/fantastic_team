"""The `anthropic` plug: a real model behind a recording relay. Turns tier, run by hand only.

`POST /v1/messages` on 127.0.0.1 is forwarded to the upstream (E2E_UPSTREAM, default
https://api.anthropic.com) with the headers the team sent, its `x-api-key` included.
The team gets the real key from its own credential store (the launcher puts the
key in the team's environment as E2E_MODEL_KEY, and nowhere else). The relay checks
only that a key variable is set; it never reads the value, never stores it and
never writes it to disk.

The relay records what the run cost: the `usage` of every streamed reply
(message_start's input_tokens, message_delta's output_tokens), the request count
and the model. On stop it writes `<run_dir>/cost.json`. It gives a USD estimate only
when E2E_PRICE_IN_PER_MTOK and E2E_PRICE_OUT_PER_MTOK are both set. A hard cap,
E2E_MAX_REQUESTS (default 60), answers 429 past it, so a loop cannot run up the bill.

Environment: E2E_ANTHROPIC_KEY or ANTHROPIC_API_KEY (one must be set, else the test is
skipped), E2E_MODEL (the model's name; default DEFAULT_MODEL), E2E_UPSTREAM,
E2E_MAX_REQUESTS, E2E_PRICE_IN_PER_MTOK, E2E_PRICE_OUT_PER_MTOK.
"""

from __future__ import annotations

import json
import os
import threading
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DEFAULT_MODEL = "claude-haiku-5-5"  # the one place a model is named
DEFAULT_UPSTREAM = "https://api.anthropic.com"
DEFAULT_CAP = 60
KEY_ENVS = ("E2E_ANTHROPIC_KEY", "ANTHROPIC_API_KEY")
HOP_REQUEST = {"host", "content-length", "connection", "accept-encoding", "transfer-encoding",
               "keep-alive"}
HOP_RESPONSE = {"connection", "keep-alive", "transfer-encoding"}


def key_env() -> str | None:
    """The name of the variable that holds the key, or None. The value is not read here."""
    for name in KEY_ENVS:
        if os.environ.get(name):
            return name
    return None


def available() -> bool:
    return key_env() is not None


def parse_usage(raw: bytes) -> tuple[int, int]:
    """(input, output) tokens of one SSE reply: message_start's input, message_delta's output."""
    tin = tout = 0
    for block in raw.decode("utf-8", "replace").split("\n\n"):
        for line in block.splitlines():
            if not line.startswith("data: "):
                continue
            try:
                d = json.loads(line[6:])
            except ValueError:
                continue
            if d.get("type") == "message_start":
                u = d.get("message", {}).get("usage", {})
                tin = int(u.get("input_tokens") or 0)
                tout = int(u.get("output_tokens") or 0)
            elif d.get("type") == "message_delta" and "output_tokens" in d.get("usage", {}):
                tout = int(d["usage"]["output_tokens"] or 0)
    return tin, tout


def _price(name: str) -> float | None:
    v = os.environ.get(name)
    return float(v) if v else None


class AnthropicRelay:
    real = True  # a real model answers: the launcher reads the key for the team

    def __init__(self, variables: dict) -> None:
        self.run_dir = variables["run_dir"]
        self.key_env = key_env()  # the variable's name only
        self.model_name = os.environ.get("E2E_MODEL") or DEFAULT_MODEL
        self.upstream = os.environ.get("E2E_UPSTREAM", DEFAULT_UPSTREAM).rstrip("/")
        self.cap = int(os.environ.get("E2E_MAX_REQUESTS", DEFAULT_CAP))
        self.requests: list[dict] = []  # parsed JSON bodies of POST /v1/messages, in order
        self.raw_bodies: list[str] = []
        self.keys: list[str | None] = []  # x-api-key of each POST /v1/messages
        self.count = 0  # message requests received, refused ones included
        self.served = 0  # replies streamed back with status 200
        self.errors = 0  # upstream errors and cap refusals
        self.usage_in = 0
        self.usage_out = 0
        self.lock = threading.Lock()
        self._stopped = False
        outer = self

        class H(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.0"

            def log_message(self, *a):
                pass

            def _json(self, status, obj):
                body = json.dumps(obj).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def _forward(self, method, body: bytes | None):
                headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP_REQUEST}
                req = urllib.request.Request(outer.upstream + self.path, data=body,
                                             headers=headers, method=method)
                try:
                    resp = urllib.request.urlopen(req, timeout=600)
                except urllib.error.HTTPError as e:
                    resp = e
                except (urllib.error.URLError, OSError) as e:
                    with outer.lock:
                        outer.errors += 1
                    return self._json(502, {"type": "error", "error": {
                        "type": "api_error", "message": f"e2e relay: upstream unreachable: {e.__class__.__name__}"}})
                collected = bytearray()
                self.send_response(resp.status if hasattr(resp, "status") else resp.code)
                for k, v in resp.headers.items():
                    if k.lower() not in HOP_RESPONSE:
                        self.send_header(k, v)
                self.send_header("Connection", "close")
                self.end_headers()
                try:
                    while True:  # stream the upstream body back unchanged
                        chunk = resp.read1(65536) if hasattr(resp, "read1") else resp.read(65536)
                        if not chunk:
                            break
                        collected += chunk
                        self.wfile.write(chunk)
                        self.wfile.flush()
                finally:
                    resp.close()
                return resp.status if hasattr(resp, "status") else resp.code, bytes(collected)

            def do_GET(self):
                if not self.path.startswith("/v1/"):
                    return self._json(404, {"type": "error", "error": {"type": "not_found_error",
                                                                       "message": "e2e relay: not a /v1 path"}})
                self._forward("GET", None)

            def do_POST(self):
                n = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(n)
                if self.path != "/v1/messages":
                    if not self.path.startswith("/v1/"):
                        return self._json(404, {"type": "error", "error": {"type": "not_found_error",
                                                                           "message": "e2e relay: not a /v1 path"}})
                    return self._forward("POST", body)
                with outer.lock:
                    outer.count += 1
                    refused = outer.count > outer.cap
                    outer.keys.append(self.headers.get("x-api-key"))
                    outer.raw_bodies.append(body.decode("utf-8", "replace"))
                    try:
                        outer.requests.append(json.loads(body))
                    except ValueError:
                        outer.requests.append({})
                    if refused:
                        outer.errors += 1
                if refused:
                    return self._json(429, {"type": "error", "error": {
                        "type": "rate_limit_error",
                        "message": f"e2e relay: request cap of {outer.cap} reached (E2E_MAX_REQUESTS)"}})
                result = self._forward("POST", body)
                if result is None:
                    return
                status, data = result
                with outer.lock:
                    if status == 200:
                        tin, tout = parse_usage(data)
                        outer.served += 1
                        outer.usage_in += tin
                        outer.usage_out += tout
                    else:
                        outer.errors += 1

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.httpd.server_address[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self.host = f"127.0.0.1:{self.port}"
        self.endpoint = self.url + "/v1/messages"
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def start(self) -> "AnthropicRelay":
        self.thread.start()
        return self

    def stop(self) -> None:
        if self._stopped:
            return
        self._stopped = True
        self.httpd.shutdown()
        self.httpd.server_close()
        try:
            with open(os.path.join(self.run_dir, "cost.json"), "w") as f:
                json.dump(self.totals(), f, indent=2)
                f.write("\n")
        except OSError:
            pass

    def totals(self) -> dict:
        """What the run cost. Never holds a key."""
        pin, pout = _price("E2E_PRICE_IN_PER_MTOK"), _price("E2E_PRICE_OUT_PER_MTOK")
        usd = None
        if pin is not None and pout is not None:
            usd = round(self.usage_in * pin / 1e6 + self.usage_out * pout / 1e6, 8)
        return {"model": self.model_name, "requests": self.count, "replies": self.served,
                "errors": self.errors, "input_tokens": self.usage_in,
                "output_tokens": self.usage_out, "usd_estimate": usd}

    @property
    def consumed(self) -> bool:
        return self.served >= 1 and self.errors == 0

    @property
    def errored(self) -> bool:
        return self.errors > 0


def make(test_dir: str, variables: dict) -> AnthropicRelay:
    if not available():
        raise RuntimeError("the anthropic plug needs E2E_ANTHROPIC_KEY or ANTHROPIC_API_KEY set")
    return AnthropicRelay(variables)
