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

GRANTS BEFORE A TURN. A turn may carry `"grant_leg": ["w2", "{{minted:2}}"]`: before the plug answers it, the
test's operator opens those agents to the bridge leg (`leg_grant`, set by the test: the ssh twins' `Box.bind`),
the ids resolved from the request like any `{{minted:N}}`. An agent that is minted when the lead spawns it has no
name before, so the operator names it when the script reaches the turn that needs it.

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

TEAM NOTICES. The cadencer tells a lead's team its structure whenever an AI agent is added or
removed (`[team] The team of …`), and each notice is a turn of its own. A request whose newest
message is notices only is answered `<call name="await"></call>` OUTSIDE the script (it keeps
whatever the agent waits on: a `done` there would end a chain the notice landed in): it takes no
turn of any lane and is counted in `notice_turns`, so a script stays the test's own steps. So is
the kernel's request to rebuild a woken agent's working context (`rebuild_turns`).

LATE WAKES. A worker's report is a plain `send` whose reply (the parent's answer) can reach the worker after its
last turn and before the parent deletes it; the request that wakes it (the newest message is `[reply to N · <agent> send …]`)
is answered `await` outside the script, counted in `late_turns`. A worker's lane is also known by its id (the system
prompt's "You are `id`"), so such a request with a fresh conversation, which holds no marker, is not taken for the lead's.

CHECK-INS. A lead's own scheduler wakes it with a `send` whose text starts `[check-in <name>]`. A tick
is a notice too (answered `await` outside the script when it is all the newest message holds); every
request whose conversation holds one is counted in `check_in_turns`, so a test sees the tick reached it.
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


# fantastic's routing names (tool_parse::grammar::ROUTING besides target_id): in a script's
# `args` they are the call's routing, as they were inline, so they go in `<routing>`.
ROUTING = ("to", "re", "reply_to", "title", "timeout", "retries", "exec_id")


