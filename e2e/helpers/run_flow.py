"""The steps both tests share: make the fake keys, boot the team, pull, hand over the task."""

from __future__ import annotations

import os
import secrets
import time

from . import forge as forge_mod


def fake_keys(ctx) -> dict:
    """A fake model key (built from pieces), an app id and a throwaway app PEM."""
    sdir = os.path.join(ctx.run_dir, "secrets")
    os.makedirs(sdir, exist_ok=True)
    with open(os.path.join(ctx.run_dir, ".gitignore"), "w") as f:  # keep keys out of scans
        f.write("secrets/\n")
    pem = os.path.join(sdir, "app.pem")
    if not forge_mod.make_app_key(pem):
        raise RuntimeError("openssl cannot make the throwaway app key")
    with open(pem) as f:
        pem_text = f.read()
    body = [l for l in pem_text.splitlines() if l and not l.startswith("-----")]
    return {"model_key": "".join(["sk-", "ant-", secrets.token_hex(20)]),
            "app_id": "123", "pem": pem_text,
            "pem_chunk": body[1][:48] if len(body) > 1 else body[0][:48]}


def boot(ctx) -> dict:
    """Boot the team with the fake keys in ITS environment only. Returns the keys."""
    keys = fake_keys(ctx)
    ctx.secrets = keys
    ctx.boot_team({"E2E_MODEL_KEY": keys["model_key"], "E2E_APP_ID": keys["app_id"],
                   "E2E_APP_PEM": keys["pem"]})
    return keys


def hand_over(ctx, text: str, wait_s: float = 120.0) -> bool:
    """Send the lead its message and wait until the scripted model has played every turn."""
    r = ctx.team.send("lead", {"type": "send", "text": text})
    if r.get("status") != "completed":
        return False
    end = time.time() + wait_s
    while time.time() < end and ctx.model.served < len(ctx.model.turns):
        time.sleep(0.3)
    time.sleep(2.0)  # the last turn's reply is routed after the last request
    return ctx.model.served >= len(ctx.model.turns)
