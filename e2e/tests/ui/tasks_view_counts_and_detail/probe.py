"""The task registry's view, used (#351): opened from the tree, it counts the registry's tasks per real
state (failed and paused too), its filters change the rows, and a click on a task shows that task's
holder, versions, history and last report, all from the registry's data. Function only."""

import re

from helpers import team_ui as D
from helpers.verdict import Verdict

NAME = "ui/tasks_view_counts_and_detail"
ORDER = ["blocked", "failed", "running", "paused", "waiting", "done"]  # the order the view groups in


def run(ctx):
    v = Verdict(NAME)
    pw = D.playwright(v)
    if pw is None:
        return v.finish()
    port = D.start(ctx, v)
    if port is None:
        return v.finish()
    D.seed_tasks(ctx, v)
    try:
        with D.Ui(pw, port) as ui:
            if ui.tree_ready(v):
                clicks(ctx, v, ui)
            ui.no_errors(v)
    except Exception as e:  # a missing browser or a crashed page is a failure, never a skip
        v.check("the browser ran every step", False, f"{type(e).__name__}: {e}")
    D.stop(ctx, v)
    return v.finish()


def rows(ui) -> list[str]:
    """The ids of the task rows drawn, top to bottom."""
    return ui.texts(ui.right.locator("#tbl button.tr > span:first-child"))


def row(ui, tid: str):
    return ui.right.locator(f'#tbl button.tr[title="{tid}"]')


def choose(ui, select: str, value: str) -> None:
    try:
        ui.right.locator(select).select_option(value, timeout=4000)
    except Exception:
        pass


