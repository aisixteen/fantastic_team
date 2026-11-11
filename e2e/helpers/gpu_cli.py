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
from . import cli, folds
from .cli import EXEC

DONE_CALL = '<call name="done"'
POLL_S = 5.0
SETTLE_S = 3.0
DONE_HOLD_S = 20.0  # a `done` must stay the lead's last row this long
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
        self.task = ""  # the start of the task handed over: a `done` counts only once the lead has it
        self.task_row = 0  # the row the task arrived in, once found

    def refresh(self) -> None:
        """Read the whole history again: its last page has no `next`, so no cursor survives a read."""
        self.rows = cli.history_rows(self.team, self.id)

    def mark(self) -> None:
        """A new task: a `done` before now does not count."""
        self.refresh()
        self.since = (self.rows[-1].get("id") or 0) if self.rows else 0

    def last_answer(self) -> dict | None:
        """The lead's latest assistant row (a notice may land after it)."""
        mine = [r for r in self.rows if "assistant" in (r.get("kind"), r.get("role"))]
        return mine[-1] if mine else None

    def got_task(self) -> bool:
        """Has the task arrived? A lead may answer `done` to a notice that raced the task (the
        cadencer's boot notice), and that `done` is not the end of the task."""
        if not self.task or self.task_row:
            return True
        hit = self.team.send(self.id, {"type": "grep", "query": self.task, "limit": 50},
                             timeout=60).get("result") or {}
        ids = [m.get("id") or 0 for m in hit.get("matches") or [] if (m.get("id") or 0) > self.since]
        self.task_row = min(ids) if ids else 0
        return bool(self.task_row)

    def said_done(self) -> bool:
        try:
            if not self.got_task():
                return False
            self.refresh()
            floor = max(self.since, self.task_row)
            last = self.last_answer()
            if not last or (last.get("id") or 0) <= floor:
                return False
            hit = self.team.send(self.id, {"type": "grep", "query": DONE_CALL, "ids": [last["id"]]},
                                 timeout=60).get("result") or {}
            return bool(hit.get("total"))
        except subprocess.TimeoutExpired:  # the agent is busy; ask again
            return False


def wait_booted(ctx, watch: "Lead", wait_s: float = 60.0) -> None:
    """Wait until the lead has answered the cadencer's boot notice (`[team] The team of … booted up`),
    so the task is the newest message it gets: a lead that reads the notice after the task may answer
    the notice alone ("no task yet") and end its turn."""
    end = time.time() + wait_s
    while time.time() < end:
        try:
            watch.refresh()
        except subprocess.TimeoutExpired:
            time.sleep(POLL_S)
            continue
        booted = [r for r in watch.rows if "booted up" in (r.get("head") or "")]
        if booted and watch.rows[-1].get("id") != booted[-1].get("id") and watch.last_answer() \
                and (watch.last_answer().get("id") or 0) > (booted[-1].get("id") or 0):
            return
        if not booted and time.time() > end - wait_s + 15:  # a team with no cadencer notice
            return
        time.sleep(POLL_S)


def hand_task(ctx, lead: str, text: str) -> threading.Thread:
    """Send the lead its task in the background (a real model takes minutes); the team's reply, or
    the error, lands in `ctx.kicked`. With `E2E_WORKER_MODEL` the task names the model to spawn
    each with (a role names none, the spawner says which): workers that one, sub-leads the lead's."""
    if cli.worker_model():  # leads on the lead's model, workers on the workers'
        text += (f" Spawn every worker with `model` {cli.worker_model()}, and every sub-lead with `model` "
                 f"{ctx.model.model_name}.")
    ctx.kicked = {}
    if getattr(ctx, "watch", None) is not None:
        ctx.watch.task = text.strip().split("\n")[0][:60]
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
            # a `done` counts once it stays the lead's last row: a lead may answer `done` to a notice
            # (the cadencer's boot notice) and take up the task on its next turn
            last = (watch.last_answer() or {}).get("id")
            time.sleep(DONE_HOLD_S)
            watch.refresh()
            if (watch.last_answer() or {}).get("id") != last:
                continue
            time.sleep(SETTLE_S)  # the last replies are routed after the turn
            return "done"
        time.sleep(POLL_S)
    return "timeout"


def play(ctx, v, lead: str, name: str, label: str = "") -> Lead:
    """Hand `lead` the task of scenario `name` and wait for its `done`; checks the wait. `label`
    prefixes the check ("A: ") when a test plays twice."""
    return play_text(ctx, v, lead, TASKS[name], label)


