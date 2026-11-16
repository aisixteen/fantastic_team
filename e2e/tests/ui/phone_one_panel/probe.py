"""The page on a phone, 390x844 (#351): one panel at a time; the tree, a tap on an agent shows that agent's
view, back returns to the tree, and the back button counts the OTHER agents that wait for the person.
Function only."""

from helpers import team_ui as D
from helpers.verdict import Verdict

NAME = "ui/phone_one_panel"
CARDS, SIZE = True, (390, 844)


def seed(ctx, v):
    D.seed_tasks(ctx, v)
    D.seed_inquiries(ctx, v)


def run(ctx):
    v = Verdict(NAME)
    pw = D.playwright(v)
    if pw is None:
        return v.finish()
    port = D.start(ctx, v, cards=CARDS)
    if port is None:
        return v.finish()
    seed(ctx, v)
    try:
        with D.Ui(pw, port, *SIZE) as ui:
            if ui.tree_ready(v):
                clicks(ctx, v, ui)
            ui.no_errors(v)
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    D.stop(ctx, v)
    return v.finish()


def panels(ui) -> dict:
    return ui.page.evaluate("""() => { const vis = id => { const r = document.getElementById(id).getBoundingClientRect(); return r.width > 0 && r.height > 0; };
      return {panel: document.getElementById('layout').getAttribute('data-panel'), left: vis('col-left'), right: vis('col-right'),
              sideways: document.documentElement.scrollWidth > window.innerWidth}; }""")


def clicks(ctx, v, ui):
    side, right, page = ui.side, ui.right, ui.page
    p = panels(ui)
    v.check("the phone shows one panel, the tree, with no sideways scroll",
            p == {"panel": "tree", "left": True, "right": False, "sideways": False}, str(p))
    who = ui.texts(side.locator("#wait .w .who b"))
    v.check("the tree panel lists who waits for the person", who == ["k-rollout", "m-ads"], str(who))

    v.check("a tap on k-rollout's row opens it", ui.open("k_rollout"), ui.right_now())
    ok, p = ui.settle(lambda: panels(ui), {"panel": "agent", "left": False, "right": True, "sideways": False})
    v.check("the one panel is now the agent; the tree is gone", ok, str(p))
    ok, got = ui.waits(right.locator("#name"), "k-rollout", exact=True)
    v.check("the panel is k-rollout's own view", ok, f"{got!r}; {ui.right_now()}")
    ok, got = ui.waits(right.locator(".inq .q"), D.QUESTION, exact=True)
    v.check("with k-rollout's question", ok, got)
    try:
        box = right.locator("#send").first.bounding_box(timeout=4000)
        frame = page.locator("#body-right iframe").first.bounding_box(timeout=4000)
        # a frame's element box is already in the page's own coordinates (Playwright: relative to the main frame)
        fits = (box is not None and frame is not None and box["x"] >= frame["x"] and box["y"] >= frame["y"]
                and box["x"] + box["width"] <= min(390, frame["x"] + frame["width"])
                and box["y"] + box["height"] <= min(844, frame["y"] + frame["height"]))
    except Exception as e:
        fits, box = False, type(e).__name__
    v.check("the composer's Send button is on the screen", fits, str(box))
    ok, got = ui.settle(lambda: ui.texts(page.locator("#back-badge:not([hidden])")), ["1"])
    v.check("the back button counts the one OTHER agent that waits (m-ads)", ok, str(got))
    page.locator("#back").click()
    ok, p = ui.settle(lambda: panels(ui), {"panel": "tree", "left": True, "right": False, "sideways": False})
    v.check("back returns to the tree", ok, str(p))
    v.check("the tree is still drawn after coming back", ui.texts(ui.item("k_rollout").locator(".name")) == ["k-rollout"])

    v.check("a tap on Lead's row opens it", ui.open("lead"), ui.right_now())
    ok, got = ui.waits(right.locator("#name"), "Lead", exact=True)
    v.check("the panel is Lead's own view", ok, f"{got!r}; {ui.right_now()}")
    ok, got = ui.settle(lambda: ui.texts(page.locator("#back-badge:not([hidden])")), ["2"])
    v.check("for an agent that asks nothing, the back button counts both waiting agents", ok, str(got))
    page.locator("#back").click()
    ui.settle(lambda: panels(ui)["panel"], "tree")

    v.check("a tap on the task registry's row opens it", ui.open("tasks"), ui.right_now())
    ok, got = ui.waits(right.locator("#k-tasks"), str(D.TOTAL), exact=True)
    v.check("the panel is the registry's view with the seeded tasks counted", ok, f"{got!r}; {ui.right_now()}")
    p = panels(ui)
    v.check("still one panel and no sideways scroll", p == {"panel": "agent", "left": False, "right": True, "sideways": False}, str(p))
