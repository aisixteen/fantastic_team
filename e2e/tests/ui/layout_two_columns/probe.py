"""The team page's layout in a real browser: two columns on a desktop, one on a phone.

The team binary serves the page on a port (the `team_page` layer lays the web host with
`{{web_port}}`); headless Chromium (Playwright) opens `/team_page/file/index.html` at two
sizes and measures the two empty cells `#col-left` and `#col-right`. Without Playwright the
probe fails with a message (it never skips)."""

import time

from helpers import app, stock as S
from helpers.verdict import Verdict

NAME = "ui/layout_two_columns"
PAGE = "/team_page/file/index.html"
RECT = """(sel) => { const r = document.querySelector(sel).getBoundingClientRect();
  return {left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height}; }"""


def visible(page, sel) -> bool:
    return page.locator(sel).is_visible()


def run(ctx):
    v = Verdict(NAME)
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        v.check("Playwright is installed (python3 -m pip install --user playwright==1.55.0 && "
                "python3 -m playwright install --with-deps chromium)", False, "import playwright failed")
        return v.finish()

    port = app.free_port()
    ctx.vars["web_port"] = port
    S.boot(ctx)
    v.check("the page is served on --port", app.wait_http(f"http://127.0.0.1:{port}{PAGE}", 30))

    errors = []
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch()
            try:
                page = browser.new_page(viewport={"width": 1440, "height": 900})
                page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
                page.on("pageerror", lambda e: errors.append(str(e)))
                page.goto(f"http://127.0.0.1:{port}{PAGE}")
                page.wait_for_selector("#col-left", timeout=15000)

                # 1. desktop: two columns side by side
                left, right = page.evaluate(RECT, "#col-left"), page.evaluate(RECT, "#col-right")
                v.check("1440x900: both cells are visible",
                        visible(page, "#col-left") and visible(page, "#col-right"))
                v.check("1440x900: the cells share one top (within 2px)",
                        abs(left["top"] - right["top"]) <= 2, f"{left['top']} {right['top']}")
                v.check("1440x900: the left cell ends before the right begins",
                        left["right"] <= right["left"], f"{left['right']} {right['left']}")
                v.check("1440x900: both cells are at least 400px tall",
                        left["height"] >= 400 and right["height"] >= 400, f"{left['height']} {right['height']}")

                # 2. phone: one column, left above right
                page.set_viewport_size({"width": 390, "height": 844})
                time.sleep(0.3)
                left, right = page.evaluate(RECT, "#col-left"), page.evaluate(RECT, "#col-right")
                v.check("390x844: the right cell is below the left cell (stacked)",
                        right["top"] >= left["bottom"], f"{left['bottom']} {right['top']}")
                v.check("390x844: both cells are at least 340px wide",
                        left["width"] >= 340 and right["width"] >= 340, f"{left['width']} {right['width']}")
                scroll = page.evaluate("document.documentElement.scrollWidth")
                v.check("390x844: no horizontal scroll", scroll <= 390, f"scrollWidth {scroll}")
            finally:
                browser.close()
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    v.check("no console error", not errors, str(errors[:3]))

    code = ctx.team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
    return v.finish()
