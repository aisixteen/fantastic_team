"""Sending, used (#351): a message typed in the composer of an agent opened from the tree reaches that
agent (the scripted model answers and the reply is drawn), and a question an agent asked is answered
from the page, by an option and in the person's own words. Function only."""

import secrets
import time

from helpers import team_ui as D
from helpers.verdict import Verdict

NAME = "ui/chat_send_and_answer"


def run(ctx):
    v = Verdict(NAME)
    pw = D.playwright(v)
    if pw is None:
        return v.finish()
    port = D.start(ctx, v)
    if port is None:
        return v.finish()
    D.seed_inquiries(ctx, v, via_ops=False)
    try:
        with D.Ui(pw, port) as ui:
            if ui.tree_ready(v):
                clicks(ctx, v, ui)
            ui.no_errors(v)
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    D.stop(ctx, v)
    return v.finish()


def bodies(ctx) -> str:
    return "".join(ctx.model.raw_bodies)


def clicks(ctx, v, ui):
    side, right, team = ui.side, ui.right, ctx.team

    # ---- a message to the lead ----
    token = "TOK_" + secrets.token_hex(6)
    v.check("a click on Lead's row opens it", ui.open("lead"), ui.right_now())
    ok, got = ui.waits(right.locator("#name"), "Lead", exact=True)
    v.check("the right cell is Lead's chat view", ok, f"{got!r}; {ui.right_now()}")
    try:
        right.locator("#text").fill(token, timeout=8000)
        right.locator("#send").click(timeout=4000)
        typed = True
    except Exception:
        typed = False
    v.check("the person types a message in the composer and sends it", typed, ui.right_now())
    ok, _ = ui.waits(right.locator(".who.user").filter(has_text=token), timeout=20)
    v.check("the person's message is drawn in the dialog", ok)
    ok, _ = ui.waits(right.locator(".who.assistant").filter(has_text="REPLY_FROM_THE_LEAD"), timeout=40)
    v.check("the agent's reply is drawn in the dialog", ok)
    ok, _ = ui.settle(lambda: token in bodies(ctx), True, timeout=20)
    v.check("the model was asked with the person's message", ok)
    hist = str(D.op(team, "lead", {"type": "history"}))
    v.check("Lead's history holds the message and the reply", token in hist and "REPLY_FROM_THE_LEAD" in hist, hist[:200])
    other = str(D.op(team, "ops", {"type": "history"}))
    v.check("no other agent got it", token not in other, other[:200])

    # ---- an option answers k-rollout's question ----
    ui.click(side.locator("#wait .w").filter(has_text="k-rollout"))
    ok, got = ui.waits(right.locator(".inq .q"), D.QUESTION, exact=True)
    v.check("k-rollout, opened from 'Waiting for you', shows its question", ok, f"{got!r}; {ui.right_now()}")
    v.check("the person picks the option 'Roll back to v2.3'",
            ui.click(right.locator(".inq button").filter(has_text=D.OPTIONS[1]), tries=3), ui.right_now())
    ok, got = ui.settle(lambda: [q.get("asker") for q in D.inquiries(team)], ["m_ads"], timeout=20)
    v.check("the question is answered: user no longer holds it", ok, str(got))
    ok, _ = ui.settle(lambda: D.OPTIONS[1] in bodies(ctx), True, timeout=30)
    v.check("k-rollout got the answer (its next model request holds it)", ok)
    ok, _ = ui.waits(right.locator(".who.assistant").filter(has_text="Rolling back to v2.3."), timeout=40)
    v.check("k-rollout's reply to the answer is drawn", ok)
    ok, got = ui.settle(lambda: ui.texts(side.locator("#wait .w .who b")), ["m-ads"], timeout=20)
    v.check("'Waiting for you' now lists m-ads alone", ok, str(got))

    # ---- the person's own words answer m-ads's question ----
    ui.click(side.locator("#wait .w").filter(has_text="m-ads"))
    ok, got = ui.waits(right.locator(".inq .q"), D.BUDGET, exact=True)
    v.check("m-ads, opened from 'Waiting for you', shows its question", ok, f"{got!r}; {ui.right_now()}")
    words = "Pause it until Monday " + secrets.token_hex(3)
    try:
        box = right.locator(".inq input[type=text]")
        if box.count():
            box.first.fill(words, timeout=4000)
            box.first.press("Enter", timeout=4000)
        else:  # the question is answered from the composer
            right.locator("#text").fill(words, timeout=4000)
            right.locator("#send").click(timeout=4000)
        time.sleep(0.5)
        sent = True
    except Exception:
        sent = False
    v.check("the person answers in their own words", sent, ui.right_now())
    ok, got = ui.settle(lambda: D.inquiries(team), [], timeout=20)
    v.check("the question is answered: user holds no open question", ok, str(got)[:200])
    ok, _ = ui.settle(lambda: words in bodies(ctx), True, timeout=30)
    v.check("m-ads got the person's words", ok)
    ok, got = ui.settle(lambda: side.locator("#wait").first.is_hidden(), True, timeout=20)
    v.check("'Waiting for you' is gone once nothing waits", ok, str(got))
