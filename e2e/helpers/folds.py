"""The three atomic folds of fantastic's autofold (fantastic #556, #511) in the coding scenarios (team #323).

THE RULE: a body is WHOLE in the request that carries it to the model, and an ANCHOR from the next
request on (never folded before the model has answered it). Three kinds of body fold:
  - a large reply (a body over AUTOFOLD_AT = 16000 tokens, ~64,000 characters):
    `[folded · <verb> reply from <agent> · ~N tokens · head: "…" · get: ID]`;
  - a thought (the `thinking` of an assistant row, which never goes back to the model whole):
    `[thought ~N tokens · id ID]`;
  - a recall reply (every one, whatever its size):
    `[folded · recall reply from <agent> · id ID · ~N tokens · get: ID]`.
The store keeps every row whole; `recall {id}` reads one back (its content, and its thinking when it had any).

The scripted twins (`cli/single_lead`, `cli_ssh/single_lead`) play it with the lead's own turns: `turns_one` makes a
thought and a large reply (an exec `run` that prints ~70k characters), `turns_two` recalls both by id. The ids are the
rows the lead's own history assigned, so the probe reads them between the two messages (`bind_rows`) and the plug
binds them to `{{row_big}}` / `{{row_thought}}` of the recall turns (`ScriptedModel.bind`). `check`
reads the request bodies the plug recorded. The gpu twins (a real model) assert nothing of this: `report_line` counts,
from the lead's history, what the model folded and recalled.

THE WINDOW. A reply over the inline cap of the lead ((window - 1024) / 8 tokens) is held in memory and answered as a
stub, not whole. The chat's own window is 131072 (a cap of 16256 tokens, below what a 16000-token fold needs with
the reply's markup), so `widen` sets the lead's `context_window` (an existing field) to the model's 200000 before the
first message.
"""

from __future__ import annotations

import json
import re

from . import cli

BIG_LINES = 400  # lines the large reply prints
BIG_PAD = 104  # characters after the 8-character `BIGnnnn ` head of each line
BIG_FIRST = "BIG0000 "
BIG_LAST = f"BIG{BIG_LINES - 1:04d} "
THOUGHT_MARK = "THOUGHT-MARK-7f3a91"
THOUGHT = (f"{THOUGHT_MARK}: before the large print I weigh what I need from it. The output is long, so I will read "
           "it once and keep only what the task needs; if I want a line of it again I can ask for the row by its id. "
           "Nothing in this reasoning goes back to the model whole after this turn, only an anchor that names it.")
WINDOW = 200000
FOLD_CHARS = 64000  # a body over this many characters (16000 tokens of 4) folds once answered
MORE = "Now read back the large reply and the thought you folded, then finish."
ANCHOR = re.compile(r"\[folded · [^\]\n]*\]|\[thought ~\d+ tokens · id \d+\]")


# ---- the script --------------------------------------------------------------------------

def big_call() -> dict:
    """The worker's exec `run` that prints BIG_LINES lines of ~112 characters: its reply is the job's last
    lines (`tail` up to 500), ~70,000 characters with the markup, over the fold and under the inline cap."""
    code = f"for i in range({BIG_LINES}): print('BIG%04d ' % i + 'x' * {BIG_PAD})"
    return {"target": cli.EXEC, "type": "run",
            "args": {"command": "python3", "args": ["-c", code], "wait_s": 30, "tail": BIG_LINES}}


ROW_NAMES = ("row_big", "row_thought")  # the placeholders `bind_rows` binds


def recall_call(name: str) -> dict:
    """The lead's `recall {id}` on itself, the id a placeholder the probe binds once it has read the row."""
    return {"target": cli.LEAD, "type": "recall", "args": {"id": "{{" + name + "}}"}}


def turns_one() -> list[dict]:
    """Message 1's end: a turn with a thought and the large call, a turn that answers it, `done`."""
    return [{"thinking": THOUGHT, "calls": [big_call()]},
            {"calls": [cli.run_call("git", "log", "-1", "--format=%s")]},
            {"calls": [{"name": "done"}]}]


def turns_two() -> list[dict]:
    """Message 2's turns: recall the large reply, recall the thought, a normal turn (the caller ends the run)."""
    return [{"calls": [recall_call("row_big")]},
            {"calls": [recall_call("row_thought")]},
            {"calls": [cli.run_call("git", "status", "--porcelain")]}]


