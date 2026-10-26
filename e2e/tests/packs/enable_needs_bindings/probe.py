"""Enabling a pack as a worker is refused while a port is unbound or its agent does not exist."""

import os

from helpers import packs as P
from helpers.verdict import Verdict

PREFIX = "enable refused:"


def run(ctx):
    v = Verdict("packs/enable_needs_bindings")
    a = None
    try:
        a = P.boot_plain(ctx, "a")
        P.write_source_worker(a.dir)
        r = P.send_pack(a, {"type": "pack", "worker": "w", "out": "out"})
        v.check("pack replies without an error", "error" not in r, str(r))
        base = {"type": "install", "pack": "out", "into": "templates", "parent_id": "lead", "approve": True}

        r = P.send_pack(a, base)
        e = r.get("error") or ""
        v.check("unbound: refused with 'enable refused:'", e.startswith(PREFIX), str(r))
        v.check("unbound: names the port connector:github", "connector:github" in e, e)
        v.check("unbound: enable_error is the text after the prefix",
                r.get("enable_error") == e[len(PREFIX):].strip() and bool(e), str(r))
        v.check("unbound: the template is kept", r.get("installed_as") == "template", str(r.get("installed_as")))
        v.check("unbound: pack.yaml is installed",
                os.path.isfile(os.path.join(a.dir, "templates", "scribe-1.0.0", "pack.yaml")))
        v.check("unbound: no ids", "ids" not in r)
        v.check("unbound: no agent", "scribe_1" not in P.agents(a))

        r = P.send_pack(a, dict(base, bindings={"connector:github": "nosuchagent"}))
        e = r.get("error") or ""
        v.check("missing agent: refused with 'enable refused:'", e.startswith(PREFIX), str(r))
        v.check("missing agent: names the port and says it does not exist",
                "connector:github" in e and "does not exist" in e, e)
        v.check("missing agent: the bound id is not echoed", "nosuchagent" not in e, e)
        v.check("missing agent: no agent", "scribe_1" not in P.agents(a))

        r = P.send_pack(a, dict(base, bindings={"connector:github": "github"}))
        v.check("bound to a live agent: no error", "error" not in r, str(r))
        v.check("bound to a live agent: installed as a worker", r.get("installed_as") == "worker", str(r))
        v.check("bound to a live agent: scribe_1 is live", "scribe_1" in P.agents(a))
    finally:
        if a:
            a.stop()
    return v.finish()
