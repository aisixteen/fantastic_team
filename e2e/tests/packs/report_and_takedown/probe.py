"""Report and takedown. A report needs a registry and a sane reason; a takedown revokes locally,
signed, even when the registry cannot be reached, and the installed pack is warned about."""

import json
import os
import time

from helpers import packs as P
from helpers import run_flow
from helpers.registry import FakeRegistry
from helpers.verdict import Verdict

URL = "https://registry.example.invalid/demo-1.0.0.json"


def settle(reg, want, wait_s=5.0):
    end = time.time() + wait_s
    while reg.handshake_failures < want and time.time() < end:
        time.sleep(0.05)


def run(ctx):
    v = Verdict("packs/report_and_takedown")
    ctx.boot_packs({"E2E_MODEL_KEY": run_flow.fake_keys(ctx)["model_key"]})
    t = ctx.team
    reg = FakeRegistry(os.path.join(ctx.run_dir, "registry")).start()
    try:
        # demo 1.0.0, published, trusted and installed
        P.make_worker(t, "demo", "1.0.0", ["@model"])
        P.pack(t, "workers/demo", "packs/demo-1.0.0")
        P.write_seed(t)
        pub = P.publish(t, "packs/demo-1.0.0", URL, ["read"])
        h = pub["published"]["hash"]
        P.trust(t, pub["published"]["signer_key"])
        inst = P.result(t.send("pack", {"type": "install", "pack": "packs/demo-1.0.0", "into": "templates"}))
        v.check("demo 1.0.0 is installed", "error" not in inst, str(inst))
        report = lambda **kw: P.listing(t, "report", **dict({"name": "demo", "version": "1.0.0",
                                                           "reason": "it does something odd"}, **kw))

        # 1. report
        r = report()
        v.check("report without a registry_url is an error: no registry_url",
                r.get("error") == "no registry_url", str(r))
        P.set_registry(t, reg.url)
        for label, kw, want in [("an empty reason", {"reason": "  "}, "report needs"),
                                ("a reason over 2000 characters", {"reason": "x" * 2001}, "over 2000"),
                                ("a name that is not a safe path segment", {"name": "../x"}, "safe path segment")]:
            r = report(**kw)
            v.check(f"report with {label} is refused: {want}", want in r.get("error", ""), str(r))
        v.check("and none of them reached the registry", reg.quiet(), f"{reg.connections} {reg.requests}")
        r = report()
        settle(reg, 1)
        v.check("a valid report goes to the registry and fails there: report failed",
                r.get("error", "").startswith("report failed") and "reported" not in r, str(r))
        v.check("the registry saw one connection attempt and, by design, no request",
                reg.connections == 1 and reg.requests == [], f"{reg.connections} {reg.requests}")

        # 4. a takedown with a bad key leaves the index alone
        P.set_registry(t, "")
        index_before = P.text_of(t, "packs/index.json")
        for label, key in [("a missing key file", "keys/nope")]:
            r = P.takedown(t, "demo", "1.0.0", key=key)
            v.check(f"takedown with {label}: signing key file could not be read",
                    "signing key file could not be read" in r.get("error", ""), str(r))
        P.put(t, "keys/junk", "this is not a key")
        r = P.takedown(t, "demo", "1.0.0", key="keys/junk")
        v.check("takedown with a key file that is not a key is an error and not revoked",
                "error" in r and "revoked" not in r, str(r))
        v.check("and the index is unchanged", P.text_of(t, "packs/index.json") == index_before)

        # 2. takedown with a registry that cannot be reached
        P.set_registry(t, reg.url)
        before = reg.connections
        td = P.takedown(t, "demo", "1.0.0")
        settle(reg, 2)
        v.check("takedown revokes locally: revoked {name, version}, pushed false",
                td.get("revoked") == {"name": "demo", "version": "1.0.0"} and td.get("pushed") is False, str(td))
        v.check("and says the registry push failed",
                "revoked locally, but the registry push failed" in td.get("error", ""), str(td))
        v.check("the push was tried (a connection, no request)",
                reg.connections > before and reg.requests == [], f"{reg.connections} {reg.requests}")
        P.set_registry(t, "")
        doc = json.loads(P.text_of(t, "packs/index.json") or "{}")
        flags = {(e["name"], e["version"]): e.get("revoked") for e in doc.get("entries", [])}
        v.check("packs/index.json holds revoked true for demo 1.0.0", flags.get(("demo", "1.0.0")) is True, str(flags))
        lst = P.listing(t, "list")
        v.check("the re-signed entry still verifies: list shows it, revoked, not skipped",
                [(p["name"], p["revoked"]) for p in lst.get("packs", [])] == [("demo", True)]
                and not lst.get("skipped"), str(lst))

        # 3. the installed pack is warned about; its hash cannot be fetched
        up = P.listing(t, "updates", opt_in=True)
        v.check("the installed pack gets a warning that names demo",
                any("demo" in w for w in up.get("warnings", [])), str(up))
        fe = P.listing(t, "fetch", name="demo", hash=h, stage="stage/x")
        v.check("fetch of the revoked hash is refused: revoked", "revoked" in fe.get("error", ""), str(fe))
        v.check("the signing seed is in no takedown reply", P.SEED not in json.dumps([td, up, fe]))
    finally:
        reg.stop()
    t.stop()
    return v.finish()
