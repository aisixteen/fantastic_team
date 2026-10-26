"""The steps both tests share: make the fake keys, boot the team, pull, hand over the task."""

from __future__ import annotations

import os
import secrets
import threading
import time

from . import forge as forge_mod


def fake_keys(ctx) -> dict:
    """The model key, an app id and a throwaway app PEM.

    A fake plug gets a fake model key (built from pieces). A real plug (`ctx.model.real`)
    gets its key from the environment, at run time; it goes into the team's environment
    only, never onto disk. The app id and the PEM stay throwaway."""
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
    if ctx.model.real:
        model_key = os.environ.get(ctx.model.key_env or "", "")
        if not model_key:
            raise RuntimeError("a real plug needs its key in the environment")
    else:
        model_key = "".join(["sk-", "ant-", secrets.token_hex(20)])
    return {"model_key": model_key,
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


def kick(ctx, text: str, timeout: float = 1800.0) -> threading.Thread:
    """Send the lead its message in the background: a real model takes minutes to answer.

    The team's reply (or error) lands in `ctx.kicked`. Pair with `wait_done`.
    """
    ctx.kicked = {}

    def go():
        try:
            ctx.kicked["reply"] = ctx.team.send("lead", {"type": "send", "text": text}, timeout=timeout)
        except Exception as e:  # recorded, the probe decides
            ctx.kicked["error"] = str(e)

    t = threading.Thread(target=go, daemon=True)
    t.start()
    return t


def wait_done(ctx, predicate, wait_s: float = 900.0, every: float = 5.0) -> bool:
    """Poll `predicate()` until it is true. Stops early when the plug errored or the team exited."""
    end = time.time() + wait_s
    while time.time() < end:
        if predicate():
            return True
        if ctx.model.errored:
            return False
        if ctx.team.proc and ctx.team.proc.poll() is not None:
            return False
        time.sleep(every)
    return bool(predicate())


def wait_model_body(ctx, needle: str, timeout: float = 30.0, every: float = 0.3):
    """Poll the scripted model's request bodies for `needle`; the body, or None."""
    end = time.time() + timeout
    while True:
        for b in list(ctx.model.raw_bodies):
            if needle in b:
                return b
        if time.time() >= end:
            return None
        time.sleep(every)
