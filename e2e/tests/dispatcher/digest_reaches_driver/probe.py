"""The cadencer's own timer sends the spawned tree's lead a digest: it arrives as a model request."""

from helpers import run_flow, stock as S, team as T
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("dispatcher/digest_reaches_driver")
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

    r = team.send("cadencer", {"type": "set", "cadence": 2})
    v.check("the operator sets the cadence to 2s", r.get("status") == "completed"
            and r["result"].get("cadence") == 2, str(r))
    body = run_flow.wait_model_body(ctx, f"{workers[0]}: idle" if workers else "idle", 30, notices=True)
    v.check("the lead's model got a request holding the digest", body is not None)
    body = body or ""
    v.check("the digest names both spawned workers as idle", all(f"{w}: idle" in body for w in workers))
    v.check("the digest ends with the load line", "load: busy 0 idle 2" in body
            and all(body.index(f"{w}: idle") < body.index("load: busy") for w in workers if f"{w}: idle" in body))
    st = team.send("cadencer", {"type": "status"}).get("result", {})
    v.check("the timer ticked by itself", st.get("ticks", 0) >= 1, str(st))
    v.check("a digest was counted as sent", st.get("digests_sent", 0) >= 1, str(st))
    v.check("no delivery failed", st.get("delivery_errors") == 0, str(st))
    v.check("the digest volume is metered", st.get("digest_bytes_per_hour", 0) > 0, str(st))
    calls = T.log_calls(team.log())
    v.check("the probe never sent a tick", ("cadencer", "tick") not in calls)
    team.stop()
    return v.finish()
