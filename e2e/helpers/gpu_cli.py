"""The gpu tier's half of the cli scenarios (issue #237): a real model, no script.

The team is built by `helpers/cli_scenarios.py` (the same calls and checks as the `cli/` test of
the same name). Here are the parts a script does not need: the task in plain words, the wait for
the lead to answer `done`, and the run's report. Nothing asserts the model's own path (which tool
it called first, how many turns it took): `report` prints it in the run's log.
"""

from __future__ import annotations

import subprocess
import threading
import time
from collections import Counter

from . import app, team as team_mod
from .cli import EXEC

DONE_CALL = '<call name="done"'
POLL_S = 5.0
SETTLE_S = 3.0
CHECK_MARGIN_S = 120  # kept from the test's timeout for the checks after the wait

# ---- the task, in plain words ----------------------------------------------------------

BROKEN = (f"The repository your exec agent `{EXEC}` runs in has a failing test: `python3 test_calc.py` "
          "fails, because two functions are wrong (`add` in adder.py, `mul` in muler.py). ")
COMMIT = ("The fix must be a git commit, with a clean work tree afterwards. git has no identity set here, "
          "so commit with `git -c user.name=Worker -c user.email=worker@example.invalid commit -a -m MESSAGE`. ")
END = " When the test passes on the committed tree, end your run with the `done` call."
DELETE = (" Whoever spawns an agent deletes it with `delete_agent` on `kernel` when its work is done, "
          "and never deletes an agent it did not spawn.")
# a spawned agent tells its parent with a plain `send` (team#247): the message starts the parent's turn
TELL = (" A spawned agent may `send` its parent (the agent that spawned it), and that message starts "
        "the parent's next turn.")

TASKS = {
    "single_lead": BROKEN + "Find the bugs yourself, fix them and commit the fix." + " " + COMMIT.strip() + END,
    "lead_spawns_two": (
        BROKEN + "Do not fix it yourself: spawn two workers with the role `worker` (`spawn` on `roles`), "
        "one for adder.py and one for muler.py, each task naming its file. " + COMMIT +
        "Tell each worker to commit its fix this way, and: \"send your parent a message when you are done\"." + TELL
        + DELETE + " Check that the test passes at the end." + END),
    "precreated_1_2": (
        BROKEN + "Two workers already exist, `w1` and `w2`, and can run the exec agent. Do not fix it "
        "yourself: send `w1` a message to fix adder.py and `w2` one to fix muler.py (verb `send`). " + COMMIT +
        "Tell each worker to commit its fix this way. Wait for both answers, then check that the test passes." + END),
    "precreated_tree": (
        BROKEN + "Your two sub-leads already exist, `s1` and `s2`, and each has a worker (`w1` under `s1`, "
        "`w2` under `s2`). Do not fix it yourself: send `s1` a message to have adder.py fixed through its "
        "worker and `s2` one to have muler.py fixed through its worker (verb `send`). " + COMMIT +
        "Tell them to pass on that the fix is committed this way. Wait for both answers, then check that the "
        "test passes." + END),
    "leads_spawn_leads": (
        BROKEN + "Do not fix it yourself: spawn two sub-leads with the role `sublead` (`spawn` on `roles`), "
        "one for adder.py and one for muler.py, each task naming its file. Each sub-lead spawns one worker "
        "for its file. " + COMMIT + "Tell them that the worker commits this way, and tell each one: \"send your parent a "
        "message when you are done\"." + TELL + DELETE +
        " Check that the test passes at the end." + END),
    "headless_and_canvas": (
        "Make a canvas for your notes: call `create_agent` on `kernel` with the id `sketch`, `parent_id` "
        "`lead` (yourself) and `handler_module` `canvas_page.tools`. Make nothing else and change no file. "
        "Then end your run with the `done` call."),
}


# ---- the wait for `done` ---------------------------------------------------------------

class Lead:
    """Watches one lead's conversation. The lead has answered `done` when its last row is an
    assistant row holding the done call (the `history` rows are digests, so the call is found
    with `grep` on that row)."""

    def __init__(self, team, lead: str) -> None:
        self.team, self.id = team, lead
        self.rows: list[dict] = []
        self.since = 0  # `done` counts only in a row whose id is past this one (a lead told twice, #242)

    def refresh(self) -> None:
        """Read the whole history again: its last page has no `next`, so no cursor survives a read."""
        rows, after = [], None
        while True:
            payload = {"type": "history"}
            if after is not None:
                payload["after"] = after
            res = self.team.send(self.id, payload, timeout=60).get("result") or {}
            rows += res.get("rows") or []
            after = res.get("next")
            if not after:
                break
        self.rows = rows

    def mark(self) -> None:
        """A new task: a `done` before now does not count."""
        self.refresh()
        self.since = (self.rows[-1].get("id") or 0) if self.rows else 0

    def said_done(self) -> bool:
        try:
            self.refresh()
            last = self.rows[-1] if self.rows and (self.rows[-1].get("id") or 0) > self.since else None
            if not last or last.get("kind") != "assistant":
                return False
            hit = self.team.send(self.id, {"type": "grep", "query": DONE_CALL, "ids": [last["id"]]},
                                 timeout=60).get("result") or {}
            return bool(hit.get("total"))
        except subprocess.TimeoutExpired:  # the agent is busy; ask again
            return False


