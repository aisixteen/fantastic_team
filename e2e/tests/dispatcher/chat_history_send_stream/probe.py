"""A live team's chat reaches a web client: history, send, and the streamed parts."""

from helpers import run_flow, web
from helpers.verdict import Verdict

SAID = "hello from the scripted model"
CLIENT = "e2e-page"


def run(ctx):
    v = Verdict("dispatcher/chat_history_send_stream")
    port = run_flow.boot_page(ctx)
    ws = web.WsClient(port, "/canvas_ws/ws")
    ws.watch("lead")

    h0 = ws.call("lead", {"type": "history"})
    v.check("history is empty before the first turn", h0.get("rows") == [] and h0.get("total") == 0, str(h0))
    s0 = ws.call("lead", {"type": "status"})
    v.check("status is idle before the turn", s0.get("generating") is False and s0.get("current") is None, str(s0))

    sent = ws.call("lead", {"type": "send", "text": "hi", "client_id": CLIENT}, timeout=60)
    v.check("send is accepted and answers with the scripted text",
            SAID in str(sent.get("final", "")) and not sent.get("error"), str(sent))
    v.check("the model was asked once", ctx.model.served == 1, f"served {ctx.model.served}")

    ws.wait_event(lambda f: f["payload"].get("type") == "status"
                  and f["payload"].get("phase") == "done" and f["payload"].get("client_id") == CLIENT, 20)
    mine = [f["payload"] for f in ws.events("lead")
            if f["payload"].get("client_id") == CLIENT]
    kinds = [p.get("type") for p in mine]
    text_start = next((p for p in mine if p.get("type") == "part_start" and p.get("kind") == "text"), None)
    sid = text_start and text_start.get("stream_id")
    deltas = [p for p in mine if p.get("type") == "part_delta" and p.get("stream_id") == sid]
    ends = [p for p in mine if p.get("type") == "part_end" and p.get("stream_id") == sid]
    v.check("the stream opens a text part for this client", text_start is not None, str(kinds))
    v.check("the part has deltas, and they carry the answer exactly once",
            len(deltas) >= 1 and "".join(d.get("text", "") for d in deltas).count(SAID) == 1,
            str([d.get("text") for d in deltas]))
    v.check("the part ends", len(ends) == 1, str(kinds))
    order = [i for i, p in enumerate(mine) if p.get("stream_id") == sid]
    v.check("start, delta, end come in that order",
            bool(order) and [mine[i]["type"] for i in order][0] == "part_start"
            and [mine[i]["type"] for i in order][-1] == "part_end", str(kinds))
    v.check("the turn ends with a done status",
            any(p.get("type") == "status" and p.get("phase") == "done" for p in mine), str(kinds))

    h1 = ws.call("lead", {"type": "history"})
    heads = [(r.get("role"), r.get("head", "")) for r in h1.get("rows", [])]
    v.check("history holds the user text", any(r == "user" and "hi" in h for r, h in heads), str(heads))
    v.check("history holds the assistant text", any(r == "assistant" and SAID in h for r, h in heads), str(heads))
    s1 = ws.call("lead", {"type": "status"})
    v.check("status is idle after the turn", s1.get("generating") is False and s1.get("current") is None, str(s1))

    # The page reaches only what it is granted: the chat verbs of the lead, nothing else.
    out = ws.call("worker_exec", {"type": "run", "command": "true"})
    v.check("a tool outside the grant is refused", out.get("reason") == "unauthorized" or out.get("refused"), str(out))
    out = ws.call("lead", {"type": "reflect"})
    v.check("a verb of the lead outside the chat verbs is refused",
            out.get("reason") == "unauthorized" or out.get("refused"), str(out))
    ws.close()
    return v.finish()
