"""The agents' live states, used (#351): the states that have a source today. An agent that asks the
person is marked in the tree, and stays counted on its branch when the branch is folded; an agent whose
turn runs is marked as working, its chat offers Stop, and Stop ends the turn. Function only.

Not here, on purpose (no source yet, see the README): stalled, paused, failed, done."""

import threading

from helpers import team_ui as D
from helpers.verdict import Verdict

NAME = "ui/agent_states_in_tree"


def run(ctx):
    v = Verdict(NAME)
    pw = D.playwright(v)
    if pw is None:
        return v.finish()
    port = D.start(ctx, v)
    if port is None:
        return v.finish()
    D.seed_inquiries(ctx, v)
    try:
        with D.Ui(pw, port) as ui:
            if ui.tree_ready(v):
                clicks(ctx, v, ui)
            ui.no_errors(v)
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    D.stop(ctx, v)
    return v.finish()


def generating(ctx) -> bool:
    return D.op(ctx.team, "lead", {"type": "status"}, timeout=20).get("generating") is True


def clicks(ctx, v, ui):
    side, right = ui.side, ui.right
    for id in D.NAMES:
        ui.reveal(id)

    # ---- asks ----
    marks = {id: ui.texts(ui.item(id).locator(".badge.ask")) for id in D.NAMES}
    v.check("exactly the two agents that asked are marked as asking",
            marks == {id: (["?"] if id in ("k_rollout", "m_ads") else []) for id in D.NAMES}, str(marks))
    v.check("an agent that asks nothing and runs nothing shows no state", ui.texts(ui.item("k_scale").locator(".sub")) == [],
            str(ui.texts(ui.item("k_scale").locator(".sub"))))
    ui.click(side.locator('#tree [aria-label="Fold cluster ops"]'))
    ok, got = ui.settle(lambda: ui.item("k_rollout").count(), 0)
    v.check("folding cluster ops hides k-rollout", ok, str(got))
    ok, got = ui.waits(ui.item("ops").locator(".asks"), "1 asks", exact=True)
    v.check("the folded branch says one agent inside asks", ok, got)
    v.check("folding Lead counts both askers below it",
            ui.click(side.locator('#tree [aria-label="Fold Lead"]')) and ui.waits(ui.item("lead").locator(".asks"), "2 asks", exact=True)[0],
            str(ui.texts(ui.item("lead").locator(".asks"))))
    ui.click(side.locator('#tree [aria-label="Unfold Lead"]'))
    ui.click(side.locator('#tree [aria-label="Unfold cluster ops"]'))
    ok, got = ui.settle(lambda: ui.item("k_rollout").count(), 1)
    v.check("unfolding brings k-rollout back", ok, str(got))

    # ---- working, and Stop ----
    threading.Thread(target=lambda: ctx.team.send("lead", {"type": "send", "text": "Plan the launch week."}, timeout=120),
                     daemon=True).start()
    ok, got = ui.settle(lambda: generating(ctx), True, timeout=20)
    v.check("Lead's turn runs (the model holds its answer back)", ok, str(got))
    ok, got = ui.waits(ui.item("lead").locator(".badge.active"), timeout=20)
    v.check("Lead's row is marked as working while its turn runs", ok, str(ui.texts(ui.item("lead").locator(".badge"))))
    state = ui.texts(ui.item("lead").locator(".sub"))
    v.check("the row says what Lead is doing (thinking, writing or using tools)",
            state and state[0] in ("thinking", "writing", "using tools"), str(state))
    v.check("no other agent is marked as working",
            not any(ui.item(id).locator(".badge.active").count() for id in D.NAMES if id != "lead"))
    v.check("a click on Lead's row opens it", ui.open("lead"), ui.right_now())
    ok, got = ui.waits(right.locator("#name"), "Lead", exact=True)
    v.check("the right cell is Lead's chat view", ok, f"{got!r}; {ui.right_now()}")
    ok, _ = ui.settle(lambda: right.locator("#stop").first.is_visible(), True)
    v.check("the chat offers Stop while the turn runs", ok)
    ui.click(right.locator("#stop"), tries=3)
    ok, got = ui.settle(lambda: generating(ctx), False, timeout=20)
    v.check("Stop ends Lead's turn", ok, str(got))
    ok, got = ui.settle(lambda: ui.item("lead").locator(".badge.active").count(), 0, timeout=20)
    v.check("Lead's row is no longer marked as working", ok, str(got))