def play_text(ctx, v, lead: str, text: str, label: str = "") -> Lead:
    """Hand `lead` the task `text` and wait for its `done`; checks the wait (`play` of a task in plain words)."""
    watch = ctx.watch = Lead(ctx.team, lead)
    wait_booted(ctx, watch)
    watch.mark()
    started = time.time()
    hand_task(ctx, lead, text)
    how = wait_done(ctx, watch, max(60.0, ctx.test["timeout"] - CHECK_MARGIN_S))
    ctx.elapsed = time.time() - started
    why = {"timeout": "the lead did not answer done before the wait ran out",
           "capped": f"the request cap ended the run ({ctx.model.errored_reason})",
           "exited": "the team exited before the lead answered done"}.get(how, "")
    v.check(f"{label}the lead answered done", how == "done", why + f"; {getattr(ctx, 'kicked', {})}")
    return watch


# ---- checks and the report -------------------------------------------------------------

def lead_on_default(ctx, v, label: str = "", named: bool = False) -> None:
    """The lead runs its provider's default (`E2E_MODEL`): the provider's `models` and the relay's
    first request body. It names no model, unless its record file does (`named`: the precreated
    fleets, which name the lead's model `E2E_MODEL`)."""
    want = ctx.model.model_name
    lead = ctx.team.agents().get(cli.LEAD) or {}
    if named:
        v.check(f"{label}the lead's record names the default model", lead.get("model") == want, str(lead.get("model")))
    else:
        v.check(f"{label}the lead names no model: it runs its provider's default", "model" not in lead, str(lead.get("model")))
    m = ctx.team.send(cli.PROVIDER, {"type": "models"}, timeout=60).get("result") or {}
    v.check(f"{label}the provider's default model is {want}", m.get("default") == want, str(m))
    seen = [r.get("model") for r in ctx.model.requests if isinstance(r, dict) and r.get("model")]
    v.check(f"{label}the lead's first turn ran on the default ({want})", bool(seen) and seen[0] == want, str(seen[:3]))


def workers_on_model(ctx, v, label: str = "") -> None:
    """The workers ran on the model the lead was told to give them (`E2E_WORKER_MODEL`): the
    relay's request bodies."""
    worker = cli.worker_model()
    seen = sorted({r.get("model") for r in ctx.model.requests if isinstance(r, dict) and r.get("model")})
    v.check(f"{label}the workers ran on {worker}, as the task told the lead", bool(worker) and worker in seen, str(seen))


def check_ins_ran(ctx, v, label: str = "") -> None:
    """Each lead given check-ins (`cli.CHECK_INS`) made its own scheduler and was woken by it (a
    `[check-in` row in its conversation), and no schedule is left once the leads are done (the scheduler
    agent may stay)."""
    for lead in getattr(ctx, "check_in_leads", []):
        # the lead's own `schedule` call holds the tag too: only a row it RECEIVED counts
        try:
            watch = Lead(ctx.team, lead)
            watch.refresh()
            got = {r.get("id") for r in watch.rows if r.get("role") == "user"}
            hit = ctx.team.send(lead, {"type": "grep", "query": "[check-in", "limit": 200},
                                timeout=60).get("result") or {}
            ticks = [m for m in hit.get("matches") or [] if m.get("id") in got]
        except subprocess.TimeoutExpired:
            ticks, hit = [], {"error": "busy"}
        v.check(f"{label}{lead} made its own scheduler and a check-in woke it", bool(ticks), str(hit)[:200])
    left = cli.schedules_left(ctx)
    v.check(f"{label}no schedule is left (a scheduler agent may stay)", not left, str(left))
    per = cli.schedulers_per_lead(ctx)
    v.check(f"{label}no more than one scheduler per lead", all(n <= 1 for n in per.values()), str(per))


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
    if watch:
        try:
            watch.refresh()  # the folds are counted from the whole history, as it stands now
        except Exception:  # the report must never fail
            pass
    if watch and watch.rows:
        print("  report: " + folds.report_line(watch.rows, sum(n for k, n in calls.items() if k.endswith(" recall"))))
        mine = [r for r in watch.rows if r.get("kind") == "assistant"]
        first = mine[0].get("head", "")[:100] if mine else ""
        print(f"  report: the lead took {len(mine)} turns; its first: {first!r}")


def precreated_run(ctx, name: str, tree: dict):
    """`gpu_cli/<name>` for the precreated trees: the lead is told the task; the tree after is the tree before."""
    from . import cli_scenarios as S
    from .verdict import Verdict
    v = Verdict(f"gpu_cli/{name}")
    before = S.precreate(ctx, v, tree)
    watch = play(ctx, v, "lead", name)
    check_ins_ran(ctx, v)
    lead_on_default(ctx, v, named=True)
    workers_on_model(ctx, v)
    S.precreated_end(ctx, v, before)
    check_key_not_on_disk(ctx, v)
    report(ctx, watch)
    return v.finish()
