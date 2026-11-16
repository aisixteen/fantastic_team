"""The main page, used (#351): the left tree lists the team's real agents, "Waiting for you" holds a real
question, and a click on each kind of row opens THAT agent's own view, with its data, in the right cell.
Function only: no check is about the look or the wording of a label.

The fleet is `fleets/team_ui`; see `helpers/team_ui.py` for what is seeded and how."""

import re

from helpers import team_ui as D
from helpers.verdict import Verdict

NAME = "ui/main_opens_each_agent"
CHAT_HEAD = "#name"  # the base chat view's title


def run(ctx):
    v = Verdict(NAME)
    pw = D.playwright(v)
    if pw is None:
        return v.finish()
    port = D.start(ctx, v, cards=True)
    if port is None:
        return v.finish()
    D.seed_tasks(ctx, v)
    D.seed_inquiries(ctx, v)
    try:
        with D.Ui(pw, port) as ui:
            clicks(ctx, v, ui)
            ui.no_errors(v)
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    D.stop(ctx, v)
    return v.finish()


def chat_opens(v, ui, id: str, how: str) -> bool:
    """The right cell is the chat view of `id`: its title is the agent's own name."""
    ok, got = ui.waits(ui.right.locator(CHAT_HEAD), D.NAMES[id], exact=True)
    return v.check(f"{how}: the right cell is {D.NAMES[id]}'s chat view (its title is its name)", ok,
                   f"title {got!r}; the right cell holds {ui.right_now()}")


