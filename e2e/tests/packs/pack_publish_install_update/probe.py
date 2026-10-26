"""A pack is packed, published (signed), listed, installed, updated (opt-in) and taken down."""

import json

from helpers import packs as P
from helpers import run_flow
from helpers.verdict import Verdict

URL = "https://registry.example.invalid/demo-{v}.json"


def run(ctx):
    v = Verdict("packs/pack_publish_install_update")
    ctx.boot_packs({"E2E_MODEL_KEY": run_flow.fake_keys(ctx)["model_key"]})
    t = ctx.team

    # 1. pack v1.0.0; the Python content hash is the binary's
    P.make_worker(t, "demo", "1.0.0", ["@model"])
    r = P.pack(t, "workers/demo", "packs/demo-1.0.0")
    v.check("pack writes the pack folder", "error" not in r and r.get("packed") == "packs/demo-1.0.0", str(r))
    manifest1 = P.text_of(t, "packs/demo-1.0.0/pack.yaml") or ""
    v.check("packs/demo-1.0.0/pack.yaml exists", "content_hash" in manifest1)
    files1 = {}
    for rel in r.get("files", []):
        if rel != "pack.yaml":
            files1[rel] = P.text_of(t, f"packs/demo-1.0.0/{rel}") or ""
    hash1 = P.content_hash(files1)
    v.check("the harness computes the binary's content hash", hash1 in manifest1, hash1)

    # 2. publish: untrusted signer is skipped until trusted
    P.write_seed(t)
    pub = P.publish(t, "packs/demo-1.0.0", URL.format(v="1.0.0"), ["read"])
    v.check("publish signs the pack: name demo, untrusted, the note names trusted_keys",
            pub.get("published", {}).get("name") == "demo" and pub.get("trusted") is False
            and "trusted_keys" in pub.get("note", ""), str(pub))
    signer = pub.get("published", {}).get("signer_key", "")
    v.check("the published hash is the content hash", pub.get("published", {}).get("hash") == hash1)
    lst = P.listing(t, "list")
    v.check("list skips an entry of an untrusted signer, and says so",
            lst.get("packs") == [] and lst.get("skipped"), str(lst))
    P.trust(t, signer)
    lst = P.listing(t, "list")
    packs = lst.get("packs", [])
    v.check("after trust, list shows demo 1.0.0, not installed, not revoked",
            len(packs) == 1 and packs[0]["name"] == "demo" and packs[0]["version"] == "1.0.0"
            and packs[0]["installed"] is False and packs[0]["revoked"] is False, str(lst))

    # 3. install as a template
    inst = P.result(t.send("pack", {"type": "install", "pack": "packs/demo-1.0.0", "into": "templates"}))
    v.check("install copies the pack as a template and creates no agent",
            "error" not in inst and inst.get("installed_as") == "template", str(inst))
    v.check("templates/demo-1.0.0/pack.yaml exists", P.exists(t, "templates/demo-1.0.0/pack.yaml"))
    show = P.listing(t, "show", name="demo")
    v.check("show says demo is installed", show.get("installed") is True, str(show))

    # 4. v1.1.0 adds a port and a grant
    P.make_worker(t, "demo", "1.1.0", ["@model", "@connector:github"], folder="workers/demo2")
    r2 = P.pack(t, "workers/demo2", "packs/demo-1.1.0")
    v.check("pack v1.1.0", "error" not in r2, str(r2))
    pub2 = P.publish(t, "packs/demo-1.1.0", URL.format(v="1.1.0"), ["read", "write"])
    hash2 = pub2.get("published", {}).get("hash", "")
    v.check("publish v1.1.0 with grants read and write", hash2 and hash2 != hash1, str(pub2))

    # 5. updates are opt-in and install nothing
    off = P.listing(t, "updates", opt_in=False)
    v.check("updates without opt_in offers none", off.get("updates") == [], str(off))
    on = P.listing(t, "updates", opt_in=True)
    ups = on.get("updates", [])
    diff = ups[0].get("permission_diff", {}) if ups else {}
    v.check("updates with opt_in offers exactly the newer version, with its pinned hash",
            len(ups) == 1 and ups[0].get("installed_version") == "1.0.0"
            and ups[0].get("new_version") == "1.1.0" and ups[0].get("new_hash") == hash2, str(on))
    v.check("the permission diff names the added port and grant and nothing removed",
            diff.get("added_ports") == ["@connector:github"] and diff.get("added_grants") == ["write"]
            and diff.get("removed_ports") == [] and diff.get("removed_grants") == [], str(diff))
    v.check("an update installs nothing: templates/demo-1.1.0 does not exist",
            not P.exists(t, "templates/demo-1.1.0/pack.yaml"))

    # 6. takedown of the installed 1.0.0
    td = P.takedown(t, "demo", "1.0.0")
    v.check("takedown revokes locally and pushes nowhere (no registry_url)",
            td.get("revoked") == {"name": "demo", "version": "1.0.0"} and td.get("pushed") is False, str(td))
    on = P.listing(t, "updates", opt_in=True)
    v.check("the installed revoked pack becomes a warning that names demo",
            any("demo" in w for w in on.get("warnings", [])), str(on))
    sh = P.listing(t, "show", name="demo", version="1.0.0")
    v.check("show marks 1.0.0 revoked and refuses its install; list offers the newer 1.1.0",
            sh.get("revoked") is True and sh.get("install_decision", {}).get("allow") is False
            and [(p["version"], p["revoked"]) for p in P.listing(t, "list").get("packs", [])]
            == [("1.1.0", False)], str(sh))
    fe = P.listing(t, "fetch", name="demo", hash=hash1, stage="stage/x")
    v.check("fetch of the revoked hash is refused with revoked, before any download",
            "revoked" in fe.get("error", ""), str(fe))
    v.check("nothing is written under stage/x", not P.exists(t, "stage/x/pack.yaml"))

    # 7. a tampered flag is not honoured: the unsigned edit is skipped, not believed
    idx = P.text_of(t, "packs/index.json") or ""
    doc = json.loads(idx)
    for e in doc["entries"]:
        if e["version"] == "1.0.0":
            e["revoked"] = False
    P.put(t, "packs/index.json", json.dumps(doc))
    lst = P.listing(t, "list")
    seen = {(p["name"], p["version"]) for p in lst.get("packs", [])}
    v.check("an entry whose revoked flag was edited without re-signing is skipped and reported",
            ("demo", "1.0.0") not in seen and lst.get("skipped"), str(lst))
    on = P.listing(t, "updates", opt_in=True)
    v.check("a skipped entry cannot be offered as an update either", all(
        u.get("new_version") != "1.0.0" for u in on.get("updates", [])), str(on))

    t.stop()
    return v.finish()