def hand_task(ctx, lead: str, text: str) -> threading.Thread:
    """Send the lead its task in the background (a real model takes minutes); the team's reply, or
    the error, lands in `ctx.kicked`. With `E2E_WORKER_MODEL` the task names the model to spawn
    with: a role names none, the spawner says which."""
    if cli.worker_model():
        text += f" Spawn every agent with `model` {cli.worker_model()}."
    ctx.kicked = {}
    limit = ctx.test["timeout"]

    def go():
        try:
            ctx.kicked["reply"] = ctx.team.send(lead, {"type": "send", "text": text}, timeout=limit)
        except Exception as e:  # recorded; the wait decides
            ctx.kicked["error"] = str(e)

    t = threading.Thread(target=go, daemon=True)
    t.start()
    return t


def wait_done(ctx, watch: Lead, wait_s: float) -> str:
    """Wait for the lead's `done`. Returns `done`, `timeout`, `capped` (the plug's request cap
    stopped the team) or `exited` (the team went away)."""
    end = time.time() + wait_s
    while time.time() < end:
        if ctx.model.capped:  # an upstream error alone is the model's to retry, not the end of the run
            return "capped"
        if ctx.team.proc and ctx.team.proc.poll() is not None:
            return "exited"
        if watch.said_done():
            time.sleep(SETTLE_S)  # the last replies are routed after the turn
            return "done"
        time.sleep(POLL_S)
    return "timeout"


def play(ctx, v, lead: str, name: str, label: str = "") -> Lead:
    """Hand `lead` the task of scenario `name` and wait for its `done`; checks the wait. `label`
    prefixes the check ("A: ") when a test plays twice."""
    watch = Lead(ctx.team, lead)
    started = time.time()
    hand_task(ctx, lead, TASKS[name])
    how = wait_done(ctx, watch, max(60.0, ctx.test["timeout"] - CHECK_MARGIN_S))
    ctx.elapsed = time.time() - started
    why = {"timeout": "the lead did not answer done before the wait ran out",
           "capped": f"the request cap ended the run ({ctx.model.errored_reason})",
           "exited": "the team exited before the lead answered done"}.get(how, "")
    v.check(f"{label}the lead answered done", how == "done", why + f"; {getattr(ctx, 'kicked', {})}")
    return watch


# ---- checks and the report -------------------------------------------------------------

def check_key_not_on_disk(ctx, v) -> None:
    """The door's key is read from the environment by the credential store and never stored."""
    import os
    if not ctx.model.key_env:
        return
    key = os.environ.get(ctx.model.key_env, "")
    if key:
        v.check("the door's key is on no file of the run (outside secrets/)",
                app.files_holding(ctx.run_dir, key) == [], "a file of the run holds the key")


def report(ctx, watch: Lead | None = None) -> None:
    """Print what the model did: not asserted, only logged."""
    ctx.model.finish(ctx.team)
    totals = ctx.model.totals()
    print(f"  report: {getattr(ctx, 'elapsed', 0):.0f}s, {totals['requests']} requests, "
          f"{totals['input_tokens']} tokens in, {totals['output_tokens']} out (cost.json on stop)")
    if totals.get("agents"):  # the agents' own counts: cache reads and writes, the cost when priced
        a = totals["agents"]
        print(f"  report: agents: {a['requests']} requests, {a['input_tokens']} in, "
              f"{a['cache_write_tokens']} cache write, {a['cache_read_tokens']} cache read, "
              f"{a['output_tokens']} out, cost_usd {'unpriced' if a['cost_usd'] is None else a['cost_usd']}")
    calls = Counter(f"{a} {verb}" for a, verb in team_mod.log_calls(ctx.team.log()))
    print(f"  report: calls {dict(calls.most_common(12))}")
    if watch and watch.rows:
        mine = [r for r in watch.rows if r.get("kind") == "assistant"]
        first = mine[0].get("head", "")[:100] if mine else ""
        print(f"  report: the lead took {len(mine)} turns; its first: {first!r}")
