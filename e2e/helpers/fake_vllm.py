"""A fake vLLM server: the routes `vllm_backend` calls, nothing more (stdlib only).

- `GET  {base}/v1/models`            the served model and its `max_model_len` (a record that leaves
                                      `context_window` out reads it here before its first turn)
- `POST {base}/v1/chat/completions`  an SSE stream of `chat.completion.chunk` frames: a role frame,
                                      the text in two content frames (a `reasoning_content` frame
                                      first when the turn has `thinking`), a `finish_reason` frame,
                                      the usage-only frame the backend counts tokens from
                                      (`stream_options.include_usage`, `choices: []`), `[DONE]`.

What it answers is a script of the scripted plug (`backends/scripted.py`: turns, lanes, `after`,
`{{id:N}}`) or one fixed text. With `key` it wants `Authorization: Bearer <key>` and answers 401
without it, like the shared GPU's door. It never sees a GPU and is the stand-in for
`VLLM_ENDPOINT` in the selftest and in a dry run of the gpu tests:

    python3 e2e/helpers/fake_vllm.py --script e2e/tests/cli/single_lead/script.json
    VLLM_ENDPOINT=<printed url> FANTASTIC_AI_MODEL=fake VLLM_KV_SLOTS=2 VLLM_KV_OVERFLOW=evict \\
        python3 e2e/run.py --only gpu_cli/single_lead --bin ABS_PATH
"""

from __future__ import annotations

import json
import os
import sys
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

from backends import scripted  # noqa: E402

MODEL = "fake-model"
WINDOW = 262144


def halves(text: str) -> list[str]:
    mid = len(text) // 2
    return [p for p in (text[:mid], text[mid:]) if p]


def frames(text: str, model: str, thinking: str | None, usage: dict) -> list[bytes]:
    """The SSE body of one reply, as vLLM streams it."""
    cid = "chatcmpl-" + uuid.uuid4().hex[:16]

    def chunk(choices, **extra):
        body = {"id": cid, "object": "chat.completion.chunk", "created": 1, "model": model,
                "choices": choices, **extra}
        return f"data: {json.dumps(body)}\n\n".encode()

    out = [chunk([{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": None}])]
    if thinking:
        out += [chunk([{"index": 0, "delta": {"reasoning_content": p}, "finish_reason": None}])
                for p in halves(thinking)]
    out += [chunk([{"index": 0, "delta": {"content": p}, "finish_reason": None}]) for p in halves(text)]
    out += [chunk([{"index": 0, "delta": {}, "finish_reason": "stop"}]),
            chunk([], usage=usage),
            b"data: [DONE]\n\n"]
    return out


class FakeVllm:
    def __init__(self, script: list[dict] | None = None, text: str = "ok", key: str | None = None,
                 model: str = MODEL, variables: dict | None = None) -> None:
        self.script = scripted.ScriptedModel(script, variables) if script is not None else None
        self.text, self.key, self.model = text, key, model
        self.requests: list[dict] = []  # parsed chat bodies, in order
        self.auths: list[str | None] = []  # the Authorization header of each chat request
        self.refused = 0  # 401s
        self.model_reads = 0
        self.prompt_tokens = 0
        self.completion_tokens = 0
        self.lock = threading.Lock()
        outer = self

        class H(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.0"

            def log_message(self, *a):
                pass

            def _send(self, status, body: bytes, ctype="application/json"):
                self.send_response(status)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def _authorized(self) -> bool:
                if outer.key is None or self.headers.get("Authorization") == f"Bearer {outer.key}":
                    return True
                with outer.lock:
                    outer.refused += 1
                self._send(401, b'{"error":{"message":"missing or wrong key","type":"auth"}}')
                return False

            def do_GET(self):
                if self.path.rstrip("/") != "/v1/models":
                    return self._send(404, b'{"error":{"message":"no"}}')
                if not self._authorized():
                    return
                with outer.lock:
                    outer.model_reads += 1
                self._send(200, json.dumps({"object": "list", "data": [
                    {"id": outer.model, "object": "model", "max_model_len": WINDOW}]}).encode())

            def do_POST(self):
                n = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(n).decode("utf-8", "replace")
                if self.path != "/v1/chat/completions":
                    return self._send(404, b'{"error":{"message":"no"}}')
                if not self._authorized():
                    return
                auth = self.headers.get("Authorization")
                if outer.script is not None:
                    played = outer.script.play(raw, None)
                    if played is None:
                        return self._send(500, b'{"error":{"message":"fake vllm: script exhausted"}}')
                    text, thinking = played
                else:
                    text, thinking = outer.text, None
                usage = {"prompt_tokens": max(1, len(raw) // 4), "completion_tokens": max(1, len(text) // 4)}
                usage["total_tokens"] = usage["prompt_tokens"] + usage["completion_tokens"]
                with outer.lock:
                    try:
                        outer.requests.append(json.loads(raw))
                    except ValueError:
                        outer.requests.append({})
                    outer.auths.append(auth)
                    outer.prompt_tokens += usage["prompt_tokens"]
                    outer.completion_tokens += usage["completion_tokens"]
                self._send(200, b"".join(frames(text, outer.model, thinking, usage)),
                           "text/event-stream; charset=utf-8")

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.httpd.server_address[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self.endpoint = self.url + "/v1"
        self.host = f"127.0.0.1:{self.port}"
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    @property
    def served(self) -> int:
        with self.lock:
            return len(self.requests)

    def start(self) -> "FakeVllm":
        self.thread.start()
        return self

    def stop(self) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()
        if self.script is not None:
            self.script.httpd.server_close()  # the scripted model's own socket, never served


def parse_chat_stream(raw: bytes) -> dict:
    """What a streaming client reads off the body: the text, the reasoning, the finish reason and
    the usage frame (the way `vllm_backend` decodes it)."""
    out = {"text": "", "reasoning": "", "finish": None, "usage": None, "done": False}
    for line in raw.decode().splitlines():
        if not line.startswith("data:"):
            continue
        data = line[5:].strip()
        if data == "[DONE]":
            out["done"] = True
            continue
        obj = json.loads(data)
        if obj.get("usage"):
            out["usage"] = obj["usage"]
        for choice in obj.get("choices") or []:
            d = choice.get("delta") or {}
            out["text"] += d.get("content") or ""
            out["reasoning"] += d.get("reasoning_content") or ""
            out["finish"] = choice.get("finish_reason") or out["finish"]
    return out


def main() -> int:
    import argparse
    import time

    ap = argparse.ArgumentParser(description="a fake vLLM /v1 server for a dry run of the gpu tests")
    ap.add_argument("--script", help="a scripted-plug script.json to play (default: answer `ok`)")
    ap.add_argument("--key-env", help="the name of a variable holding the door key to require")
    a = ap.parse_args()
    script = json.load(open(a.script)) if a.script else None
    key = os.environ.get(a.key_env) if a.key_env else None
    srv = FakeVllm(script=script, key=key).start()
    print(srv.endpoint, flush=True)
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        srv.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
