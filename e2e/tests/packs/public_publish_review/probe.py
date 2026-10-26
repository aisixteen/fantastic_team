"""Review before listing: a pack with a scan finding is refused locally and held (not sent) for the
hosted registry; a clean one goes on. The signing key never shows."""

import json
import os
import time

from helpers import packs_pub as P
from helpers import run_flow
from helpers.registry import FakeRegistry
from helpers.verdict import Verdict

URL = "https://registry.example.invalid/{n}.json"


def settle(reg, want, wait_s=5.0):
    end = time.time() + wait_s
    while reg.handshake_failures < want and time.time() < end:
        time.sleep(0.05)


def run(ctx):
    v = Verdict("packs/public_publish_review")
    ctx.boot_packs({"E2E_MODEL_KEY": run_flow.fake_keys(ctx)["model_key"]})
    t = ctx.team
    reg = FakeRegistry(os.path.join(ctx.run_dir, "registry")).start()
    replies = []
    try:
        P.write_seed(t)
        # a clean pack, and a pack with a finding (a code file) written by hand: the `pack` verb
        # refuses such a pack, so it can only come from outside
        P.make_worker(t, "clean", "1.0.0", ["@model"])
        pc = P.pack(t, "workers/clean", "packs/clean-1.0.0")
        v.check("pack writes the clean pack", "error" not in pc, str(pc))
        P.hand_pack(t, "packs/risky-1.0.0", "risky", "1.0.0", {
            "worker.json": json.dumps({"name": "risky", "version": "1.0.0", "ports": ["@model"]}),
            "bin/run.sh": "echo hello"})

        # 1. local review
        pub = P.publish(t, "packs/clean-1.0.0", URL.format(n="clean"), ["read"])
        replies.append(pub)
        P.trust(t, pub["published"]["signer_key"])
        v.check("a clean pack is published and listed",
                [p["name"] for p in P.listing(t, "list").get("packs", [])] == ["clean"])
        index_before = P.text_of(t, "packs/index.json")
        bad = P.publish(t, "packs/risky-1.0.0", URL.format(n="risky"), ["read"])
        replies.append(bad)
        rules = [(f.get("path"), f.get("rule")) for f in bad.get("findings", [])]
        v.check("publish refuses a pack with a scan finding, naming the file and the rule",
                bad.get("error") == "pack refused by scan" and ("bin/run.sh", "code_file") in rules, str(bad))
        v.check("the index is unchanged by the refused publish", P.text_of(t, "packs/index.json") == index_before)
        token = "ghp_" + "A1b2C3d4" * 5  # a key-shaped line, built from pieces
        P.make_worker(t, "leaky", "1.0.0", ["@model"], directives={"main.md": "Use token " + token})
        lk = P.pack(t, "workers/leaky", "packs/leaky-1.0.0")
        v.check("the pack verb refuses a directive with a secret-looking line",
                lk.get("error") == "pack refused by scan"
                and any(f.get("rule") == "secret" for f in lk.get("findings", [])), str(lk))
        v.check("and the refusal never echoes the secret", token not in json.dumps(lk))
        v.check("nothing was written for the refused pack", not P.exists(t, "packs/leaky-1.0.0/pack.yaml"))

        # 2. hosted review: the finding is held, nothing is sent
        P.set_registry(t, reg.url)
        held = P.listing(t, "submit", pack="packs/risky-1.0.0", url=URL.format(n="risky"),
                         signing_key=P.SEED_PATH, grants=["read"])
        replies.append(held)
        v.check("submit holds the pack with a finding: listed false, status held, the finding",
                held.get("listed") is False and held.get("status") == "held"
                and {"path": "bin/run.sh", "rule": "code_file"} in held.get("findings", []), str(held))
        v.check("the registry saw nothing for the held pack", reg.quiet(),
                f"{reg.connections} {reg.requests}")
        sent = P.listing(t, "submit", pack="packs/clean-1.0.0", url=URL.format(n="clean2"),
                         signing_key=P.SEED_PATH, grants=["read"])
        replies.append(sent)
        settle(reg, 1)
        v.check("submit of the clean pack goes on to the registry and fails there: submit failed",
                sent.get("error", "").startswith("submit failed"), str(sent))
        v.check("the registry saw the connection attempt, and no request (it is not trusted)",
                reg.connections >= 1 and reg.handshake_failures >= 1 and reg.requests == [],
                f"{reg.connections} {reg.handshake_failures} {reg.requests}")
        v.check("a failed submit lists nothing", sent.get("listed") is not True)

        # 3. the key never appears
        everything = json.dumps(replies)
        v.check("the signing seed is in no reply", P.SEED not in everything)
        t.stop()
        v.check("and not in the team's log", P.SEED not in t.log())
    finally:
        reg.stop()
    return v.finish()