def clicks(ctx, v, ui):
    side, right, page = ui.side, ui.right, ui.page
    if not ui.tree_ready(v):
        return

    # ---- the title bar and the tree ----
    ok, got = ui.waits(page.locator("#title"), f"team / {D.ROOT_NAME}", exact=True)
    v.check(f"the title bar names the team and its kernel: 'team / {D.ROOT_NAME}'", ok, got)
    ok, got = ui.waits(side.locator("#ttl"), D.ROOT_NAME, exact=True)
    v.check("the tree's head names the kernel", ok, got)
    for id in D.NAMES:
        ui.reveal(id)
    names = {id: (ui.texts(ui.item(id).locator(".name")) or ["(no row)"])[0] for id in D.NAMES}
    v.check("the tree lists the six AI agents by their own names", names == D.NAMES, str(names))
    pad = {id: ui.attr(ui.item(id), "style") for id in ("lead", "ops", "k_rollout")}
    depth = {id: int((re.search(r"padding-left:\s*(\d+)", s) or [0, -1])[1]) for id, s in pad.items()}
    v.check("the tree nests them as the fleet does: Lead > cluster ops > k-rollout",
            depth["lead"] < depth["ops"] < depth["k_rollout"], str(depth))
    tools = ["tasks", "github", "bundles", "cadencer", "worker_exec"]
    missing = [id for id in tools if not ui.reveal(id)]
    v.check("the tool agents of the fleet are rows a person can reach", not missing, f"no row for {missing}")

    # ---- Waiting for you ----
    ok, got = ui.waits(side.locator("#wait h2"), "Waiting for you")
    v.check("'Waiting for you' is shown", ok, got)
    v.check("its count is the two open questions", ui.texts(side.locator("#wait h2 .pill")) == ["2"],
            str(ui.texts(side.locator("#wait h2 .pill"))))
    who = ui.texts(side.locator("#wait .w .who b"))
    v.check("its rows name the askers by their names, the oldest first", who == ["k-rollout", "m-ads"], str(who))
    asked = ui.texts(side.locator("#wait .w .who > span"))
    v.check("each row carries the agent's own question", asked == [D.QUESTION, D.BUDGET], str(asked))
    ages = ui.texts(side.locator("#wait .w .age"))
    v.check("each row carries the question's age", len(ages) == 2 and all(re.fullmatch(r"now|\d+[mhd]", a) for a in ages),
            str(ages))
    row = ui.item("k_rollout")
    v.check("k-rollout's tree row carries the '?' badge and 'asks you'",
            ui.texts(row.locator(".badge.ask")) == ["?"] and ui.texts(row.locator(".sub")) == ["asks you"],
            f"{ui.texts(row.locator('.badge'))} {ui.texts(row.locator('.sub'))}")

    # ---- a click on a waiting row opens that agent ----
    ui.click(side.locator("#wait .w").filter(has_text="k-rollout"))
    ok, _ = ui.settle(lambda: ui.attr(page.locator("#body-right"), "data-open"), "k_rollout")
    v.check("a click on k-rollout's waiting row opens k-rollout", ok, ui.right_now())
    chat_opens(v, ui, "k_rollout", "from 'Waiting for you'")
    ok, got = ui.waits(right.locator("#state"), "asks you")
    v.check("its header marks it as asking the person ('?', 'asks you')", ok and ui.texts(right.locator("#badge")) == ["?"],
            f"state {got!r} badge {ui.texts(right.locator('#badge'))}")
    ok, got = ui.waits(right.locator(".who.user").filter(has_text=D.TASK_TEXT))
    v.check("its dialog holds the task cluster ops handed it", ok, got[:120])
    ok, got = ui.waits(right.locator(".who.assistant").filter(has_text=D.FINDING))
    v.check("its dialog holds k-rollout's own words", ok, got[:120])
    ok, got = ui.waits(right.locator(".inq .q"), D.QUESTION, exact=True)
    v.check("the chat shows k-rollout's open question", ok, got)
    buttons = [b for b in ui.texts(right.locator(".inq button")) if b in D.OPTIONS]
    v.check("the card offers the question's three options as buttons", buttons == D.OPTIONS,
            str(ui.texts(right.locator(".inq button"))))
    v.check("the opened agent's row is the selected one in the tree",
            "sel" in ui.attr(ui.item("k_rollout"), "class"), ui.attr(ui.item("k_rollout"), "class"))

    # ---- each kind of row opens its own view ----
    v.check("a click on Lead's row opens it", ui.open("lead"), ui.right_now())
    if chat_opens(v, ui, "lead", "an AI agent's row"):
        ok, _ = ui.waits(right.locator("#text"))
        v.check("Lead's chat is its own: none of k-rollout's messages, no question card",
                ok and not ui.texts(right.locator(".who").filter(has_text=D.TASK_TEXT))
                and not ui.texts(right.locator(".inq")), str(ui.texts(right.locator(".who"))[:2]))
    else:
        v.check("Lead's chat is its own: none of k-rollout's messages, no question card", False,
                ui.right_now())
    v.check("a click on m-ads's row opens it", ui.open("m_ads"), ui.right_now())
    chat_opens(v, ui, "m_ads", "a sub-agent's row")
    ok, got = ui.waits(right.locator(".inq .q"), D.BUDGET, exact=True)
    v.check("m-ads's chat shows m-ads's own question", ok, got)

    v.check("a click on the task registry's row opens it", ui.open("tasks"), ui.right_now())
    ok, got = ui.waits(right.locator("#sub"),
                       f"{D.TOTAL} tasks · {D.COUNT['running']} held · {D.COUNT['blocked']} blocked", exact=True)
    v.check("the right cell is the registry's own view with the fleet's tasks counted", ok,
            f"{got!r}; {ui.right_now()}")
    v.check("the registry view is titled with the agent's name", ui.texts(right.locator("#name")) == ["tasks"],
            str(ui.texts(right.locator("#name"))))

    v.check("a click on the cadencer's row opens it", ui.open("cadencer"), ui.right_now())
    ok, got = ui.waits(right.locator("#state"), "on", exact=True)
    v.check("the right cell is the cadencer's own view: its chip says the switch is on", ok,
            f"{got!r}; {ui.right_now()}")
    off = ui.texts(right.locator('button[aria-label^="switch digests on for "]'))
    v.check("the cadencer view lists the lead its record switches off (m_ads)", off == ["m_ads ×"], str(off))

    v.check("a click on the GitHub connector's row opens it", ui.open("github"), ui.right_now())
    mode = (D.op(ctx.team, "github", {"type": "reflect"}).get("auth") or {}).get("mode")
    ok, got = ui.waits(right.locator("#auth"), f"auth: {mode}", exact=True)
    v.check(f"the right cell is the connector's own view: its chip is the connector's auth mode ({mode})", ok,
            f"{got!r}; {ui.right_now()}")
    allow = ui.texts(right.locator("dl.kv dd").first.locator(".pill"))
    v.check("the connector view lists the fleet's repo (acme/widgets)", allow == ["acme/widgets"], str(allow))

    v.check("a click on the bundles registry's row opens it", ui.open("bundles"), ui.right_now())
    kinds = [b.get("name") for b in D.op(ctx.team, "bundles", {"type": "bundles"}).get("bundles", [])]
    ok, _ = ui.waits(right.locator('button[aria-label^="kind "]'))
    shown = ui.texts(right.locator('button[aria-label^="kind "] b'))
    v.check("the right cell is the registry's own view: its kinds are the bundles the registry lists",
            ok and bool(kinds) and shown == kinds, f"shown {shown} registry {kinds}; {ui.right_now()}")
    # ---- an agent with no view: the name card ----
    v.check("a click on an agent with no view (worker_exec) opens it", ui.open("worker_exec"), ui.right_now())
    ok, got = ui.waits(page.locator("#body-right .card .name"), "worker_exec", exact=True)
    v.check("the right cell is the name card: the agent's name", ok, f"{got!r}; {ui.right_now()}")
    card = ui.texts(page.locator("#body-right .card > div"))
    v.check("the card carries the agent's bundle (exec_bridge.tools) and holds no view",
            card[:2] == ["worker_exec", "exec_bridge.tools"] and page.locator("#body-right iframe").count() == 0, str(card))

    # ---- the filter ----
    try:
        side.locator("#filter").fill("rollout", timeout=4000)
    except Exception:
        pass
    ok, got = ui.settle(lambda: ui.texts(side.locator("#tree [role=treeitem] .name")),
                        lambda rows: rows[-3:] == ["Lead", "cluster ops", "k-rollout"] and "marketing" not in rows)
    v.check("the filter keeps the matching agent and the branch that leads to it", ok, str(got))
