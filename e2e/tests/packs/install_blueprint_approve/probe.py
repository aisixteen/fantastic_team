"""install with parent_id: a preview first (nothing created), then approve builds the agent."""

import json
import os

from helpers import packs as P
from helpers.verdict import Verdict

BIND = {"connector:github": "github"}


def run(ctx):
    v = Verdict("packs/install_blueprint_approve")
    a = None
    try:
        a = P.boot_plain(ctx, "a")
        P.write_source_worker(a.dir)
        r = P.send_pack(a, {"type": "pack", "worker": "w", "out": "out"})
        v.check("pack replies without an error", "error" not in r, str(r))
        base = {"type": "install", "pack": "out", "into": "templates", "parent_id": "lead", "bindings": BIND}

        r = P.send_pack(a, base)
        v.check("the preview has no error", "error" not in r, str(r))
        v.check("the preview asks for approval",
                (r.get("approval") or {}).get("approval_required") is True, str(r.get("approval")))
        v.check("the preview is still a template", r.get("installed_as") == "template", str(r.get("installed_as")))
        v.check("the preview creates no ids", "ids" not in r)
        v.check("the preview creates no agent", "scribe_1" not in P.agents(a))
        bj = os.path.join(a.dir, "templates", "scribe-1.0.0", "bindings.json")
        got = json.load(open(bj)) if os.path.isfile(bj) else None
        v.check("bindings.json keeps the binding", got == BIND, str(got))

        r = P.send_pack(a, dict(base, approve=True))
        v.check("approve has no error", "error" not in r, str(r))
        v.check("approve installs as a worker", r.get("installed_as") == "worker", str(r.get("installed_as")))
        v.check("approve returns the new id scribe_1", (r.get("ids") or {}).get("scribe") == "scribe_1", str(r.get("ids")))
        v.check("scribe_1 is live", "scribe_1" in P.agents(a))
        v.check("the port is bound to github",
                ((r.get("imported") or {}).get("ports") or {}).get("@connector:github") == "github", str(r.get("imported")))
        rec = a.send("scribe_1", {"type": "reflect"})
        text = json.dumps(rec)
        v.check("the agent's can_call names github", '"github"' in text and "@connector:github" not in text, text[:400])
    finally:
        if a:
            a.stop()
    return v.finish()
