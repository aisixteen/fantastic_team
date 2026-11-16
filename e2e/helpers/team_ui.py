"""The team page, used like a person uses it (#351): what the functional UI tests share
(`ui/main_opens_each_agent`, `ui/tasks_view_counts_and_detail`, `ui/tool_views_real_data`,
`ui/file_row_opens_file`, `ui/phone_one_panel`, `ui/chat_send_and_answer`, `ui/agent_states_in_tree`).
They check function (the right agent opens, with its real data; a message arrives), never the look.

The fleet is the layer `fleets/team_ui` (record files): a team under the product's ids.
`Lead` holds `cluster ops` (with `k-rollout` and `k-scale`) and `marketing` (with `m-ads`), the task
registry `tasks`, the connector `github` and the bundles registry `bundles`; `cadencer` and `dispatcher`
sit under the root, named `launch-week`. The page's leg `team_ws` holds, per agent, the calls of that
agent's view, declared in the records.

What a record cannot hold is made by the team's own verbs before the browser opens: the tasks (`TASKS`,
the registry's `add` / `assign` / `reassign` / `status` / `report`, sent as the operator) and the open
questions (two scripted agents call `user` `inquiry`). Every check compares the page with these
constants, or with what the operator reads back from the same agent.

`Ui` is the browser: Playwright's Chrome on the team page; the left cell (the base sidebar view) and the
right cell (the opened agent's view) are sandboxed srcdoc iframes read through frame locators.
"""

from __future__ import annotations

import os
import re
import time

from . import app, stock as S

PAGE = "/team_page/file/index.html"
ROOT_NAME = "launch-week"
NAMES = {"lead": "Lead", "ops": "cluster ops", "k_rollout": "k-rollout", "k_scale": "k-scale",
         "mkt": "marketing", "m_ads": "m-ads"}
PARENT = {"ops": "lead", "mkt": "lead", "k_rollout": "ops", "k_scale": "ops", "m_ads": "mkt"}

# the open questions
TASK_TEXT = "Roll out api v2.4 to prod-eu, one pod at a time. Stop and ask if a pod is not ready in 5 minutes."
FINDING = "Pod 3 cannot pull the image. I stopped at 2 of 3 and left v2.3 serving on the third pod."
QUESTION = "Pod 3 can't pull api:2.4.0 in eu. What should I do?"
OPTIONS = ["Copy the image to eu, then retry", "Roll back to v2.3", "Leave it at 2 of 3"]
BUDGET = "Budget is over the cap: raise it or pause?"

# a file of the project folder
FILE_PATH = "deploy/api/api.yaml"
FILE_LINES = ["apiVersion: apps/v1", "kind: Deployment", "metadata:", "  name: api", "  namespace: prod-eu",
              "spec:", "  replicas: 3", "  template:", "    spec:", "      containers:", "        - name: api",
              "          image: images.eu/api:2.4.0"]
OTHER_FILES = {"deploy/api/hpa.yaml": "minReplicas: 4\nmaxReplicas: 6\n",
               "deploy/api/rollout.log": "pod 1 ready\npod 2 ready\npod 3 ImagePullBackOff\n"}

# the tasks: (id, holder, state). Every one of the six registry states is there, failed and paused too.
R = "acme/widgets#"
HELD = R + "691"  # the task with a history, versions and a last report
BARE = R + "699"  # held by nobody, nothing reported
TASKS = [
    (R + "214", "m_ads", "blocked"),
    (R + "205", "k_scale", "failed"),
    (HELD, "k_rollout", "running"),
    (R + "616", "ops", "running"),
    (R + "219", {"subtree": "mkt"}, "running"),
    (R + "312", {"subtree": "ops"}, "paused"),
    (BARE, None, "waiting"),
    (R + "700", None, "waiting"),
    (R + "601", "lead", "done"), (R + "602", "lead", "done"), (R + "603", "lead", "done"),
    (R + "604", "lead", "done"), (R + "605", "lead", "done"),
]
VERSION = {"repo": "acme/widgets", "branch": "fix/691-cache-marks", "commit": "9f3c2ab"}
REPORT = "replay test added; cache.rs tests pass, waiting on lint"
STATES = ["waiting", "running", "blocked", "done", "failed", "paused"]
COUNT = {s: sum(1 for t in TASKS if t[2] == s) for s in STATES}
TOTAL = len(TASKS)
AGENT_HOLDERS = len({t[1] for t in TASKS if isinstance(t[1], str)})
SUBTREE_HOLDERS = len({t[1]["subtree"] for t in TASKS if isinstance(t[1], dict)})
DONE_SHOWN = 3  # the registry view lists three done tasks and folds the rest behind one button


