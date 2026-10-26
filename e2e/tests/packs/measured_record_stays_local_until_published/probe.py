"""The measured record never leaves the machine without the owner's publish (opt_in)."""

import os

from helpers import packs as P
from helpers import run_flow
from helpers.verdict import Verdict

SKIP_DIRS = {"secrets", ".fantastic", "home"}


def files_holding(run_dir, needle):
    """Run-dir files (not the keys, the team's own state or its log) whose text holds `needle`."""
    hits = []
    for base, dirs, names in os.walk(run_dir):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS and not (base == run_dir and d in ("forge", "clone"))]
        for n in names:
            if n == "team.log":
                continue
            path = os.path.join(base, n)
            try:
                if needle in open(path, errors="replace").read():
                    hits.append(os.path.relpath(path, run_dir))
            except OSError:
                pass
    return hits


def measure(team, pack, key):
    r = P.result(team.send("pack", {"type": "eval", "pack": pack, "worker": "lead", "signing_key": key,
                                     "timeout_s": 60}))
    return r.get("measured") or {}, r


def publish(team, pack, key, measured, opt_in):
    payload = {"type": "publish", "pack": pack, "url": "https://example.invalid/" + os.path.basename(pack),
               "signing_key": key, "measured": measured}
    if opt_in:
        payload["opt_in"] = True
    return P.result(team.send("pack_listing", payload))


def run(ctx):
    v = Verdict("packs/measured_record_stays_local_until_published")
    run_flow.boot(ctx)
    P.wire_packs(ctx.team)
    team = ctx.team
    key = P.write_signing_seed(team.dir)
    p1 = P.make_pack(team, "p1", {"a.yaml": P.PASS_EVAL})
    p2 = P.make_pack(team, "p2", {"a.yaml": P.PASS_EVAL})
    requests_before = len(ctx.forge.requests)

    # measured and signed on this machine
    m1, _ = measure(team, p1, key)
    v.check("the eval reply carries a signed measured record",
            m1.get("signed") is True and bool(m1.get("signature")) and m1.get("tasks") == 1
            and m1.get("success_rate") == 1.0, str(m1))
    sig = m1.get("signature", "")

    # nothing left: no file, no index, no listing, no request to the forge
    v.check("the record is in no file of the run dir", files_holding(team.dir, sig) == [] if sig else False,
            str(files_holding(team.dir, sig)))
    v.check("there is no index file", not os.path.exists(os.path.join(team.dir, "packs", "index.json")))
    listed = P.result(team.send("pack_listing", {"type": "list"}))
    v.check("the listing holds no pack", (listed.get("packs") or []) == [], str(listed)[:300])
    v.check("no request left for the forge", len(ctx.forge.requests) == requests_before,
            str(len(ctx.forge.requests) - requests_before))

    # a publish without opt_in publishes the pack but not the record
    r = publish(team, p1, key, m1, opt_in=False)
    v.check("publish without opt_in does not publish the record",
            r.get("measured_published") is False and r.get("measured_reason") == "opt_in is not set"
            and "published" in r, str(r)[:400])
    signer = (r.get("published") or {}).get("signer_key")
    up = team.send("kernel", {"type": "update_agent", "id": "pack_listing", "trusted_keys": [signer]})
    v.check("the listing trusts the signer", up.get("status") == "completed", str(up))
    shown = P.result(team.send("pack_listing", {"type": "show", "name": "p1"}))
    v.check("show has the pack but no measured record", shown.get("name") == "p1" and "measured" not in shown,
            str(shown)[:400])
    v.check("the record is still not in the index", files_holding(team.dir, sig) == [],
            str(files_holding(team.dir, sig)))

    # with opt_in the owner publishes it, beside the entry
    m2, _ = measure(team, p2, key)
    r = publish(team, p2, key, m2, opt_in=True)
    v.check("publish with opt_in publishes the record", r.get("measured_published") is True, str(r)[:400])
    shown = P.result(team.send("pack_listing", {"type": "show", "name": "p2"}))
    got = shown.get("measured") or {}
    v.check("show returns the same measured record",
            got.get("tasks") == m2.get("tasks") == 1 and got.get("success_rate") == m2.get("success_rate") == 1.0,
            str(shown)[:500])
    v.check("the index now holds the signature (the walk can see it)",
            files_holding(team.dir, m2.get("signature", "?")) == ["packs/index.json"],
            str(files_holding(team.dir, m2.get("signature", "?"))))

    code = team.stop()
    v.check("the team stopped on SIGTERM", code in (0, -15), f"exit {code}")
    return v.finish()
