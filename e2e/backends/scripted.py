"""The `scripted` plug: a fake of Anthropic's Messages wire that plays a script.

`POST /v1/messages` answers as SSE (message_start, content_block_start, ping,
text_delta x2, content_block_stop, message_delta, message_stop: the frame shapes
of fantastic's e2e/mocks/anthropic.py, copied, not imported). The team binary's
anthropic backend sends no `tools` array: a model calls a tool by writing a
`<call name="send">` element in its text. So a script turn is text, and a turn
may list `calls` that are rendered into that element form.

A script is a list of turns. A turn is one of:
  {"text": "words"}
  {"calls": [{"target": "git", "type": "worktree_add", "args": {"path": "..."}}],
   "say": "optional words before the calls"}
Any turn may also carry `"thinking": "words"`: the stream then opens with a `thinking`
content block (a `thinking_delta` x2, index 0) before the text block (index 1), so a
reasoning part reaches the team. `parse_stream` still returns the text only.
A turn may carry `"delay": seconds`: the plug waits that long before it answers, so a
test can look at the team while a turn is running.
Strings may hold `{{name}}` placeholders, filled from `variables`.

It records every request body and every `x-api-key`; a request past the end of
the script gets a 500, and `exhausted` says so.
"""

from __future__ import annotations

import json
import re
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MODEL = "scripted-model"


def render_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (list, tuple)):
        return "".join(f"<item>{render_value(x)}</item>" for x in v)
    if isinstance(v, dict):
        return "".join(f"<{k}>{render_value(x)}</{k}>" for k, x in v.items())
    return str(v)


def render_call(c: dict) -> str:
    """One `<call name="send">` element, one child tag per argument."""
    parts = [f'<call name="{c.get("name", "send")}">']
    if c.get("target"):
        parts.append(f"<target_id>{c['target']}</target_id>")
    if c.get("type"):
        parts.append(f"<type>{c['type']}</type>")
    for k, v in (c.get("args") or {}).items():
        parts.append(f"<{k}>{render_value(v)}</{k}>")
    parts.append("</call>")
    return "\n".join(parts)


def render_turn(turn: dict) -> str:
    if "text" in turn:
        return turn["text"]
    out = [turn["say"]] if turn.get("say") else []
    out += [render_call(c) for c in turn["calls"]]
    return "\n".join(out)


def fill(obj, variables: dict):
    if isinstance(obj, str):
        return re.sub(r"\{\{(\w+)\}\}", lambda m: str(variables[m.group(1)]), obj)
    if isinstance(obj, list):
        return [fill(x, variables) for x in obj]
    if isinstance(obj, dict):
        return {k: fill(x, variables) for k, x in obj.items()}
    return obj


def halves(text: str) -> list[str]:
    mid = len(text) // 2
    return [text[:mid], text[mid:]]


def sse(kind: str, body: dict) -> bytes:
    return f"event: {kind}\ndata: {json.dumps(body)}\n\n".encode()


def stream_frames(text: str, model: str = MODEL, thinking: str | None = None) -> list[bytes]:
    ev = lambda kind, **b: sse(kind, {"type": kind, **b})
    out = [ev("message_start", message={
        "id": "msg_" + uuid.uuid4().hex[:24], "type": "message", "role": "assistant",
        "content": [], "model": model, "stop_reason": None, "stop_sequence": None,
        "usage": {"input_tokens": 10, "output_tokens": 1}})]
    at = 0
    if thinking:
        out += [ev("content_block_start", index=0, content_block={"type": "thinking", "thinking": ""})]
        out += [ev("content_block_delta", index=0, delta={"type": "thinking_delta", "thinking": p})
                for p in halves(thinking) if p]
        out += [ev("content_block_stop", index=0)]
        at = 1
    out += [ev("content_block_start", index=at, content_block={"type": "text", "text": ""}),
            ev("ping")]
    out += [ev("content_block_delta", index=at, delta={"type": "text_delta", "text": p})
            for p in halves(text) if p]
    out += [ev("content_block_stop", index=at),
            ev("message_delta", delta={"stop_reason": "end_turn", "stop_sequence": None},
               usage={"output_tokens": max(1, len(text) // 4)}),
            ev("message_stop")]
    return out


def parse_stream(raw: bytes) -> str:
    """The text a stream carries (what a plain client decodes). For the self-test."""
    text = ""
    for block in raw.decode().split("\n\n"):
        for line in block.splitlines():
            if line.startswith("data: "):
                d = json.loads(line[6:])
                if d.get("type") == "content_block_delta":
                    text += d["delta"].get("text", "")
    return text


class ScriptedModel:
    model_name = MODEL  # what team.yaml's `{{model_name}}` takes
    key_env = None  # no real key
    real = False  # a script answers; nothing is sent upstream

    def __init__(self, script: list[dict], variables: dict | None = None) -> None:
        self.turns = [render_turn(fill(t, variables or {})) for t in script]
        self.thinking = [fill(t, variables or {}).get("thinking") for t in script]
        self.delays = [float(t.get("delay") or 0) for t in script]
        self.requests: list[dict] = []  # parsed JSON bodies, in order
        self.raw_bodies: list[str] = []
        self.keys: list[str | None] = []  # x-api-key of each POST /v1/messages
        self.served = 0
        self.overrun = 0
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

            def do_GET(self):
                if self.path.startswith("/v1/models/"):
                    self._send(200, json.dumps({
                        "type": "model", "id": self.path.rsplit("/", 1)[-1],
                        "display_name": MODEL, "created_at": "2026-01-01T00:00:00Z",
                        "max_input_tokens": 200000, "max_tokens": 8192,
                        "capabilities": None, "lifecycle": "active"}).encode())
                else:
                    self._send(404, b'{"type":"error","error":{"type":"not_found_error","message":"no"}}')

            def do_POST(self):
                n = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(n).decode("utf-8", "replace")
                if self.path != "/v1/messages":
                    return self._send(404, b'{"type":"error","error":{"type":"not_found_error","message":"no"}}')
                with outer.lock:
                    outer.raw_bodies.append(raw)
                    try:
                        outer.requests.append(json.loads(raw))
                    except ValueError:
                        outer.requests.append({})
                    outer.keys.append(self.headers.get("x-api-key"))
                    i = outer.served
                    outer.served += 1
                    if i >= len(outer.turns):
                        outer.overrun += 1
                if i >= len(outer.turns):
                    return self._send(500, json.dumps({"type": "error", "error": {
                        "type": "api_error", "message": "scripted model: script exhausted"}}).encode())
                if outer.delays[i]:
                    time.sleep(outer.delays[i])
                data = b"".join(stream_frames(outer.turns[i], thinking=outer.thinking[i]))
                self._send(200, data, "text/event-stream; charset=utf-8")

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.httpd.server_address[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self.host = f"127.0.0.1:{self.port}"
        self.endpoint = self.url + "/v1/messages"
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def start(self) -> "ScriptedModel":
        self.thread.start()
        return self

    def stop(self) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()

    @property
    def consumed(self) -> bool:
        return self.served == len(self.turns) and self.overrun == 0

    @property
    def errored(self) -> bool:
        return self.overrun > 0


def load(path: str, variables: dict | None = None) -> ScriptedModel:
    with open(path) as f:
        return ScriptedModel(json.load(f), variables)