def at_of(script: list[dict]) -> int:
    """The index (among the lead's turns) of the turn that thinks and prints: its request is `at`, the
    request that carries the large reply whole is `at + 1`."""
    return next(i for i, t in enumerate(script) if "thinking" in t)


# ---- the run -----------------------------------------------------------------------------

def widen(ctx, v, lead: str = cli.LEAD) -> None:
    """The lead's window, as the model's (see THE WINDOW)."""
    r = ctx.team.send("kernel", {"type": "update_agent", "id": lead, "context_window": WINDOW})
    v.check("the lead's context window is the model's, so a 70k-character reply is inlined whole",
            r.get("status") == "completed", str(r)[:300])


def rows_of(rows: list[dict]) -> dict:
    """The ids the second message recalls, from the lead's `history` digests: the user row whose body is over
    the fold, and the assistant row that kept a thought."""
    big = [r for r in rows if r.get("role") == "user" and (r.get("bytes") or 0) > FOLD_CHARS]
    thought = [r for r in rows if r.get("role") == "assistant" and r.get("has_thinking")]
    return {"big": [r["id"] for r in big], "thought": [r["id"] for r in thought]}


def bind_rows(ctx, v, lead: str = cli.LEAD, label: str = "") -> dict:
    """Between the two messages: find the large reply's row and the thought's row in the lead's history, and
    hand them to the plug for the recall turns."""
    found = rows_of(cli.history_rows(ctx.team, lead))
    v.check(f"{label}the store holds the large reply as one row over {FOLD_CHARS} characters and the thought as one row",
            len(found["big"]) == 1 and len(found["thought"]) == 1, str(found))
    if len(found["big"]) == 1 and len(found["thought"]) == 1:
        ctx.model.bind(row_big=found["big"][0], row_thought=found["thought"][0])
    return found


def main_requests(ctx) -> list[str]:
    """The text of each request of the lead's lane (every message, in order, joined): what the model was shown."""
    out = []
    for body, lane in zip(ctx.model.requests, ctx.model.request_lanes):
        if lane != "main":
            continue
        parts = []
        for m in (body or {}).get("messages", []):
            c = m.get("content")
            parts.append(c if isinstance(c, str) else "\n".join(b.get("text", "") for b in c or [] if isinstance(b, dict)))
        out.append("\n".join(parts))
    return out


def anchors(text: str) -> list[str]:
    return ANCHOR.findall(text)


def kind_of(anchor: str) -> str:
    """`large` (a non-recall reply), `thought` or `recall`."""
    if anchor.startswith("[thought"):
        return "thought"
    return "recall" if anchor.startswith("[folded · recall reply") else "large"


