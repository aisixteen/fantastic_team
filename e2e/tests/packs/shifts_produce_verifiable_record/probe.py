"""Shifts produce a signed measured record that verifies."""

import os

from helpers import packs_evals as P
from helpers import run_flow
from helpers.verdict import Verdict


def set_state(team, number, text):
    """The queue state file of issue `number`, where the puller keeps it (queue_files' root)."""
    d = os.path.join(team.dir, "work", "tasks", "queue")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, f"acme-widgets-{number}.state"), "w") as f:
        f.write(text)
    return True


def publish(team, pack, key, measured):
    return P.result(team.send("pack_listing", {
        "type": "publish", "pack": pack, "url": "https://example.invalid/" + os.path.basename(pack),
        "signing_key": key, "measured": measured, "opt_in": True}))


def run(ctx):
    v = Verdict("packs/shifts_produce_verifiable_record")
    ctx.forge.extra_issues = 1  # issues 1 and 2
    run_flow.boot(ctx)
    P.wire_packs(ctx.team)
    # wire() replaces the github connector's callable_by; the board (a sibling of the lead) needs its entry back
    P.grant(ctx.team, "github", {"board": "*"})
    # The planned board keeps its state in the `queue` (a ram_store, which has no files). Point
    # it at the file store the puller writes the state files to, as the operator may.
    P.grant(ctx.team, "queue_files", {"board": "*"})
    up = ctx.team.send("kernel", {"type": "update_agent", "id": "board", "store": "queue_files",
                                  "can_call": {"github": "*", "queue": "*", "queue_files": "*"}})
    if up.get("status") != "completed":
        raise RuntimeError(f"cannot point the board at queue_files: {up}")
    team = ctx.team
    key = P.write_signing_seed(team.dir)
    packs = {n: P.make_pack(team, n, {"a.yaml": P.PASS_EVAL}) for n in ("p", "q", "r")}

    # the lead claims both tasks; one ends done, the other failed
    pulled = P.result(team.send("board", {"type": "pull"}))
    v.check("the board cached both issues", pulled.get("tasks") == 2 and not pulled.get("errors"), str(pulled))
    ok = set_state(team, 1, "in_work lead\n") and set_state(team, 2, "in_work lead\n")
    v.check("the claim state files were written", ok)
    team.send("board", {"type": "pull"})
    ok = set_state(team, 1, "done acme/widgets#7\n") and set_state(team, 2, "failed\n")
    v.check("the outcome state files were written", ok)
    pulled = P.result(team.send("board", {"type": "pull"}))
    tasks = P.result(team.send("board", {"type": "board"})).get("tasks") or {}
    v.check("the board holds one done and one failed task",
            len(tasks.get("done") or []) == 1 and len(tasks.get("failed") or []) == 1, str(tasks)[:500])

    # without from_shifts the record counts the eval run
    r = P.result(team.send("pack", {"type": "eval", "pack": packs["p"], "worker": "lead",
                                     "signing_key": key, "timeout_s": 60}))
    m = r.get("measured") or {}
    v.check("without from_shifts the record counts the eval run (1 task)",
            m.get("tasks") == 1 and m.get("success_rate") == 1.0, str(m))

    # with from_shifts it counts the shifts
    r = P.result(team.send("pack", {"type": "eval", "pack": packs["q"], "worker": "lead",
                                     "signing_key": key, "from_shifts": True, "timeout_s": 60}))
    m = r.get("measured") or {}
    v.check("from_shifts counts the 2 shifts, 1 success, rate 0.5",
            m.get("tasks") == 2 and m.get("successes") == 1 and m.get("success_rate") == 0.5, str(m))
    v.check("the record is signed", m.get("signed") is True and bool(m.get("signature")), str(m))

    # it verifies: publish accepts it
    pub = publish(team, packs["q"], key, m)
    v.check("publish accepts the record (the signature verifies)", pub.get("measured_published") is True, str(pub)[:400])

    # a tampered field does not verify
    r2 = P.result(team.send("pack", {"type": "eval", "pack": packs["r"], "worker": "lead",
                                      "signing_key": key, "from_shifts": True, "timeout_s": 60}))
    bad = dict(r2.get("measured") or {}, success_rate=1.0)
    pub = publish(team, packs["r"], key, bad)
    v.check("a tampered record is not published, the reason says it does not verify",
            pub.get("measured_published") is False and "does not verify" in pub.get("measured_reason", ""),
            str(pub)[:400])

    code = team.stop()
    v.check("the team stopped on SIGTERM", code in (0, -15), f"exit {code}")
    return v.finish()
