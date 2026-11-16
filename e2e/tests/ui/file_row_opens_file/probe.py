"""A file opened from the tree (#351): with the files shown, the tree lists the project's real files, and
a click on a file opens THAT file, with its text, in the right cell. Function only."""

import os

from helpers import team_ui as D
from helpers.verdict import Verdict

NAME = "ui/file_row_opens_file"
CARDS, SIZE = True, (1440, 900)


def seed(ctx, v):
    """Nothing to make: the files are laid before the start (`team_ui.lay_files`)."""


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


def file_row(ui, path: str):
    return ui.side.locator(f'[role=treeitem][data-path="{path}"]')


def clicks(ctx, v, ui):
    side, right = ui.side, ui.right
    ui.click(side.locator("#sw"))
    ok, got = ui.settle(lambda: ui.attr(side.locator("#sw"), "aria-checked"), "true")
    v.check("the Files switch turns the files on", ok, str(got))
    ok, _ = ui.waits(file_row(ui, "deploy"), timeout=20)
    v.check("the project's folder 'deploy' is a row", ok, str(ui.texts(side.locator("[role=treeitem][data-path] .name"))[:12]))
    for folder in ("deploy", "deploy/api"):
        if not file_row(ui, D.FILE_PATH).count():
            ui.click(file_row(ui, folder).locator(".main"), tries=3)
            ui.waits(file_row(ui, folder + "/api" if folder == "deploy" else D.FILE_PATH), timeout=10)
    names = sorted(ui.texts(side.locator('[role=treeitem][data-path^="deploy/api/"] .name')))
    v.check("unfolded, deploy/api lists its three real files", names == ["api.yaml", "hpa.yaml", "rollout.log"], str(names))

    ui.click(file_row(ui, D.FILE_PATH).locator(".main"), tries=3)
    ok, got = ui.settle(lambda: "sel" in ui.attr(file_row(ui, D.FILE_PATH), "class"), True)
    v.check("a click on api.yaml selects its row", ok, str(got))
    ok, got = ui.waits(right.locator("#name"), "api.yaml", exact=True, timeout=20)
    if not v.check("a click on api.yaml opens that file in the right cell", ok, f"{got!r}; {ui.right_now()}"):
        return
    ok, _ = ui.waits(right.locator("#body .ln"), timeout=20)
    lines = [t.lstrip("0123456789") for t in ui.texts(right.locator("#body .ln"))]
    v.check("the right cell shows the file's text, line by line",
            ok and [l.strip() for l in lines] == [l.strip() for l in D.FILE_LINES], str(lines)[:300])
    v.check("the view says where the file is (its path)",
            all(p in " ".join(ui.texts(right.locator("#crumbs"))) for p in ("deploy", "api", "api.yaml")),
            str(ui.texts(right.locator("#crumbs"))))
    with open(os.path.join(ctx.run_dir, D.FILE_PATH), "a") as f:
        f.write("# CHANGED_ON_DISK\n")
    ok, got = ui.waits(right.locator("#body"), "CHANGED_ON_DISK", timeout=20)
    v.check("a change of the file on disk shows in the open view", ok, got[-80:])
    ui.click(file_row(ui, "deploy/api/rollout.log").locator(".main"), tries=3)
    ok, got = ui.waits(right.locator("#body"), "pod 3 ImagePullBackOff", timeout=20)
    v.check("a click on another file opens THAT file (rollout.log)", ok and ui.texts(right.locator("#name")) == ["rollout.log"],
            f"{ui.texts(right.locator('#name'))} {got[-80:]}")
    v.check("a click on an agent after a file opens the agent", ui.open("tasks") and ui.waits(right.locator("#k-tasks"))[0],
            ui.right_now())
