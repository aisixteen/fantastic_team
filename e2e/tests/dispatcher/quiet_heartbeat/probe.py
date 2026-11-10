"""Quiet mode: a lead with an idle member and work queued gets a short heartbeat every third quiet beat."""

from helpers import run_flow, stock as S
from helpers.verdict import Verdict

HEARTBEAT = "heartbeat: 2 idle, 1 queued"


def run(ctx):
    v = Verdict("dispatcher/quiet_heartbeat")
    team = ctx.boot_team(puller=False, board=True, board_store="queue", groups={}, limits={"total": 5},
                         roles_tiers={"strong": S.lead_tier(ctx), "small": S.inert_tier(ctx),
                                      "issue_keeper": S.inert_tier(ctx)})
    made = S.spawn_project_lead(ctx, team)
    pl = made.get("id")
    v.check("the registry spawned the project_lead", bool(pl), str(made))
    if not pl:
        team.stop()
        return v.finish()
    played = run_flow.hand_over(ctx, "Start two workers.", lead=pl, upto=3)
    v.check("the scripted lead spawned its two workers", played, f"served {ctx.model.served}")
    workers = [w["id"] for w in team.send("roles", {"type": "fleet"})["result"].get("roles", {}).get("worker", [])]
    v.check("the registry's fleet lists two workers", len(workers) == 2, str(workers))
    added = team.send("tasks", {"type": "add", "id": "queued-1", "source": "gh:acme/widgets#1"})
    v.check("one task waits in the task registry", added.get("status") == "completed"
            and added["result"].get("status") == "waiting", str(added))
    v.check("no heartbeat was sent before the cadence was set",
            not any("heartbeat:" in b for b in ctx.model.raw_bodies + ctx.model.notice_bodies))

    r = team.send("cadencer", {"type": "set", "cadence": 1})
    v.check("the operator sets the cadence to 1s", r.get("status") == "completed", str(r))
    first = run_flow.wait_model_body(ctx, f"{workers[0]}: idle" if workers else "idle", 30, notices=True)
    v.check("the first beat sent a regular digest (both workers idle)",
            first is not None and all(f"{w}: idle" in first for w in workers))
    v.check("the regular digest is not a heartbeat",
            not any("heartbeat:" in b for b in ctx.model.raw_bodies + ctx.model.notice_bodies))

    beat = run_flow.wait_model_body(ctx, HEARTBEAT, 30, notices=True)
    v.check("a heartbeat with the idle and queued counts reached the lead", beat is not None)
    st = team.send("cadencer", {"type": "status"}).get("result", {})
    v.check("it came only after three quiet beats: four ticks at least",
            st.get("ticks", 0) >= 4, str(st))
    v.check("two digests so far: the regular one and the heartbeat",
            st.get("digests_sent", 0) >= 2, str(st))
    v.check("no delivery failed", st.get("delivery_errors") == 0, str(st))
    team.stop()
    return v.finish()