def holder_text(h) -> str:
    return h if isinstance(h, str) else "subtree " + h["subtree"] if isinstance(h, dict) else "nobody"


def op(team, target: str, payload: dict, timeout: float = 60.0) -> dict:
    """One call as the operator; the reply's `result` ({} when there is none)."""
    r = team.send(target, payload, timeout=timeout).get("result")
    return r if isinstance(r, dict) else {}


def lay_files(ctx) -> None:
    """The project's files, written before the first start (the `files` agent's root is the run folder)."""
    files = {FILE_PATH: "\n".join(FILE_LINES) + "\n", **OTHER_FILES}
    for path, text in files.items():
        full = os.path.join(ctx.run_dir, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as f:
            f.write(text)


def start(ctx, v, cards: bool = True):
    """Start the team on its fleet with the page served (`webui --port N` from the first start: no
    restart, so no agent rebuilds its context); the repo's bundle cards are laid, so the registry lists them. Returns the port, or None when the page is not served."""
    port = app.free_port()
    ctx.vars["web_port"] = port
    lay_files(ctx)
    S.boot(ctx, extra_args=["webui", "--port", port], cards=True if cards is None else cards)
    served = app.wait_http(f"http://127.0.0.1:{port}{PAGE}", 30)
    v.check("the page is served on --port", served)
    return port if served else None


def report_as_holder(team, tid: str, state: str, errors: list) -> None:
    """`k_rollout`'s last report on `tid`. The registry takes a report only from its caller or the caller's
    descendants (the operator's `send` is `mcp`, which k_rollout is not under), so the entry the registry
    holds is read back with `get` and written with its `report` into the registry's own store, the row
    the registry itself appends on a report."""
    got = op(team, "tasks", {"type": "get", "id": tid})
    entry = {k: v for k, v in got.items() if k != "children"}
    if entry.get("id") != tid:
        errors.append(f"get {tid}: {str(got)[:160]}")
        return
    entry["report"] = {"from": "k_rollout", "state": state, "result": REPORT, "at": int(time.time()), "usage": None}
    r = team.send("memory", {"type": "append", "rows": [{"kind": "task", "task_id": tid, "content": entry}]})
    if r.get("status") != "completed" or (r.get("result") or {}).get("error"):
        errors.append(f"append report {tid}: {str(r)[:160]}")


def seed_tasks(ctx, v) -> bool:
    """The registry's tasks, made with its own verbs as the operator. `HELD` gets a history (nobody ->
    ops -> k_rollout), a version and a last report. Checked: the registry lists exactly `TASKS`."""
    team, errors = ctx.team, []

    def call(payload):
        r = team.send("tasks", payload)
        if r.get("status") != "completed" or (r.get("result") or {}).get("error"):
            errors.append(f"{payload.get('type')} {payload.get('id')}: {str(r)[:160]}")

    for tid, holder, state in TASKS:
        add = {"type": "add", "id": tid, "source": "gh:" + tid}
        if tid == HELD:
            call(add)
            call({"type": "assign", "id": tid, "holder": "ops"})
            call({"type": "reassign", "id": tid, "holder": "k_rollout"})
            call({"type": "status", "id": tid, "status": state, "versions": [VERSION]})
            report_as_holder(team, tid, state, errors)
            continue
        if holder is not None:
            add["holder"] = holder
        call({**add, "status": state})
    got = {t.get("id"): (t.get("holder"), t.get("status")) for t in op(team, "tasks", {"type": "list"}).get("tasks", [])}
    want = {tid: (holder, state) for tid, holder, state in TASKS}
    return v.check(f"the registry holds the {TOTAL} seeded tasks, each with its holder and state",
                   got == want and not errors,
                   "; ".join(errors[:3]) or str({k: got.get(k) for k in want if got.get(k) != want[k]})[:300])


def inquiries(team) -> list[dict]:
    return op(team, "user", {"type": "inquiries"}).get("inquiries") or []


def seed_inquiries(ctx, v, via_ops: bool = True, wait_s: float = 90.0) -> bool:
    """Two agents ask the person, each with its own scripted turn (the test's `script.json`, lanes
    `ops`, `krollout`, `mads`): `k_rollout` gets its task from `cluster ops` (or, without `via_ops`,
    from the operator) and asks `QUESTION` with `OPTIONS`; `m_ads` asks `BUDGET`. Waits until `user`
    lists both, `k_rollout`'s first."""
    team = ctx.team
    if via_ops:
        team.send("ops", {"type": "send", "text": "[lane:ops] Hand k-rollout its rollout task."})
    else:
        team.send("k_rollout", {"type": "send", "text": "[lane:krollout] " + TASK_TEXT})
    end, got = time.time() + wait_s, []
    while time.time() < end and "k_rollout" not in [q.get("asker") for q in got]:
        time.sleep(0.5)
        got = inquiries(team)
    team.send("m_ads", {"type": "send", "text": "[lane:mads] Keep the search ads under the daily cap."})
    while time.time() < end and [q.get("asker") for q in got] != ["k_rollout", "m_ads"]:
        time.sleep(0.5)
        got = inquiries(team)
    texts = {q.get("asker"): q.get("text") for q in got}
    return v.check("two agents asked the person: k_rollout its question with options, then m_ads",
                   texts == {"k_rollout": QUESTION, "m_ads": BUDGET}, str(got)[:300])


def playwright(v):
    """Playwright's entry, or None with a failed check (a missing browser is a failure, never a skip)."""
    try:
        from playwright.sync_api import sync_playwright
        return sync_playwright
    except ImportError:
        v.check("Playwright is installed (python3 -m pip install --user playwright==1.55.0 && "
                "Google Chrome installed)", False, "import playwright failed")
        return None


RIGHT_NOW = """() => {
  const r = document.getElementById('body-right');
  if (!r) return 'no right cell';
  if (r.querySelector('iframe')) return 'a view (iframe) for ' + r.getAttribute('data-open');
  const c = r.querySelector('.card');
  if (c) return 'the name card: ' + c.innerText.replace(/\\s+/g, ' ');
  const o = r.querySelector('.opened');
  return o ? 'only the bare id "' + o.textContent + '" (a placeholder, no view)' : 'an empty cell';
}"""


class Ui:
    """The team page in Chrome. `side` is the left cell's document, `right` the opened view's."""

    def __init__(self, sync_playwright, port: int, width: int = 1440, height: int = 900) -> None:
        self._enter, self.port, self.size = sync_playwright, port, {"width": width, "height": height}
        self.errors: list[str] = []

    def __enter__(self) -> "Ui":
        self._pw = self._enter().start()
        self.browser = self._pw.chromium.launch(channel="chrome")
        self.page = self.browser.new_page(viewport=self.size)
        self.page.on("console", lambda m: self.errors.append(m.text) if m.type == "error" else None)
        self.page.on("pageerror", lambda e: self.errors.append(str(e)))
        self.page.goto(f"http://127.0.0.1:{self.port}{PAGE}")
        self.side = self.page.frame_locator("#body-left iframe")
        self.right = self.page.frame_locator("#body-right iframe")
        return self

    def __exit__(self, *exc) -> None:
        try:
            self.browser.close()
        finally:
            self._pw.stop()

    # ---- reading -------------------------------------------------------------------------------

    def waits(self, loc, want=None, timeout: float = 12.0, exact: bool = False) -> tuple[bool, str]:
        """Wait until `loc`'s first element is there (and its text is / holds `want`, a string or a
        predicate). Returns (ok, the last text seen). Never raises."""
        end, last = time.time() + timeout, "(nothing there)"
        while True:
            try:
                if loc.count():
                    last = re.sub(r"\s+", " ", loc.first.text_content(timeout=1000) or "").strip()
                    if want is None or (want(last) if callable(want) else last == want if exact else want in last):
                        return True, last
            except Exception as e:  # the cell is being redrawn: read again
                last = f"({type(e).__name__})"
            if time.time() >= end:
                return False, last
            time.sleep(0.25)

    def texts(self, loc) -> list[str]:
        """The text of every element of `loc`, spaces collapsed; [] when the document is not there."""
        try:
            return [re.sub(r"\s+", " ", t or "").strip()
                    for t in loc.evaluate_all("els => els.map(e => e.textContent)")]
        except Exception:
            return []

    def settle(self, read, want, timeout: float = 12.0):
        """Poll `read()` until it equals `want` (or `want(value)` is true). Returns (ok, last value)."""
        end = time.time() + timeout
        while True:
            try:
                got = read()
            except Exception as e:
                got = f"({type(e).__name__})"
            if want(got) if callable(want) else got == want:
                return True, got
            if time.time() >= end:
                return False, got
            time.sleep(0.25)

    def right_now(self) -> str:
        """What the right cell holds, in words: for a failed check's detail."""
        try:
            return self.page.evaluate(RIGHT_NOW)
        except Exception as e:
            return f"({type(e).__name__})"

    def attr(self, loc, name: str) -> str:
        try:
            return loc.first.get_attribute(name, timeout=3000) or ""
        except Exception:
            return ""

    # ---- the tree ------------------------------------------------------------------------------

    def tree_ready(self, v) -> bool:
        ok, _ = self.waits(self.side.locator("#tree [role=treeitem]"), timeout=30)
        return v.check("the left cell draws the tree", ok)

    def item(self, id: str):
        """The row of agent `id`, in the tree or in the pinned tools under it."""
        return self.side.locator(f'[role=treeitem][data-id="{id}"]')

    def reveal(self, id: str) -> bool:
        """Make `id`'s row visible the way a person does: unfold the branches, open the tools group."""
        for _ in range(8):
            if self.item(id).count():
                return True
            try:
                folded = self.side.locator('#tree [aria-label^="Unfold "]')
                if folded.count():
                    folded.first.click(timeout=3000)
                    continue
                tools = self.side.locator('#pins button.tg[aria-expanded="false"]')
                if tools.count():
                    tools.first.click(timeout=3000)
                    continue
            except Exception:
                pass
            time.sleep(0.5)
        return self.item(id).count() > 0

    def click(self, loc, tries: int = 6) -> bool:
        """Click; the views redraw while they settle, so a click may land on a replaced node: again."""
        for _ in range(tries):
            try:
                loc.first.click(timeout=4000)
                return True
            except Exception:
                time.sleep(0.3)
        return False

    def open(self, id: str) -> bool:
        """Click `id`'s row; true once the page marks it as the opened agent."""
        if not self.reveal(id):
            return False
        for _ in range(6):
            try:
                self.item(id).locator(".main").first.click(timeout=4000)
                self.page.wait_for_selector(f'#body-right[data-open="{id}"]', timeout=4000)
                return True
            except Exception:
                pass
        return False

    def no_errors(self, v) -> bool:
        return v.check("no console error", not self.errors, str(self.errors[:3]))


def stop(ctx, v) -> None:
    code = ctx.team.stop()
    v.check("the team stopped on SIGTERM and drained", code in (0, -15), f"exit {code}")