def check(ctx, v, label: str, at: int, found: dict, lead: str = cli.LEAD) -> None:
    """The three folds, in the requests of the lead (request `at` is the turn that thinks and prints):
    at+1 the large reply whole; at+2 .. at+3 an anchor; at+4 the recalled reply whole (once: the first is still an
    anchor), at+5 its recall anchor and the recalled thought whole, at+6 both recall anchors."""
    reqs = main_requests(ctx)
    n = at + 7
    if not v.check(f"{label}folds: the lead's lane made the {n} requests the fold turns need (it made {len(reqs)})",
                   len(reqs) >= n, f"{len(reqs)} requests"):
        return
    big_id, thought_id = (found["big"] or [0])[0], (found["thought"] or [0])[0]
    count = lambda k, mark: reqs[k].count(mark)
    for k in range(0, at + 1):
        v.check(f"{label}folds: request {k} holds neither the large reply nor the thought", not count(k, BIG_LAST)
                and not count(k, THOUGHT_MARK))
    v.check(f"{label}folds: the large reply is WHOLE in the request right after its call (first and last line, no anchor of it)",
            count(at + 1, BIG_FIRST) == 1 and count(at + 1, BIG_LAST) == 1 and not any(kind_of(a) == "large" for a in anchors(reqs[at + 1])),
            f"first {count(at + 1, BIG_FIRST)} last {count(at + 1, BIG_LAST)} anchors {anchors(reqs[at + 1])}")
    for k in (at + 2, at + 3):
        big = [a for a in anchors(reqs[k]) if kind_of(a) == "large"]
        v.check(f"{label}folds: request {k}: the large reply is only its anchor (`get: {big_id}`), no line of it",
                len(big) == 1 and f"get: {big_id}]" in big[0] and not count(k, BIG_FIRST) and not count(k, BIG_LAST), str(big))
    v.check(f"{label}folds: the thought is never whole in a request before its recall (only its anchor, from the next request on)",
            all(not count(k, THOUGHT_MARK) for k in range(0, at + 5)), "the thought text reached the model")
    for k in range(at + 1, len(reqs)):
        th = [a for a in anchors(reqs[k]) if kind_of(a) == "thought"]
        if not (len(th) >= 1 and f"id {thought_id}]" in th[0]):
            v.check(f"{label}folds: request {k} names the thought by its anchor (id {thought_id})", False, str(th))
            break
    else:
        v.check(f"{label}folds: every request from {at + 1} on names the thought by its anchor (id {thought_id})", True)
    v.check(f"{label}folds: the recall reply of the large row carries the whole body (once: the first is still an anchor)",
            count(at + 4, BIG_FIRST) == 1 and count(at + 4, BIG_LAST) == 1, f"first {count(at + 4, BIG_FIRST)} last {count(at + 4, BIG_LAST)}")
    rec5 = [a for a in anchors(reqs[at + 5]) if kind_of(a) == "recall"]
    v.check(f"{label}folds: one request later that recall reply is an anchor again (id {big_id}), and the body is gone",
            len(rec5) == 1 and f"id {big_id} " in rec5[0] and not count(at + 5, BIG_LAST), str(rec5))
    v.check(f"{label}folds: the recall reply of the thought carries the whole thought",
            count(at + 5, THOUGHT_MARK) == 1, f"{count(at + 5, THOUGHT_MARK)}")
    rec6 = [a for a in anchors(reqs[at + 6]) if kind_of(a) == "recall"]
    v.check(f"{label}folds: one request later the thought's recall reply is an anchor too (ids {big_id}, {thought_id}), the thought gone",
            len(rec6) == 2 and f"id {big_id} " in rec6[0] and f"id {thought_id} " in rec6[1] and not count(at + 6, THOUGHT_MARK), str(rec6))
    seen: dict[str, int] = {}
    for k, text in enumerate(reqs):
        for a in anchors(text):
            seen.setdefault(a, k)
    broken = [(a[:60], k) for a, first in seen.items() for k in range(first, len(reqs)) if a not in anchors(reqs[k])]
    v.check(f"{label}folds: every anchor is byte-identical in every request after its first ({len(seen)} anchors)",
            bool(seen) and not broken, str(broken[:3]))
    whole = ctx.team.send(lead, {"type": "recall", "id": big_id}, timeout=60).get("result") or {}
    text = json.dumps(whole)
    v.check(f"{label}folds: the store still holds the large reply whole (recall {big_id}: first and last line)",
            BIG_FIRST in text and BIG_LAST in text, text[:200])
    kept = ctx.team.send(lead, {"type": "recall", "id": thought_id}, timeout=60).get("result") or {}
    v.check(f"{label}folds: the store still holds the thought whole (recall {thought_id}: its thinking)",
            THOUGHT_MARK in json.dumps(kept), json.dumps(kept)[:200])


# ---- the real-model report (gpu_cli, gpu_cli_ssh): counts, no pass or fail ---------------------

def counts(rows: list[dict], recall_calls: int = 0) -> dict:
    """What the lead's history says it folded: `large` (a reply body over the fold), `thoughts` (rows that kept
    a thought, each folded to an anchor), `recall_replies` (replies to a `recall`, each folded one request after),
    `refolds` (recall replies the model has answered, so already an anchor again), `recalls` (the calls)."""
    ordered = sorted((r for r in rows if r.get("id") is not None), key=lambda r: r["id"])
    large = recalled = refolds = thoughts = 0
    for i, r in enumerate(ordered):
        if r.get("has_thinking"):
            thoughts += 1
        if r.get("role") != "user" or not str(r.get("head", "")).startswith("[reply to"):
            continue
        if re.match(r"\[reply to \S+ · \S+ recall\b", str(r["head"])):
            recalled += 1
            refolds += any(x.get("role") == "assistant" for x in ordered[i + 1:])
        elif (r.get("bytes") or 0) > FOLD_CHARS:
            large += 1
    return {"large_replies": large, "thoughts": thoughts, "recalls": recall_calls,
            "recall_replies": recalled, "refolds": refolds}


def report_line(rows: list[dict], recall_calls: int = 0) -> str:
    c = counts(rows, recall_calls)
    return ("folds: " + ", ".join(f"{k.replace('_', ' ')} {n}" for k, n in c.items())
            + " (counted from the lead's history; the model decides, nothing is asserted)")