def render_call(c: dict) -> str:
    """One call element. A `send` is the routing form (fantastic#527): `<routing>` holds the
    target and any routing name, `<payload>` the verb and its arguments, delivered whole. Any
    other call (`done`, `await`, …) is its name with its arguments as child tags."""
    name = c.get("name", "send")
    args = dict(c.get("args") or {})
    if name != "send":
        body = [f"<{k}>{render_value(v)}</{k}>" for k, v in args.items()]
        return "\n".join([f'<call name="{name}">', *body, "</call>"])
    routing = [f"<target_id>{c['target']}</target_id>"] if c.get("target") else []
    routing += [f"<{k}>{render_value(args.pop(k))}</{k}>" for k in ROUTING if k in args]
    payload = [f"<type>{c['type']}</type>"] if c.get("type") else []
    payload += [f"<{k}>{render_value(v)}</{k}>" for k, v in args.items()]
    return "\n".join([f'<call name="{name}">', f"<routing>{''.join(routing)}</routing>",
                      f"<payload>{''.join(payload)}</payload>", "</call>"])


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
        self.notice_turns = 0  # team notices answered outside the script
        self.notice_bodies: list[str] = []  # the bodies of those notices, in order
        self.rebuild_turns = 0  # the kernel's context rebuilds, answered outside the script
        self.check_in_turns = 0  # requests whose conversation holds a scheduler's check-in
        self.check_in_lanes: set[str] = set()  # the lanes those requests came from
        self.overrun = 0
        self.late_turns = 0  # a finished lane's agent woken by the reply to its last send, answered outside the script
        self.agent_lanes: dict[str, str] = {}  # agent id (the system prompt's "You are `id`") -> its lane
        self.leg_grant = self.no_leg_grant  # the test sets it when its script has `grant_leg` turns
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

    @staticmethod
    def no_leg_grant(ids: list[str]) -> None:
        raise RuntimeError(f"scripted model: a turn grants the leg {ids} and the test set no leg_grant")

    def render(self) -> None:
        filled = [fill(t, self.variables) for t in self.script]
        self.turns = [render_turn(t) for t in filled]
        self.thinking = [t.get("thinking") for t in filled]
        self.grants = [list(t.get("grant_leg") or []) for t in filled]

    def bind(self, keep: tuple = (), **variables) -> "ScriptedModel":
        """Fill the placeholders left open at load with values known only once the test runs. It may be called
        again mid-run (a value that a turn already played made, such as a row id of the lead's own history): the
        turns not served yet are rendered again. A name still open after it is an error, but for the names in
        `keep`, which a later call fills."""
        self.variables.update(variables)
        self.render()
        left = sorted({m for t in self.turns for m in re.findall(r"\{\{(\w+)\}\}", t)} - set(keep))
        if left:
            raise ValueError(f"scripted model: placeholders with no value: {left}")
        return self

    # a team notice, fantastic's await note, and its memory recall row (the kernel's, after a restart)
    NOTICE_HEADS = ("[from cadencer]", "[team] The team of", "Members:", "- `", "[await: nothing to wait for]",
                    "[recall ")

    @staticmethod
    def is_check_in(block: str) -> bool:
        """A tick of a lead's own scheduler: `[from <scheduler> …] [check-in <name>] …`."""
        return block.startswith("[check-in") or (block.startswith("[from ") and "[check-in" in block[:300])

    @classmethod
    def is_notice(cls, block: str) -> bool:
        return block.startswith(cls.NOTICE_HEADS) or cls.is_check_in(block)

    @classmethod
    def own_blocks(cls, msg: dict) -> list[str]:
        """The blocks of a message (joined by a blank line) that are not team notices, check-ins or
        fantastic's await note. The wire merges consecutive user messages, so a notice and a task can
        share one."""
        return [b for b in cls.blocks(msg) if not cls.is_notice(b)]

    @classmethod
    def check_in(cls, raw: str) -> bool:
        """Does this request's conversation hold a scheduler's check-in (a tick that arrived while
        the agent worked sits in an earlier message than the newest)?"""
        try:
            msgs = json.loads(raw).get("messages", [])
        except (ValueError, AttributeError, TypeError):
            return False
        return any(m.get("role") == "user" and any(cls.is_check_in(b) for b in cls.blocks(m))
                   for m in msgs if isinstance(m, dict))

    @staticmethod
    def blocks(msg: dict) -> list[str]:
        c = msg.get("content")
        text = c if isinstance(c, str) else "\n\n".join(
            b.get("text", "") for b in (c or []) if isinstance(b, dict))
        return [b.strip() for b in text.split("\n\n") if b.strip()]

    @classmethod
    def notice_only(cls, raw: str) -> bool:
        """Is the newest message of this request team notices (and await notes) and nothing else?"""
        try:
            msgs = [m for m in json.loads(raw).get("messages", []) if m.get("role") != "system"]
            last = msgs[-1]
        except (ValueError, IndexError, AttributeError, TypeError):
            return False
        if last.get("role") != "user" or not last.get("content"):
            return False
        own = cls.own_blocks(last)
        if not own:
            return True
        # the kernel's "[N message(s) arrived while you were working …]": a notice when the N
        # messages that arrived (the latest `[from …]` / `[team]` blocks above it) are all notices
        arrived = re.match(r"\[(\d+) message\(s\) arrived while you were working", own[-1])
        if not arrived or len(own) > 1:
            return False
        blocks = []
        for m in msgs[:-1]:
            if m.get("role") == "user":
                blocks += cls.blocks(m)
        came = [b for b in blocks if b.startswith(("[from ", "[team] The team of"))][-int(arrived.group(1)):]
        return bool(came) and all(cls.is_notice(b) for b in came)

    @staticmethod
    def rebuild(raw: str) -> bool:
        """Is this the kernel asking the model to rebuild an agent's working context (when a notice
        wakes it after a restart)? Not the agent's own turn: answered with a word the kernel reads as
        no plan, so it keeps its mechanical fit."""
        try:
            msgs = [m for m in json.loads(raw).get("messages", []) if m.get("role") != "system"]
            c = msgs[0].get("content")
        except (ValueError, IndexError, AttributeError, TypeError):
            return False
        text = c if isinstance(c, str) else "".join(b.get("text", "") for b in (c or []) if isinstance(b, dict))
        return len(msgs) == 1 and text.startswith("You are rebuilding your own working context.")

    def play(self, raw: str, key: str | None = None) -> tuple[str, str | None] | None:
        """Serve one request body: record it, pick the turn of its lane, wait for the lanes the
        turn is `after`, and return (text, thinking) -- or None when the script is exhausted. The
        Messages handler and a fake of another wire (`helpers/fake_vllm.py`) both play through here."""
        if self.check_in(raw):
            with self.lock:
                self.check_in_turns += 1
                self.check_in_lanes.add(self.lane_of(raw))
        if self.notice_only(raw):
            with self.lock:
                self.notice_turns += 1
                self.notice_bodies.append(raw)
            return '<call name="await"></call>', None
        if self.rebuild(raw):
            with self.lock:
                self.rebuild_turns += 1
            return "The head is fine as it is.", None
        with self.lock:
            lane = self.lane_of(raw)
            if (lane != "main" and self.cursor.get(lane, 0) >= len(self.lane_turns(lane))
                    and self.parent_answer(raw)):
                # a worker's report is a `send` whose reply (the parent's answer) can wake it once its script is
                # played, before the parent deletes it: that is no turn of the script, and it is not past its end
                self.late_turns += 1
                return '<call name="await"></call>', None
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
        if self.grants[i]:
            self.leg_grant([self.resolve_ids(g, raw) for g in self.grants[i]])
        if self.delays[i]:
            time.sleep(self.delays[i])
        return text, self.thinking[i]

    @classmethod
    def parent_answer(cls, raw: str) -> bool:
        """Is the newest message of this request the reply to a `send` (`[reply to N · <agent> send …]`)?"""
        try:
            msgs = [m for m in json.loads(raw).get("messages", []) if m.get("role") != "system"]
            first = (cls.blocks(msgs[-1]) or [""])[0]
        except (ValueError, IndexError, AttributeError, TypeError):
            return False
        return msgs[-1].get("role") == "user" and re.match(r"\[reply to \d+ · \w+ send ", first) is not None

    def lane_of(self, raw: str) -> str:
        """The lane of a request: the `[lane:NAME]` marker in its first message (a system message
        is not one: an OpenAI-shaped body opens with it)."""
        if not self.laned:
            return "main"
        try:
            body = json.loads(raw)
            msgs = [m for m in body.get("messages", []) if m.get("role") != "system"]
        except (ValueError, AttributeError, TypeError):
            return "main"
        system = body.get("system") if isinstance(body, dict) else None
        who = re.search(r"You are `(\w+)`", system) if isinstance(system, str) else None
        # notices and await notes may come before the agent's task, merged into its message: the
        # lane is in the first user message that holds anything else
        m = None
        for msg in (x for x in msgs if x.get("role") == "user"):
            own = self.own_blocks(msg)
            if own:
                m = re.search(r"\[lane:(\w+)\]", "\n\n".join(own))
                break
        if m and who:
            self.agent_lanes.setdefault(who.group(1), m.group(1))
        return m.group(1) if m else (self.agent_lanes.get(who.group(1), "main") if who else "main")

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
