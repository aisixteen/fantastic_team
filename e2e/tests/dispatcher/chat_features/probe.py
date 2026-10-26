"""One live turn with reasoning and a tool call, then a second turn with an attachment."""

import time

from helpers import run_flow, web
from helpers.verdict import Verdict

CLIENT = "e2e-page"
THOUGHT = "let me check the queue"
MARKER = "ATTACH-MARKER-5821"


def own(ws, src="lead"):
    return [f["payload"] for f in ws.events(src) if f["payload"].get("client_id") == CLIENT]


def parts(payloads, kind):
    """(start, joined delta text, end) for each part of this kind."""
    out = []
    for p in payloads:
        if p.get("type") == "part_start" and p.get("kind") == kind:
            sid = p["stream_id"]
            text = "".join(d.get("text", "") for d in payloads
                           if d.get("type") == "part_delta" and d.get("stream_id") == sid)
            ended = any(e.get("type") == "part_end" and e.get("stream_id") == sid for e in payloads)
            out.append((p, text, ended))
    return out


def run(ctx):
    v = Verdict("dispatcher/chat_features")
    port = run_flow.boot_page(ctx, watch_only=["queue"])
    ws = web.WsClient(port, "/canvas_ws/ws")
    ws2 = web.WsClient(port, "/canvas_ws/ws")  # a second page: asks `status` while the first waits
    ws.watch("lead")
    ws.watch("queue")

    cid = ws.send_call("lead", {"type": "send", "text": "check the queue", "client_id": CLIENT})
    running = None
    end = time.time() + 30
    while time.time() < end and not running:
        if ctx.model.served >= 2:  # the second reply is held for 3 s: the turn is running
            s = ws2.call("lead", {"type": "status", "client_id": CLIENT})
            if s.get("generating") and (s.get("current") or {}).get("is_mine"):
                running = s
        time.sleep(0.2)
    v.check("status reports the turn while it runs (generating, the page's own, its phase)",
            running is not None and (running["current"].get("phase") in ("thinking", "streaming")),
            str(running))
    sent = ws.wait_reply(cid, 60)
    v.check("send answers", not sent.get("error"), str(sent))

    def dones():
        return [f for f in ws.seen if f.get("type") == "event" and f["payload"].get("phase") == "done"
                and f["payload"].get("client_id") == CLIENT]
    ended = ws.wait_event(lambda f: len(dones()) >= 2, 40)
    v.check("the chain ends: two model turns, two done statuses", ended and ctx.model.served == 2,
            f"served {ctx.model.served}, dones {len(dones())}")
    time.sleep(0.5)
    idle = ws2.call("lead", {"type": "status"})
    v.check("status is idle once the turn is over", idle.get("generating") is False, str(idle))

    mine = own(ws)
    think = parts(mine, "thinking")
    v.check("a reasoning part streams, with the thought, and ends",
            len(think) == 1 and think[0][1] == THOUGHT and think[0][2], str(think))
    texts = [t for _, t, _ in parts(mine, "text")]
    v.check("the first text part is the words before the call", texts[:1] == ["checking\n"], str(texts))
    v.check("the final text part is `done`", texts[-1:] == ["done\n"], str(texts))
    qev = [f["payload"] for f in ws.events("queue")]
    starts = [p for p in qev if p.get("type") == "part_start" and p.get("kind") == "call"]
    tool = starts + [p for p in qev if p.get("stream_id") in {s.get("stream_id") for s in starts}]
    tdelta = "".join(p.get("text", "") for p in tool if p.get("type") == "part_delta")
    v.check("the tool call is a call part that names its target", len(starts) == 1
            and starts[0].get("target") == "queue" and starts[0].get("name") == "send", str(tool))
    v.check("the tool part carries the verb", "<type>read</type>" in tdelta, tdelta)
    v.check("the queue got the verb", any(f["payload"].get("type") == "read" for f in ws.events("queue")))

    done = [p for p in mine if p.get("type") == "status" and p.get("phase") == "done"]
    detail = (done[-1].get("detail") if done else None) or {}
    v.check("the done status carries the context numbers",
            detail.get("context_window") == 100000 and isinstance(detail.get("prompt_tokens"), int), str(detail))

    hist = ws2.call("lead", {"type": "history"})
    rows = hist.get("rows", [])
    v.check("history replays the reasoning (the assistant row has thinking)",
            any(r.get("role") == "assistant" and r.get("has_thinking") for r in rows), str(rows))
    v.check("history replays the tool call and its reply",
            any("queue" in r.get("head", "") and "<call" in r.get("head", "") for r in rows)
            and any(r.get("head", "").startswith("[reply to") and "queue read" in r.get("head", "") for r in rows),
            str([r.get("head") for r in rows]))

    inq = ws2.call("user", {"type": "inquiries", "asker": "lead"})
    v.check("the user agent answers `inquiries` for the lead (none open)",
            isinstance(inq.get("inquiries"), list) and inq["inquiries"] == [], str(inq))

    text = f"read this\n\n[attached: notes.txt]\n{MARKER}"
    r3 = ws.call("lead", {"type": "send", "text": text, "client_id": CLIENT}, timeout=60)
    v.check("a send with an attachment (text inside the send) is accepted", not r3.get("error"), str(r3))
    body = ctx.model.raw_bodies[2] if len(ctx.model.raw_bodies) > 2 else ""
    v.check("the model got the attached text", MARKER in body and "[attached: notes.txt]" in body,
            f"{len(ctx.model.raw_bodies)} requests; {body[-300:]}")
    ws.close()
    ws2.close()
    return v.finish()
