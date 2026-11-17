"""The team page's left cell draws the base sidebar view (#608) in a real browser: agent rows (the
lead and its child), the Files switch shows the project's files in the tree, a click on an agent opens it.

The view is a null-origin srcdoc iframe in `#body-left`; Playwright's frame locator reads its document.
Without Playwright the probe fails with a message (it never skips)."""

import os

from helpers import app, stock as S
from helpers.verdict import Verdict

NAME = "ui/sidebar_renders"
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
    # the project's files, in the folder the `files` agent roots at
    os.makedirs(os.path.join(ctx.work, "docs"), exist_ok=True)
    with open(os.path.join(ctx.work, "docs", "a.md"), "w") as f:
        f.write("# a\n")
    with open(os.path.join(ctx.work, "notes.txt"), "w") as f:
        f.write("notes\n")
    S.serve_webui(ctx.team, port)
    v.check("the page is served on --port", app.wait_http(f"http://127.0.0.1:{port}{PAGE}", 30))

    errors = []
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(channel="chrome")
            try:
                page = browser.new_page(viewport={"width": 1440, "height": 900})
                page.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
                page.on("pageerror", lambda e: errors.append(str(e)))
                page.goto(f"http://127.0.0.1:{port}{PAGE}")
                view = page.frame_locator("#body-left iframe")

                # 1. the base view is drawn in the left cell
                try:
                    view.locator(ROW).first.wait_for(timeout=30000)
                    drawn = True
                except Exception:
                    drawn = False
                v.check("the left cell draws the sidebar view (agent rows)", drawn)
                v.check("its Kernels rail, Agents nav and Files switch (off) are there",
                        view.locator("nav[aria-label=Kernels]").count() == 1
                        and view.locator("nav[aria-label=Agents]").count() == 1
                        and view.locator("#sw").get_attribute("aria-checked") == "false"
                        and view.locator("#full").get_attribute("aria-checked") == "false")

                # 2. agent rows: the lead and its child
                ids = view.locator(ROW).evaluate_all("els => els.map(e => e.dataset.id).filter(Boolean)")
                v.check("the lead's row is drawn", "lead" in ids, str(ids))
                if "child_one" not in ids:  # not an AI agent: a row of the tools group, opened
                    group = view.locator("#pins button.tg[aria-expanded='false']")
                    if group.count():
                        group.first.click()
                ids = view.locator("[role=treeitem]").evaluate_all("els => els.map(e => e.dataset.id).filter(Boolean)")
                v.check("the lead's child row is drawn", "child_one" in ids, str(ids))

                # 3. the Files switch puts the project's files in the tree
                v.check("no file row while the switch is off",
                        view.locator("[data-path]").count() == 0)
                view.locator("#sw").click()
                try:
                    view.locator("[data-path='notes.txt']").wait_for(timeout=20000)
                    shown = True
                except Exception:
                    shown = False
                v.check("the Files switch shows the project's files as rows (notes.txt, docs)",
                        shown and view.locator("[data-path='docs']").count() == 1)

                # 4. the Full switch shows the whole kernel tree; off again, compact comes back
                view.locator("#full").click()
                try:
                    view.locator("#tree [data-id=files]").wait_for(timeout=10000)
                    full = True
                except Exception:
                    full = False
                tree_ids = view.locator("#tree [role=treeitem]").evaluate_all("els => els.map(e => e.dataset.id).filter(Boolean)")
                v.check("Full: the whole kernel tree, with the agents of the tools group and files in their places",
                        full and "child_one" in tree_ids and "lead" in tree_ids and view.locator("#pins").is_hidden(),
                        str(tree_ids))
                view.locator("#full").click()
                try:
                    view.locator("#pins").wait_for(state="visible", timeout=10000)
                    compact = True
                except Exception:
                    compact = False
                tree_ids = view.locator("#tree [role=treeitem]").evaluate_all("els => els.map(e => e.dataset.id).filter(Boolean)")
                v.check("Full off: compact again (files and the tools group pinned below, not in the tree)",
                        compact and "files" not in tree_ids and view.locator("#full").get_attribute("aria-checked") == "false",
                        str(tree_ids))

                # 5. a click on an agent opens it
                view.locator("#tree [data-id=lead] .main").click()
                try:
                    page.wait_for_selector("#body-right[data-open=lead]", timeout=10000)
                    opened = True
                except Exception:
                    opened = False
                v.check("a click on the lead's row opens it (the right cell names it)", opened)
            finally:
                browser.close()
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    v.check("no console error", not errors, str(errors[:3]))

    code = ctx.team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
    return v.finish()
