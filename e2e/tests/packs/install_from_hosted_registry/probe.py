"""fetch pins by name and hash in the signed index and refuses before it touches the network;
a registry whose certificate the binary does not trust gets a connection and nothing else."""

import os

from helpers import packs_pub as P
from helpers import run_flow
from helpers.registry import FakeRegistry
from helpers.verdict import Verdict


def settle(reg, want, wait_s=5.0):
    """Wait until the fake registry has seen `want` failed handshakes (they land on a thread)."""
    import time
    end = time.time() + wait_s
    while reg.handshake_failures < want and time.time() < end:
        time.sleep(0.05)


def run(ctx):
    v = Verdict("packs/install_from_hosted_registry")
    ctx.boot_packs({"E2E_MODEL_KEY": run_flow.fake_keys(ctx)["model_key"]})
    t = ctx.team
    reg = FakeRegistry(os.path.join(ctx.run_dir, "registry")).start()
    try:
        P.make_worker(t, "demo", "1.0.0", ["@model"])
        P.pack(t, "workers/demo", "packs/demo-1.0.0")
        P.write_seed(t)
        pub = P.publish(t, "packs/demo-1.0.0", reg.url + "/demo-1.0.0.json", ["read"])
        good = pub["published"]["hash"]
        P.trust(t, pub["published"]["signer_key"])
        v.check("the signed local index lists demo with the fake registry as its url",
                [p["name"] for p in P.listing(t, "list").get("packs", [])] == ["demo"])

        # 1. a hash that is not in the index: refused before any connection
        bad = "0" * 64
        r = P.listing(t, "fetch", name="demo", hash=bad, stage="stage/a")
        v.check("fetch of a hash the index does not pin is an error", "error" in r, str(r))
        v.check("the registry saw no connection for it", reg.connections == 0, str(reg.connections))
        v.check("nothing was staged", not P.exists(t, "stage/a/pack.yaml"))

        # 2. a revoked entry: refused with revoked, before any connection
        P.takedown(t, "demo", "1.0.0")
        r = P.listing(t, "fetch", name="demo", hash=good, stage="stage/b")
        v.check("fetch of a revoked entry is refused with revoked", "revoked" in r.get("error", ""), str(r))
        v.check("the registry still saw no connection", reg.connections == 0, str(reg.connections))
        v.check("nothing was staged for the revoked entry", not P.exists(t, "stage/b/pack.yaml"))

        # a second, live version to download
        P.make_worker(t, "demo", "1.1.0", ["@model"], folder="workers/demo2")
        P.pack(t, "workers/demo2", "packs/demo-1.1.0")
        pub2 = P.publish(t, "packs/demo-1.1.0", reg.url + "/demo-1.1.0.json", ["read"])
        live = pub2["published"]["hash"]

        # 4. right hash, wrong version: refused naming the version, before any connection
        r = P.listing(t, "fetch", name="demo", hash=live, version="9.9.9", stage="stage/c")
        v.check("fetch with the right hash and a wrong version names the version",
                "1.1.0" in r.get("error", "") and "9.9.9" in r.get("error", ""), str(r))
        v.check("the wrong version made no connection either", reg.connections == 0, str(reg.connections))

        # 3. the right name and hash goes on to the download; the binary distrusts the cert
        r = P.listing(t, "fetch", name="demo", hash=live, stage="stage/d")
        settle(reg, 1)
        v.check("fetch of a live pinned entry goes on to the download and fails: could not reach",
                "could not reach the registry" in r.get("error", ""), str(r))
        v.check("the registry saw a connection and a failed handshake, and no HTTP request",
                reg.connections >= 1 and reg.handshake_failures >= 1 and reg.requests == [],
                f"{reg.connections} {reg.handshake_failures} {reg.requests}")
        v.check("the error echoes no body and nothing was staged",
                "staged" not in r and not P.exists(t, "stage/d/pack.yaml"))

        # 5. http:// is refused
        before = reg.connections
        P.set_registry(t, f"http://127.0.0.1:{reg.port}")
        r = P.listing(t, "list")
        v.check("a registry_url over http is refused: must be https",
                "must be https" in r.get("error", ""), str(r))
        v.check("the registry saw nothing more", reg.connections == before and reg.requests == [])
    finally:
        reg.stop()
    t.stop()
    return v.finish()
