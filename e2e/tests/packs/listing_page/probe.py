"""The listing page: static, escaped html built from the signed index and the installed packs."""

import re

from helpers import packs_pub as P
from helpers import run_flow
from helpers.verdict import Verdict


def run(ctx):
    v = Verdict("packs/listing_page")
    ctx.boot_packs({"E2E_MODEL_KEY": run_flow.fake_keys(ctx)["model_key"]})
    t = ctx.team
    page = lambda: P.listing(t, "page")

    # an untrusted signer lists nothing
    P.make_worker(t, "demo", "1.0.0", ["@model", "@connector:github"])
    P.pack(t, "workers/demo", "packs/demo-1.0.0")
    P.write_seed(t)
    pub = P.publish(t, "packs/demo-1.0.0", "https://registry.example.invalid/demo-1.0.0.json",
                    ["read", "write"])
    r = page()
    html = r.get("html", "")
    v.check("an untrusted signer's pack is not on the page: 'No packs listed.'",
            "No packs listed." in html and "demo" not in html, html[:300])

    # trusted and installed
    P.trust(t, pub["published"]["signer_key"])
    inst = P.result(t.send("pack", {"type": "install", "pack": "packs/demo-1.0.0", "into": "templates"}))
    v.check("demo installs", "error" not in inst, str(inst))
    html = page().get("html", "")
    v.check("the page names demo and its version", "<h2>demo <small>1.0.0</small>" in html, html[:400])
    for port in ("@model", "@connector:github"):
        v.check(f"the port {port} is a list item", f"<li>{port}</li>" in html)
    for g in ("read", "write"):
        v.check(f"the grant {g} is a list item", f"<li>{g}</li>" in html)
    v.check("the installed badge is shown", 'class="badge installed"' in html)
    v.check("the price is stated as not stated", "<b>Price:</b> not stated" in html)
    v.check("the evals count is stated as not stated", "<b>Evals:</b> not stated" in html)
    v.check("a fresh pack is not marked revoked", 'class="badge revoked"' not in html)

    # markup in a port string is escaped
    P.make_worker(t, "plain", "1.0.0", ["@connector:<b>x</b>"])
    rp = P.pack(t, "workers/plain", "packs/plain-1.0.0")
    if "error" in rp:
        v.check("pack accepts a port string with markup (else the escaping is a unit item)", False, str(rp))
    else:
        P.publish(t, "packs/plain-1.0.0", "https://registry.example.invalid/plain-1.0.0.json", ["read"])
        html = page().get("html", "")
        v.check("the markup in a port is escaped: &lt;b&gt; is there, a live <b>x is not",
                "&lt;b&gt;x&lt;/b&gt;" in html and "<b>x" not in html)

    # takedown shows the revoked badge
    P.takedown(t, "demo", "1.0.0")
    html = page().get("html", "")
    v.check("after takedown the demo section carries the revoked badge",
            re.search(r'<h2>demo .*class="badge revoked"', html) is not None)

    # the reply is html only, and static
    r = page()
    v.check("the reply carries only html", list(r) == ["html"], str(list(r)))
    v.check("the page has no script", "<script" not in r.get("html", "").lower())
    t.stop()
    return v.finish()
