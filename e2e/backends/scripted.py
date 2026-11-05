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

LANES. Several agents share one plug, and their requests race. A turn may carry
`"lane": "w1"`: it then belongs to the agent whose FIRST message holds the marker
`[lane:w1]` (a spawn's `task`, or a `send`'s `text`); turns with no `lane` are lane
`main` (an agent whose first message holds no marker). Each lane plays its own turns in
order, whatever the other lanes do. A turn may carry `"after": ["w1", "w2"]`: the plug
holds that answer until those lanes have been served every turn, so one agent works
only once another has finished (a last turn is served after the calls of the turn
before it have run). A script with no `lane` anywhere is the old single ordered script.

SPAWNED IDS. A spawned agent's id is minted (`anthropic_backend_7a308a`), so a script cannot
name it. `{{id:N}}` in a turn is the id in the Nth `<id>...</id>` of the request's body (the
reply of a `spawn`, in the order the agent saw them), filled when the turn is served;
`{{minted:N}}` is the Nth distinct minted id among them (`<handler>_<hex>`), whatever other
`<id>` tags the body holds (a forwarded `create_agent`'s reply names its id too).
Strings may hold `{{name}}` placeholders, filled from `variables`. A name the variables do not
hold is left as written, for `bind`: a test whose values exist only once it runs (the ssh twins'
port, key and tunnel, #242) binds them before the team asks the model.

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
    """`{{name}}` -> `variables[name]`; a name not in `variables` stays as written (see `bind`)."""
    if isinstance(obj, str):
        return re.sub(r"\{\{(\w+)\}\}", lambda m: str(variables.get(m.group(1), m.group(0))), obj)
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
    model_name = MODEL  # what the lead's `model` field takes (`ctx.vars['model_name']`)
    key_env = None  # no real key
    real = False  # a script answers; nothing is sent upstream
    keyed = True  # the agents read a (fake) key from the credential store

    def ai_record(self) -> dict:
        """The fields of an AI record that talks to this plug (the anthropic backend)."""
        return {"handler_module": "anthropic_backend.tools", "model": MODEL, "endpoint": self.endpoint}

    def recipe_args(self) -> list[str]:
        """fantastic's recipe that makes the root AI `ai` on this plug."""
        return ["--headless", "ai", "anthropic", "--model", MODEL, "--endpoint", self.endpoint]

    def recipe_fields(self) -> dict:
        """What the recipe leaves for `update_agent` besides the key (nothing: it names it all)."""
        return {}

    def __init__(self, script: list[dict], variables: dict | None = None) -> None:
        self.script = script
        self.variables = dict(variables or {})
        self.render()
        self.delays = [float(t.get("delay") or 0) for t in script]
        filled = [fill(t, self.variables) for t in script]
        self.laned = any("lane" in t for t in filled)
        self.lanes = [t.get("lane", "main") for t in filled]
        self.after = [list(t.get("after") or []) for t in filled]
        self.cursor: dict[str, int] = {}  # lane -> turns served
        self.request_lanes: list[str] = []  # the lane of each request, in order
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
                played = outer.play(raw, self.headers.get("x-api-key"))
                if played is None:
                    return self._send(500, json.dumps({"type": "error", "error": {
                        "type": "api_error", "message": "scripted model: script exhausted"}}).encode())
                text, thinking = played
                data = b"".join(stream_frames(text, thinking=thinking))
                self._send(200, data, "text/event-stream; charset=utf-8")

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.port = self.httpd.server_address[1]
        self.url = f"http://127.0.0.1:{self.port}"
        self.host = f"127.0.0.1:{self.port}"
        self.endpoint = self.url + "/v1/messages"
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def render(self) -> None:
        filled = [fill(t, self.variables) for t in self.script]
        self.turns = [render_turn(t) for t in filled]
        self.thinking = [t.get("thinking") for t in filled]

    def bind(self, **variables) -> "ScriptedModel":
        """Fill the placeholders left open at load with values known only once the test runs.
        Call it before the team asks the model; a name still open after it is an error."""
        self.variables.update(variables)
        self.render()
        left = sorted({m for t in self.turns for m in re.findall(r"\{\{(\w+)\}\}", t)})
        if left:
            raise ValueError(f"scripted model: placeholders with no value: {left}")
        return self

    def play(self, raw: str, key: str | None = None) -> tuple[str, str | None] | None:
        """Serve one request body: record it, pick the turn of its lane, wait for the lanes the
        turn is `after`, and return (text, thinking) -- or None when the script is exhausted. The
        Messages handler and a fake of another wire (`helpers/fake_vllm.py`) both play through here."""
        with self.lock:
            self.raw_bodies.append(raw)
            try:
                self.requests.append(json.loads(raw))
            except ValueError:
                self.requests.append({})
            self.keys.append(key)
            lane = self.lane_of(raw)
            self.request_lanes.append(lane)
            i = self.next_index(lane)
            self.served += 1
            if i is None:
                self.overrun += 1
                i = len(self.turns)
            else:
                self.cursor[lane] = self.cursor.get(lane, 0) + 1
        if i >= len(self.turns):
            return None
        self.hold(self.after[i])
        text = self.resolve_ids(self.turns[i], raw)
        if self.delays[i]:
            time.sleep(self.delays[i])
        return text, self.thinking[i]

    def lane_of(self, raw: str) -> str:
        """The lane of a request: the `[lane:NAME]` marker in its first message (a system message
        is not one: an OpenAI-shaped body opens with it)."""
        if not self.laned:
            return "main"
        try:
            msgs = [m for m in json.loads(raw).get("messages", []) if m.get("role") != "system"]
            first = msgs[0]
        except (ValueError, IndexError, AttributeError, TypeError):
            return "main"
        m = re.search(r"\[lane:(\w+)\]", json.dumps(first))
        return m.group(1) if m else "main"

    @staticmethod
    def resolve_ids(text: str, raw: str) -> str:
        """Fill `{{id:N}}` from the Nth `<id>X</id>` in the request body, and `{{minted:N}}` from
        the Nth distinct minted id in an `<id>` (`<handler>_<6+ hex>`, a spawn's), in order."""
        if "{{id:" not in text and "{{minted:" not in text:
            return text
        found = re.findall(r"<id>(\w+)</id>", raw)
        minted = list(dict.fromkeys(re.findall(r"<id>(\w+_[0-9a-f]{6,})</id>", raw)))

        def pick(among):
            def go(m):
                n = int(m.group(1))
                return among[n - 1] if 0 < n <= len(among) else m.group(0)
            return go

        text = re.sub(r"\{\{id:(\d+)\}\}", pick(found), text)
        return re.sub(r"\{\{minted:(\d+)\}\}", pick(minted), text)

    def lane_turns(self, lane: str) -> list[int]:
        return [i for i, l in enumerate(self.lanes) if l == lane]

    def next_index(self, lane: str) -> int | None:
        """The index of the lane's next turn (the whole script in order when no turn has a
        lane); None past its end. Called under the lock, before the cursor moves."""
        if not self.laned:
            return self.served if self.served < len(self.turns) else None
        mine = self.lane_turns(lane)
        at = self.cursor.get(lane, 0)
        return mine[at] if at < len(mine) else None

    def lane_done(self, lane: str) -> bool:
        with self.lock:
            return self.cursor.get(lane, 0) >= len(self.lane_turns(lane))

    def hold(self, lanes: list[str], limit: float = 600.0) -> None:
        """Wait until every lane in `lanes` has been served all its turns."""
        end = time.time() + limit
        while time.time() < end and not all(self.lane_done(l) for l in lanes):
            time.sleep(0.1)

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
