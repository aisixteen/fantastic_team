"""The page of a never-used folder (#351): what a person sees on the product's own seed
(`crates/team/blueprint.yaml`), with no record written by a test. The tree lists the seeded agents, and a
click on each opens that agent's own view. Function only."""

from helpers import app, stock as S, team_ui as D
from helpers.verdict import Verdict

NAME = "ui/seeded_page_opens_agents"


def run(ctx):
    v = Verdict(NAME)
    pw = D.playwright(v)
    if pw is None:
        return v.finish()
    port = app.free_port()
    S.boot(ctx, extra_args=["webui", "--port", port], seeded=True)
    if not v.check("the page is served on --port", app.wait_http(f"http://127.0.0.1:{port}{D.PAGE}", 30)):
        D.stop(ctx, v)
        return v.finish()
    try:
        with D.Ui(pw, port) as ui:
            if ui.tree_ready(v):
                clicks(ctx, v, ui)
            ui.no_errors(v)
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    D.stop(ctx, v)
    return v.finish()


def clicks(ctx, v, ui):
    right, page, team = ui.right, ui.page, ctx.team
    root = D.op(team, "kernel", {"type": "reflect"})
    name = root.get("display_name") or root.get("id") or ""
    ok, got = ui.waits(page.locator("#title"), f"team / {name}", exact=True)
    v.check(f"the title bar names the team and its kernel ({name})", ok and bool(name), got)
    seeded = ["lead", "tasks", "github", "bundles"]
    missing = [id for id in seeded if not ui.reveal(id)]
    v.check("the tree lists the seeded lead, task registry, connector and bundles registry", not missing, f"no row for {missing}")

    v.check("a click on the lead's row opens it", ui.open("lead"), ui.right_now())
    ok, got = ui.waits(right.locator("#text"), timeout=15)
    v.check("the right cell is the lead's chat view, with a composer to write to it", ok, ui.right_now())
    ok, got = ui.waits(right.locator("#name"), lambda t: t != "", timeout=5)
    v.check("the chat is titled with the lead's name", ok, f"{got!r}; {ui.right_now()}")

    v.check("a click on the task registry's row opens it", ui.open("tasks"), ui.right_now())
    ok, got = ui.waits(right.locator("#k-tasks"), "0", exact=True, timeout=15)
    v.check("the right cell is the registry's view: no task yet", ok and ui.waits(right.locator("#note"), "no tasks yet", exact=True)[0],
            f"{got!r}; {ui.right_now()}")

    v.check("a click on the GitHub connector's row opens it", ui.open("github"), ui.right_now())
    mode = (D.op(team, "github", {"type": "reflect"}).get("auth") or {}).get("mode")
    ok, got = ui.waits(right.locator("#auth"), f"auth: {mode}", exact=True, timeout=15)
    v.check(f"the right cell is the connector's view with its auth mode ({mode})", ok, f"{got!r}; {ui.right_now()}")

    v.check("a click on the bundles registry's row opens it", ui.open("bundles"), ui.right_now())
    running = D.op(team, "bundles", {"type": "get_count"}).get("running")
    ok, got = ui.waits(right.locator("#run"), f"{running} running", exact=True, timeout=15)
    v.check("the right cell is the registry's view with its running count", ok, f"{got!r}; {ui.right_now()}")