def clicks(ctx, v, ui):
    right = ui.right
    v.check("a click on the task registry's row opens it", ui.open("tasks"), ui.right_now())
    c = D.COUNT
    ok, got = ui.waits(right.locator("#sub"), f"{D.TOTAL} tasks · {c['running']} held · {c['blocked']} blocked", exact=True)
    if not v.check("the right cell is the registry's view: the tasks counted from the registry", ok,
                   f"{got!r}; {ui.right_now()}"):
        return

    # ---- the counts ----
    v.check("the key figure is the number of seeded tasks", ui.texts(right.locator("#k-tasks")) == [str(D.TOTAL)],
            str(ui.texts(right.locator("#k-tasks"))))
    want = f"{c['running']} held · {c['waiting']} waiting · {c['blocked']} blocked"
    v.check(f"under it: {want}", ui.texts(right.locator("#k-tasks-s")) == [want], str(ui.texts(right.locator("#k-tasks-s"))))
    holders = [str(D.AGENT_HOLDERS + D.SUBTREE_HOLDERS), f"{D.AGENT_HOLDERS} agents · {D.SUBTREE_HOLDERS} subtrees"]
    got = ui.texts(right.locator("#k-holders")) + ui.texts(right.locator("#k-holders-s"))
    v.check("the holders are counted from the tasks: agents and subtrees", got == holders, f"{got} want {holders}")
    legend = {}
    for t in ui.texts(right.locator("#status-in .leg > span")):
        m = re.fullmatch(r"(\d+) (\w+)", t)
        if m:
            legend[m[2]] = int(m[1])
    v.check("the status counts equal the seeded tasks per state, failed and paused included",
            legend == c, f"shown {legend} seeded {c}")
    try:
        bar = right.locator("#status-in .bar i").evaluate_all("els => els.map(e => parseFloat(e.style.flexGrow))")
    except Exception as e:
        bar = [type(e).__name__]
    v.check("the status bar has one segment per state, sized by its count",
            sorted(bar) == sorted(float(n) for n in c.values()), str(bar))
    heads = [re.sub(r"\s+", " ", t).lower() for t in ui.texts(right.locator("#tbl .gh"))]
    v.check("the table groups the tasks by state, each group with its count",
            heads == [f"{s}{c[s]}" for s in ORDER] or heads == [f"{s} {c[s]}" for s in ORDER], str(heads))

    # ---- the rows ----
    by_state = {s: [t[0] for t in D.TASKS if t[2] == s] for s in ORDER}
    folded = [tid for s in ORDER for tid in (by_state[s][:D.DONE_SHOWN] if s == "done" else by_state[s])]
    v.check("the rows are the seeded tasks, by state; the done ones past three are folded", rows(ui) == folded,
            f"{rows(ui)}")
    more = right.locator("#tbl button.more")
    v.check("the folded done tasks are counted on a button", ui.texts(more) == [f"{c['done'] - D.DONE_SHOWN} more done"],
            str(ui.texts(more)))
    ui.click(more)
    ok, got = ui.settle(lambda: rows(ui), [tid for s in ORDER for tid in by_state[s]])
    v.check("a click on it shows every task", ok, str(got))
    shown = {tid: ui.texts(row(ui, tid).locator("span")) for tid, _, _ in D.TASKS}
    wrong = {tid: shown[tid] for tid, holder, state in D.TASKS if shown[tid][1:3] != [D.holder_text(holder), state]}
    v.check("every row shows its task's holder (an agent, a subtree, or nobody) and its state", not wrong, str(wrong)[:300])

    # ---- the filters ----
    choose(ui, "#f-status", "failed")
    ok, got = ui.settle(lambda: rows(ui), by_state["failed"])
    v.check("the status filter 'failed' leaves the one failed task", ok, str(got))
    choose(ui, "#f-status", "paused")
    ok, got = ui.settle(lambda: rows(ui), by_state["paused"])
    v.check("the status filter 'paused' leaves the one paused task", ok, str(got))
    choose(ui, "#f-status", "")
    choose(ui, "#f-holder", "a:lead")
    ok, got = ui.settle(lambda: rows(ui), by_state["done"])
    v.check("the holder filter 'lead' leaves the tasks lead holds", ok, str(got))
    choose(ui, "#f-holder", "s:mkt")
    ok, got = ui.settle(lambda: rows(ui), [D.R + "219"])
    v.check("the holder filter 'subtree mkt' leaves the task that subtree holds", ok, str(got))
    choose(ui, "#f-status", "failed")
    ok, got = ui.waits(right.locator("#note"), "no task matches the filters", exact=True)
    v.check("two filters that match nothing say so, with no rows", ok and rows(ui) == [], f"{got!r} {rows(ui)}")
    choose(ui, "#f-status", "")
    choose(ui, "#f-holder", "")
    ok, got = ui.settle(lambda: len(rows(ui)), D.TOTAL)
    v.check("clearing the filters brings every task back", ok, str(got))

    # ---- a task's detail ----
    truth = D.op(ctx.team, "tasks", {"type": "get", "id": D.HELD})
    ui.click(row(ui, D.HELD))
    ok, got = ui.waits(right.locator("#d-id"), D.HELD, exact=True)
    v.check("a click on a task row shows that task's detail", ok and ui.texts(right.locator("#d-state")) == ["running"],
            f"{got!r} {ui.texts(right.locator('#d-state'))}")
    v.check("the detail names its tracker source and its holder",
            ui.texts(right.locator("#d-source")) == ["gh:" + D.HELD] and ui.texts(right.locator("#d-holder")) == ["k_rollout"],
            f"{ui.texts(right.locator('#d-source'))} {ui.texts(right.locator('#d-holder'))}")
    version = ui.texts(right.locator("#d-vt .vt:not(.th) span"))
    v.check("the detail lists the task's version (repo, branch, commit)",
            version == [D.VERSION["repo"], D.VERSION["branch"], D.VERSION["commit"]], str(version))
    steps = [ui.texts(r.locator("span")) for r in right.locator("#h-in .row2").all()]  # [dot, from, "to", to, when]
    hist = truth.get("history") or []
    v.check("the history shows the task's two hand-overs from the registry: nobody to ops, ops to k_rollout",
            len(hist) == 2 and [s[1:4:2] for s in steps if len(s) >= 5] == [["nobody", "ops"], ["ops", "k_rollout"]],
            f"shown {steps} registry {hist}")
    by = [h.get("by") for h in hist]
    v.check("each step names who made it, as the registry recorded it",
            len(steps) == len(by) == 2 and all(f"by {b}" in s[-1] for s, b in zip(steps, by) if b), f"{steps} by {by}")
    ok, got = ui.waits(right.locator("#h-in .sub"), lambda t: t.startswith("Last report from k_rollout") and t.endswith("state: running"))
    v.check("the last report names its sender and state", ok, got)
    v.check("the last report's text is the one the agent reported", ui.texts(right.locator("#h-in pre")) == [D.REPORT],
            str(ui.texts(right.locator("#h-in pre"))))
    ui.click(row(ui, D.BARE))
    ok, got = ui.waits(right.locator("#d-id"), D.BARE, exact=True)
    v.check("a click on another row shows THAT task: held by nobody, nothing reported",
            ok and ui.texts(right.locator("#d-holder")) == ["nobody"] and ui.texts(right.locator("#h-in .empty")) == ["nothing yet"],
            f"{got!r} {ui.texts(right.locator('#d-holder'))} {ui.texts(right.locator('#h-in'))}")

    # ---- it follows the registry ----
    D.op(ctx.team, "tasks", {"type": "status", "id": D.BARE, "status": "blocked"})
    ok, got = ui.waits(right.locator("#chip-blocked"), f"{c['blocked'] + 1} blocked", exact=True, timeout=20)
    v.check("a task that changes state in the registry changes on the page without a reload", ok, got)
