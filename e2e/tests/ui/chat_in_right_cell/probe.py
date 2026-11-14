"""The team page's right cell draws the base chat view for the agent picked in the left cell (#609, #260):
person and agent bubbles are drawn, and a message typed in the view reaches the agent.

Both cells are null-origin srcdoc iframes; Playwright's frame locator reads their documents. Without
Playwright the probe fails with a message (it never skips)."""

import json
import secrets
import time

from helpers import app, stock as S
from helpers.verdict import Verdict
from helpers.web import WsClient

NAME = "ui/chat_in_right_cell"
PAGE = "/team_page/file/index.html"
ROW = "#tree [role=treeitem]"


def run(ctx):
    v = Verdict(NAME)
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        v.check("Playwright is installed (python3 -m pip install --user playwright==1.55.0 && "
                "Google Chrome installed)", False, "import playwright failed")
        return v.finish()

    port = app.free_port()
    ctx.vars["web_port"] = port
    S.boot(ctx)
    S.serve_webui(ctx.team, port)
    v.check("the page is served on --port", app.wait_http(f"http://127.0.0.1:{port}{PAGE}", 30))

    token = "TOK_" + secrets.token_hex(6)
    errors = []
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(channel="chrome")
            try:
                page = browser.new_page(viewport={"width": 1440, "height": 900})
                page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
                page.on("pageerror", lambda e: errors.append(str(e)))
                page.goto(f"http://127.0.0.1:{port}{PAGE}")
                side = page.frame_locator("#body-left iframe")
                try:
                    side.locator(ROW).first.wait_for(timeout=30000)
                except Exception:
                    pass
                ids = side.locator(ROW).evaluate_all("els => els.map(e => e.dataset.id).filter(Boolean)")
                if "w1" not in ids:
                    fold = side.locator("[aria-label='Unfold lead']")
                    if fold.count():
                        fold.first.click()
                try:
                    side.locator("#tree [data-id=w1] .main").wait_for(timeout=15000)
                    row = True
                except Exception:
                    row = False
                v.check("the agent's row is drawn in the left cell", row)

                # 1. a click opens it; the right cell draws the chat view
                side.locator("#tree [data-id=w1] .main").click()
                try:
                    page.wait_for_selector("#body-right[data-open=w1]", timeout=10000)
                    opened = True
                except Exception:
                    opened = False
                v.check("a click on the row opens the agent in the right cell", opened)
                chat = page.frame_locator("#body-right iframe")
                try:
                    chat.locator("#text").wait_for(timeout=30000)
                    drawn = True
                except Exception:
                    drawn = False
                v.check("the right cell draws the chat view (composer #text and #send)",
                        drawn and chat.locator("#send").count() == 1)

                # 2. a message typed in the view: the person's bubble, then the agent's reply bubble
                chat.locator("#text").fill(token)
                chat.locator("#send").click()
                try:
                    chat.locator(".who.user").filter(has_text=token).first.wait_for(timeout=20000)
                    mine = True
                except Exception:
                    mine = False
                v.check("the person's bubble is visible", mine)
                try:
                    chat.locator(".who.assistant").filter(has_text="REPLY_FROM_THE_SCRIPTED_MODEL").first.wait_for(timeout=30000)
                    theirs = True
                except Exception:
                    theirs = False
                v.check("the agent's reply bubble is visible", theirs)
                v.check("the bubbles are drawn (visible)",
                        mine and theirs and chat.locator(".who.user").first.is_visible()
                        and chat.locator(".who.assistant").first.is_visible())
            finally:
                browser.close()
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")

    # 3. the message reached the agent (the model's request and the agent's history, over the leg)
    end = time.time() + 20
    while time.time() < end and ctx.model.served < 1:
        time.sleep(0.2)
    v.check("the scripted model got one request holding the token",
            ctx.model.served == 1 and token in "".join(ctx.model.raw_bodies), f"served {ctx.model.served}")
    ws = WsClient(port, "/team_ws/ws")
    try:
        hist = json.dumps(ws.call("w1", {"type": "history"}, timeout=30))
        v.check("the agent's history holds the person's message", token in hist, hist[:300])
    except (RuntimeError, OSError) as e:
        v.check("the history is readable over the page's leg", False, str(e))
    finally:
        ws.close()
    v.check("no console error", not errors, str(errors[:3]))

    code = ctx.team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
    return v.finish()
