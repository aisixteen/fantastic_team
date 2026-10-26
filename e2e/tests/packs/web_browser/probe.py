"""The pack browser panel's contract. There is no browser or display here, so the panel's page
is read as the team persisted it (an agent on the canvas), and the exact verbs and payloads the
page sends are driven by hand: list, show, the install preview then approve, eval, and the
browser view-model with and without opt-in."""

import os

from helpers import packs_pub as P
from helpers import run_flow
from helpers.verdict import Verdict

BLUEPRINT = "agents:\n  - id: notes\n    handler_module: ram_store.tools"
EVAL = "- id: says_ok\n  task: Reply with the word ok.\n  check:\n    contains: ok"
URL = "https://registry.example.invalid/notes-{v}.json"


def ids(t):
    return {a.get("id") for a in P.result(t.send("kernel", {"type": "list_agents"})).get("agents", [])}


def run(ctx):
    v = Verdict("packs/web_browser")
    ctx.boot_packs({"E2E_MODEL_KEY": run_flow.fake_keys(ctx)["model_key"]}, canvas=True)
    t = ctx.team

    # 1. the panel the team serves: an agent on the canvas, with the page and its two grants
    path = os.path.join(ctx.run_dir, ".fantastic", "agents", "canvas_page", "agents", "packs_view",
                        "agent.yaml")
    text = open(path).read() if os.path.exists(path) else ""
    page, _, grants = text.partition("\ncan_call:")
    v.check("the packs_view panel is on the canvas", "id: packs_view" in text and "parent_id: canvas_page" in text)
    v.check("its page sends pack_listing, install and eval",
            "pack_listing" in page and "'install'" in page and "'eval'" in page and "'list'" in page)
    can_call = [l.split(":")[0].strip() for l in grants.split("callable_by:")[0].splitlines()[1:] if l.strip()]
    v.check("it may call the pack and pack_listing agents and nothing else",
            sorted(can_call) == ["$self", "pack", "pack_listing"], str(can_call))
    v.check("its page makes no request of its own (no fetch, no XMLHttpRequest)",
            "fetch(" not in page and "XMLHttpRequest" not in page)

    # publish notes 1.0.0 (a blueprint and an eval) and trust the signer
    P.make_worker(t, "notes", "1.0.0", [], extra_files={"blueprint/blueprint.yaml": BLUEPRINT})
    r = P.pack(t, "workers/notes", "packs/notes-1.0.0")
    v.check("pack notes 1.0.0 (a blueprint)", "error" not in r, str(r))
    P.write_seed(t)
    pub = P.publish(t, "packs/notes-1.0.0", URL.format(v="1.0.0"), ["read"])
    P.trust(t, pub["published"]["signer_key"])

    # 2. list and show, as the page sends them
    lst = P.listing(t, "list", query="")
    v.check("list shows notes", [p["name"] for p in lst.get("packs", [])] == ["notes"], str(lst))
    v.check("list with a query matches case-insensitively",
            [p["name"] for p in P.listing(t, "list", query="NOTE").get("packs", [])] == ["notes"])
    v.check("list with a query that matches nothing is empty",
            P.listing(t, "list", query="zzz").get("packs") == [])
    show = P.listing(t, "show", name="notes")
    v.check("show returns ports, grants, price and an allowed install decision",
            show.get("ports") == [] and show.get("grants") == ["read"] and "price" in show
            and show.get("install_decision") == {"allow": True}, str(show))

    # 3. the install, in the panel's two steps
    before = ids(t)
    payload = {"type": "install", "pack": "packs/notes-1.0.0", "into": "templates",
               "bindings": {}, "parent_id": "lead"}
    prev = P.result(t.send("pack", payload))
    v.check("install without approve answers the permission preview under approval",
            "approval" in prev and "imported" not in prev and "error" not in prev, str(prev))
    v.check("and creates no agent", ids(t) == before)
    done = P.result(t.send("pack", dict(payload, approve=True)))
    new_ids = list((done.get("ids") or {}).values())
    v.check("the same payload with approve true imports the blueprint", bool(done.get("imported"))
            and len(new_ids) == 1 and "error" not in done, str(done))
    v.check("and the new agent exists", bool(new_ids) and set(new_ids) <= ids(t), str(new_ids))

    # 4. the browser view-model: updates stay hidden until opt-in
    P.make_worker(t, "notes", "1.1.0", ["@connector:github"], folder="workers/notes2")
    P.pack(t, "workers/notes2", "packs/notes-1.1.0")
    pub2 = P.publish(t, "packs/notes-1.1.0", URL.format(v="1.1.0"), ["read", "write"])
    off = P.listing(t, "browser", selected="notes", opt_in=False)
    prev_ports = off.get("selected", {}).get("permission_preview")
    v.check("browser: cards, and the selected pack's permission preview is its ports and grants",
            [c["name"] for c in off.get("cards", [])] == ["notes"]
            and prev_ports == {"ports": ["@connector:github"], "grants": ["read", "write"]}, str(off))
    v.check("without opt_in: no updates, and a note says to turn them on",
            off.get("updates") == [] and off.get("note") == "turn on updates to see changes", str(off))
    on = P.listing(t, "browser", selected="notes", opt_in=True)
    ups = on.get("updates", [])
    diff = ups[0].get("permission_diff", {}) if ups else {}
    v.check("with opt_in: the update and its permission diff, and no note",
            len(ups) == 1 and ups[0].get("new_version") == "1.1.0" and ups[0].get("new_hash")
            == pub2["published"]["hash"] and diff.get("added_ports") == ["@connector:github"]
            and diff.get("added_grants") == ["write"] and "note" not in on, str(on))

    # 5. eval, as the page sends it: the lead answers from the scripted model
    served = ctx.model.served
    P.make_worker(t, "checker", "1.0.0", [], extra_files={"evals/one.yaml": EVAL})
    P.pack(t, "workers/checker", "packs/checker-1.0.0")
    ev = P.result(t.send("pack", {"type": "eval", "pack": "packs/checker-1.0.0", "worker": "lead"}))
    v.check("eval runs the pack's eval on the lead: one eval, pass rate 1",
            ev.get("total") == 1 and ev.get("passed") == 1 and ev.get("pass_rate") == 1, str(ev))
    # the lead answers its own reply once more (an absent reply_to is self), a moment later: wait
    # for the count to settle, then read the first request this eval made
    after = run_flow.wait_model_quiet(ctx.model, quiet=2.0)
    first = ctx.model.raw_bodies[served] if len(ctx.model.raw_bodies) > served else ""
    v.check("the scripted model served the eval's request first", after >= served + 1
            and "Reply with the word ok." in first, f"{served} -> {after}")
    t.stop()
    return v.finish()
